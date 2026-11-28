#include "termigator-term-bridge/src/terminal_item.h"
#include "termigator-term-bridge/src/box_drawing.h"
#include "termigator-term-bridge/src/window_effects.h"

#include <QtQuick/QQuickWindow>

#include <QtCore/QDataStream>
#include <QtCore/QElapsedTimer>
#include <QtCore/QFile>
#include <QtCore/QTimer>
#include <QtGui/QClipboard>
#include <QtGui/QFontDatabase>
#include <QtGui/QFontMetricsF>
#include <QtGui/QFocusEvent>
#include <QtGui/QGuiApplication>
#include <QtGui/QInputMethodEvent>
#include <QtGui/QKeyEvent>
#include <QtGui/QMouseEvent>
#include <QtGui/QPainter>
#include <QtGui/QWheelEvent>

#include <cstdio>

#include "termigator-term-bridge/src/ghostty_session.h"
#include "termigator-term-bridge/src/multiexec.h"

namespace termigator::term {

namespace {

// The last mouse selection in any terminal of the app: the primary selection where the platform
// has none (Windows, macOS).
QString &lastSelection()
{
    static QString text;
    return text;
}

constexpr quint32 SNAPSHOT_MAGIC = 0x47535031; // "GSP1"

// Auto-scroll while a selection is dragged above or below the rows: a tick per frame, and a
// speed in rows per second that grows with the pointer's distance from the edge (in cells).
constexpr int AUTO_SCROLL_INTERVAL_MS = 16;
constexpr qreal AUTO_SCROLL_MIN_ROWS_PER_SEC = 8;
constexpr qreal AUTO_SCROLL_ROWS_PER_SEC_PER_CELL = 12;
constexpr qreal AUTO_SCROLL_MAX_ROWS_PER_SEC = 400;

QColor readColor(QDataStream &stream, bool &ok)
{
    quint8 r = 0;
    quint8 g = 0;
    quint8 b = 0;
    stream >> r >> g >> b;
    ok = stream.status() == QDataStream::Ok;
    return QColor(r, g, b);
}

} // namespace

// Powerline/Nerd Font icons and emoji are not guaranteed to advance exactly one cell in the
// configured font (the non-Mono Nerd Font variants are proportional for private-use glyphs), nor
// is any glyph the primary face lacks (a fallback font steps in with its own metrics). Drawing
// them inside their own cell keeps the following runs aligned; Konsole does the same by
// positioning every glyph.
bool GhosttyTerminalItem::needsOwnCell(const QString &text, int width) const
{
    if (width != 1)
        return true;
    for (const QChar ch : text) {
        const ushort c = ch.unicode();
        if ((c >= 0xe000 && c <= 0xf8ff) || c >= 0xd800)
            return true;
        if (c >= 0x80 && (!m_primaryFace.isValid() || !m_primaryFace.supportsCharacter(ch)))
            return true;
    }
    return false;
}

GhosttyTerminalItem::GhosttyTerminalItem(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    m_font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    m_font.setPointSizeF(10.5);
    m_primaryFace = QRawFont::fromFont(m_font);
    const QFontMetricsF metrics(m_font);
    m_cellWidth = metrics.horizontalAdvance(QLatin1Char('M'));
    m_cellHeight = metrics.height();
    setAntialiasing(false);
    setFocus(true);
    setFlag(QQuickItem::ItemAcceptsInputMethod, true);
    m_blinkTimer = new QTimer(this);
    m_blinkTimer->setInterval(530);
    connect(m_blinkTimer, &QTimer::timeout, this, [this] {
        m_cursorOn = !m_cursorOn;
        m_fullRedraw = true;
        update();
    });
    m_refreshTimer = new QTimer(this);
    m_refreshTimer->setSingleShot(true);
    m_refreshTimer->setInterval(0);
    connect(m_refreshTimer, &QTimer::timeout, this, &GhosttyTerminalItem::refresh);
    m_autoScrollTimer = new QTimer(this);
    m_autoScrollTimer->setTimerType(Qt::PreciseTimer);
    m_autoScrollTimer->setInterval(AUTO_SCROLL_INTERVAL_MS);
    connect(m_autoScrollTimer, &QTimer::timeout, this, &GhosttyTerminalItem::autoScrollStep);
}

GhosttyTerminalItem::~GhosttyTerminalItem() = default;

void GhosttyTerminalItem::setSession(GhosttySession *session)
{
    if (session == m_session)
        return;
    // The anchor belongs to the old session's screen.
    m_selecting = false;
    m_autoScrollTimer->stop();
    if (m_session != nullptr)
        disconnect(m_session, nullptr, this, nullptr);
    m_session = session;
    if (m_session != nullptr) {
        connect(m_session, &GhosttySession::updated, this, [this] {
            updateScrollbar();
            requestFrame();
        });
        connect(m_session, &GhosttySession::finishedChanged, this, [this] { requestFrame(); });
    }
    emit sessionChanged();
    emit colorSchemeChanged();
    emit fillColorChanged();
    updateGrid();
    updateScrollbar();
    requestFrame();
}

void GhosttyTerminalItem::setSource(const QString &path)
{
    if (path == m_source)
        return;
    m_source = path;
    emit sourceChanged();
    if (!path.isEmpty())
        feedFile(path);
}

void GhosttyTerminalItem::feed(const QByteArray &data)
{
    if (m_session != nullptr)
        m_session->feedBytes(data);
}

void GhosttyTerminalItem::feedFile(const QString &path)
{
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        std::fprintf(stderr, "ghostty spike: cannot open %s\n", qPrintable(path));
        return;
    }
    QElapsedTimer timer;
    timer.start();
    qint64 total = 0;
    while (!file.atEnd()) {
        const QByteArray chunk = file.read(64 * 1024);
        if (chunk.isEmpty())
            break;
        feed(chunk);
        total += chunk.size();
    }
    std::fprintf(stderr, "ghostty spike: fed %lld bytes in %lld ms\n",
                 static_cast<long long>(total), static_cast<long long>(timer.elapsed()));
}

QString GhosttyTerminalItem::text() const
{
    return m_session != nullptr ? m_session->historyText() : QString();
}

void GhosttyTerminalItem::scrollBy(int rows)
{
    if (m_session != nullptr)
        m_session->scrollBy(rows);
}

void GhosttyTerminalItem::sendText(const QString &text)
{
    if (m_session != nullptr)
        m_session->sendText(text);
}

void GhosttyTerminalItem::sendKey(int key, int modifiers, const QString &text)
{
    if (m_session != nullptr)
        m_session->sendKey(key, modifiers, text);
}

void GhosttyTerminalItem::setCopyOnSelect(bool copy)
{
    if (copy == m_copyOnSelect)
        return;
    m_copyOnSelect = copy;
    emit copyOnSelectChanged();
}

void GhosttyTerminalItem::setFont(const QFont &font)
{
    if (font == m_font)
        return;
    m_font = font;
    // Fall back to a Nerd Font and CJK/emoji families for glyphs the primary lacks. Without this,
    // QFont picks an arbitrary fallback for Powerline/private-use glyphs and the prompt's
    // cells stop lining up (mixed advances). The non-"Mono" variants come first: their icons are
    // full size and overflow into the next cell, which is what Powerlevel10k expects.
    m_font.setFamilies({font.family(), QStringLiteral("MesloLGS Nerd Font"),
                        QStringLiteral("JetBrainsMono Nerd Font"),
                        QStringLiteral("JetBrainsMono Nerd Font Mono"),
                        QStringLiteral("Noto Sans Mono CJK JP"),
                        QStringLiteral("Noto Color Emoji")});
    m_primaryFace = QRawFont::fromFont(m_font);
    const QFontMetricsF metrics(m_font);
    m_cellWidth = metrics.horizontalAdvance(QLatin1Char('M'));
    m_cellHeight = metrics.height();
    emit fontChanged();
    updateGrid();
    update();
}

QString GhosttyTerminalItem::colorScheme() const
{
    return m_session != nullptr ? m_session->colorScheme() : QString();
}

void GhosttyTerminalItem::setColorScheme(const QString &name)
{
    if (m_session == nullptr)
        return;
    m_session->setColorScheme(name);
    emit colorSchemeChanged();
    requestFrame();
}

void GhosttyTerminalItem::setBlinkingCursor(bool blink)
{
    if (blink == m_blinkingCursor)
        return;
    m_blinkingCursor = blink;
    updateBlink();
    emit blinkingCursorChanged();
    update();
}

void GhosttyTerminalItem::setBracketedPasteDisabled(bool disabled)
{
    if (disabled == m_bracketedPasteDisabled)
        return;
    m_bracketedPasteDisabled = disabled;
    emit bracketedPasteDisabledChanged();
}

void GhosttyTerminalItem::setConfirmMultilinePaste(bool confirm)
{
    if (confirm == m_confirmMultilinePaste)
        return;
    m_confirmMultilinePaste = confirm;
    emit confirmMultilinePasteChanged();
}

void GhosttyTerminalItem::setPasteOnRightClick(bool paste)
{
    if (paste == m_pasteOnRightClick)
        return;
    m_pasteOnRightClick = paste;
    emit pasteOnRightClickChanged();
}

void GhosttyTerminalItem::setPasteOnMiddleClick(bool paste)
{
    if (paste == m_pasteOnMiddleClick)
        return;
    m_pasteOnMiddleClick = paste;
    emit pasteOnMiddleClickChanged();
}

void GhosttyTerminalItem::setBackgroundOpacity(qreal opacity)
{
    opacity = qBound(0.0, opacity, 1.0);
    if (qFuzzyCompare(opacity, m_backgroundOpacity))
        return;
    m_backgroundOpacity = opacity;
    emit backgroundOpacityChanged();
    update();
}

void GhosttyTerminalItem::setBlurBehind(bool blur)
{
    if (blur == m_blurBehind)
        return;
    m_blurBehind = blur;
    setBlurBehindWindow(window(), blur);
    emit blurBehindChanged();
}

void GhosttyTerminalItem::itemChange(ItemChange change, const ItemChangeData &data)
{
    // A view made for a split, a drag or a detached window lands in its window here.
    if (change == ItemSceneChange && m_blurBehind)
        setBlurBehindWindow(data.window, true);
    QQuickPaintedItem::itemChange(change, data);
}

QStringList GhosttyTerminalItem::availableColorSchemes() const
{
    return GhosttySession::availableSchemes();
}

bool GhosttyTerminalItem::findNext(const QString &needle, bool caseSensitive, bool regex, bool forward,
                                bool current)
{
    const bool found = m_session != nullptr
        && m_session->findNext(needle, caseSensitive, regex, forward, current);
    requestFrame();
    emit searchResult(found);
    return found;
}

QString GhosttyTerminalItem::hotspotUrlAt(int x, int y)
{
    if (m_session == nullptr)
        return QString();
    const QPoint cell = cellAt(QPointF(x, y));
    return m_session->urlAt(cell.x(), cell.y());
}

void GhosttyTerminalItem::updateScrollbar()
{
    if (m_session == nullptr)
        return;
    const int total = m_session->scrollbarTotal();
    const int offset = m_session->scrollbarOffset();
    const int len = m_session->scrollbarLen();
    const int maximum = qMax(0, total - len);
    if (offset == m_scrollValue && maximum == m_scrollMaximum)
        return;
    m_scrollValue = offset;
    m_scrollMaximum = maximum;
    emit scrollbarChanged();
}

QPoint GhosttyTerminalItem::cellAt(const QPointF &pos) const
{
    const int x = qBound(0, static_cast<int>(pos.x() / m_cellWidth), qMax(0, m_cols - 1));
    const int y = qBound(0, static_cast<int>(pos.y() / m_cellHeight), qMax(0, m_rows - 1));
    return QPoint(x, y);
}

void GhosttyTerminalItem::select(int startCol, int startRow, int endCol, int endRow, bool rectangle)
{
    if (m_session == nullptr)
        return;
    m_session->select(startCol, startRow, endCol, endRow, rectangle);
    update();
}

void GhosttyTerminalItem::selectWord(int col, int row)
{
    if (m_session == nullptr)
        return;
    m_session->selectWord(col, row);
    update();
}

void GhosttyTerminalItem::selectLine(int col, int row)
{
    if (m_session == nullptr)
        return;
    m_session->selectLine(col, row);
    update();
}

QString GhosttyTerminalItem::selectedText()
{
    return selectedText(false);
}

QString GhosttyTerminalItem::selectedText(bool trim)
{
    if (m_session == nullptr)
        return QString();
    QString text = m_session->selectedText();
    if (trim)
        text = text.trimmed();
    return text;
}

void GhosttyTerminalItem::copyClipboard()
{
    const QString text = selectedText(false);
    if (!text.isEmpty())
        QGuiApplication::clipboard()->setText(text);
}

void GhosttyTerminalItem::pasteClipboard()
{
    if (m_session == nullptr)
        return;
    const QString text = QGuiApplication::clipboard()->text();
    if (m_confirmMultilinePaste && text.contains(QLatin1Char('\n'))) {
        emit multilinePasteRequested(text, false);
        return;
    }
    paste(text);
}

void GhosttyTerminalItem::paste(const QString &text)
{
    const bool bracketed = !m_bracketedPasteDisabled;
    MultiExecHub::instance()->pastedGhostty(m_session->sessionId(), text, bracketed);
    m_session->pasteTextMode(text, bracketed && m_session->bracketedPaste());
}

void GhosttyTerminalItem::pasteText(const QString &text, bool appendReturn)
{
    if (m_session == nullptr)
        return;
    QString data = text;
    if (appendReturn)
        data += QLatin1Char('\n');
    paste(data);
}

void GhosttyTerminalItem::pasteSelection()
{
    if (m_session == nullptr)
        return;
    QClipboard *clipboard = QGuiApplication::clipboard();
    QString text;
    if (clipboard->supportsSelection()) {
        text = clipboard->text(QClipboard::Selection);
    } else {
        // Windows has no primary selection: the last mouse selection in any of our terminals,
        // else the system clipboard.
        text = lastSelection();
        if (text.isEmpty())
            text = clipboard->text();
    }
    if (text.isEmpty())
        return;
    if (m_confirmMultilinePaste && text.contains(QLatin1Char('\n'))) {
        emit multilinePasteRequested(text, false);
        return;
    }
    paste(text);
}

void GhosttyTerminalItem::setScrollbarValue(int value)
{
    if (m_session == nullptr)
        return;
    m_session->scrollToRow(qBound(0, value, m_scrollMaximum));
    updateScrollbar();
}

void GhosttyTerminalItem::finishSelection()
{
    // Like X11 terminals: a mouse selection is the primary selection, without touching the
    // clipboard (that is what copyOnSelect is for).
    const QString text = selectedText(false);
    if (!text.isEmpty()) {
        lastSelection() = text;
        QClipboard *clipboard = QGuiApplication::clipboard();
        if (clipboard->supportsSelection())
            clipboard->setText(text, QClipboard::Selection);
    }
    if (m_copyOnSelect)
        copyClipboard();
    emit selectionChanged();
}

void GhosttyTerminalItem::keyPressEvent(QKeyEvent *event)
{
    if (m_session == nullptr || !m_session->attached()) {
        QQuickPaintedItem::keyPressEvent(event);
        return;
    }
    MultiExecHub::instance()->keyPressedGhostty(m_session->sessionId(), event->key(),
                                                event->modifiers(), event->text());
    m_session->sendKey(event->key(), event->modifiers(), event->text());
    event->accept();
}

void GhosttyTerminalItem::inputMethodEvent(QInputMethodEvent *event)
{
    if (m_session == nullptr) {
        QQuickPaintedItem::inputMethodEvent(event);
        return;
    }
    if (!event->commitString().isEmpty()) {
        MultiExecHub::instance()->textCommittedGhostty(m_session->sessionId(), event->commitString());
        m_session->sendText(event->commitString());
    }
    m_preedit = event->preeditString();
    update();
    event->accept();
}

QVariant GhosttyTerminalItem::inputMethodQuery(Qt::InputMethodQuery query) const
{
    if (query == Qt::ImCursorRectangle) {
        return QRectF(m_cursorX * m_cellWidth, m_cursorY * m_cellHeight, m_cellWidth,
                      m_cellHeight);
    }
    if (query == Qt::ImEnabled)
        return true;
    return QQuickPaintedItem::inputMethodQuery(query);
}

void GhosttyTerminalItem::focusInEvent(QFocusEvent *event)
{
    m_hasFocus = true;
    update();
    QQuickPaintedItem::focusInEvent(event);
}

void GhosttyTerminalItem::focusOutEvent(QFocusEvent *event)
{
    m_hasFocus = false;
    update();
    QQuickPaintedItem::focusOutEvent(event);
}

void GhosttyTerminalItem::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickPaintedItem::geometryChange(newGeometry, oldGeometry);
    if (newGeometry.size() != oldGeometry.size())
        updateGrid();
}

void GhosttyTerminalItem::updateBlink()
{
    m_cursorOn = true;
    if ((m_cursorBlinking || m_blinkingCursor) && m_cursorVisible)
        m_blinkTimer->start();
    else
        m_blinkTimer->stop();
}

void GhosttyTerminalItem::updateGrid()
{
    if (m_session == nullptr)
        return;
    const int cols = qMax(1, static_cast<int>(width() / m_cellWidth));
    const int rows = qMax(1, static_cast<int>(height() / m_cellHeight));
    // Whole pixels, as the mouse reports get them.
    m_session->resizeTerminal(cols, rows, static_cast<int>(m_cellWidth),
                              static_cast<int>(m_cellHeight));
    requestFrame();
}

void GhosttyTerminalItem::mousePressEvent(QMouseEvent *event)
{
    if (m_session == nullptr) {
        QQuickPaintedItem::mousePressEvent(event);
        return;
    }
    const QPoint cell = cellAt(event->position());
    const int button = event->button() == Qt::LeftButton
        ? 1
        : (event->button() == Qt::RightButton ? 2 : (event->button() == Qt::MiddleButton ? 4 : 0));
    // Ctrl+click on a hotspot opens the link, even when the program reports the mouse.
    if (button == 1 && (event->modifiers() & Qt::ControlModifier)) {
        const QString url = m_session->urlAt(cell.x(), cell.y());
        if (!url.isEmpty()) {
            emit linkActivated(url);
            event->accept();
            return;
        }
    }
    // Shift overrides a program's mouse reporting (mc, tmux, vim), as in xterm and Konsole.
    const bool local = event->modifiers() & Qt::ShiftModifier;
    const QByteArray encoded = local
        ? QByteArray()
        : m_session->encodeMouse(cell.x(), cell.y(), button, 0,
                                 static_cast<int>(event->modifiers()),
                                 static_cast<int>(m_cellWidth), static_cast<int>(m_cellHeight));
    if (!encoded.isEmpty()) {
        m_session->sendBytes(encoded);
        event->accept();
        return;
    }
    if (button == 4) {
        if (m_pasteOnMiddleClick)
            pasteSelection();
        event->accept();
        return;
    }
    if (button == 2) {
        if (m_pasteOnRightClick && selectedText(true).isEmpty())
            pasteClipboard();
        else
            emit configureRequest(event->position().toPoint());
        event->accept();
        return;
    }
    if (button == 1) {
        m_selecting = true;
        m_anchor = QPoint(cell.x(), cell.y() + m_session->scrollbarOffset());
        m_dragPos = event->position();
        m_dragRectangle = event->modifiers() & Qt::AltModifier;
        updateDragSelection();
    }
    event->accept();
}

void GhosttyTerminalItem::mouseMoveEvent(QMouseEvent *event)
{
    if (m_session == nullptr) {
        QQuickPaintedItem::mouseMoveEvent(event);
        return;
    }
    const QPoint cell = cellAt(event->position());
    // The held buttons: a program tracking drags only (DECSET 1002) ignores a motion without one.
    const QByteArray encoded = m_session->encodeMouse(
        cell.x(), cell.y(), static_cast<int>(event->buttons().toInt()), 2,
        static_cast<int>(event->modifiers()), static_cast<int>(m_cellWidth),
        static_cast<int>(m_cellHeight));
    if (!encoded.isEmpty() && !m_selecting) {
        m_session->sendBytes(encoded);
        event->accept();
        return;
    }
    if (m_selecting) {
        m_dragPos = event->position();
        m_dragRectangle = event->modifiers() & Qt::AltModifier;
        updateDragSelection();
        updateAutoScroll();
    }
    event->accept();
}

void GhosttyTerminalItem::mouseReleaseEvent(QMouseEvent *event)
{
    if (m_session == nullptr) {
        QQuickPaintedItem::mouseReleaseEvent(event);
        return;
    }
    const QPoint cell = cellAt(event->position());
    const int button = event->button() == Qt::LeftButton
        ? 1
        : (event->button() == Qt::RightButton ? 2 : (event->button() == Qt::MiddleButton ? 4 : 0));
    const QByteArray encoded =
        m_session->encodeMouse(cell.x(), cell.y(), button, 1, static_cast<int>(event->modifiers()),
                               static_cast<int>(m_cellWidth), static_cast<int>(m_cellHeight));
    // A Shift+drag selection ends here even when the program reports the mouse.
    if (!encoded.isEmpty() && !m_selecting && !(event->modifiers() & Qt::ShiftModifier)) {
        m_session->sendBytes(encoded);
        event->accept();
        return;
    }
    if (m_selecting)
        stopSelecting();
    event->accept();
}

void GhosttyTerminalItem::mouseUngrabEvent()
{
    // Another item took the pointer (a popup, a drag): the selection ends where it got to.
    if (m_selecting && m_session != nullptr)
        stopSelecting();
    QQuickPaintedItem::mouseUngrabEvent();
}

void GhosttyTerminalItem::stopSelecting()
{
    m_selecting = false;
    m_autoScrollTimer->stop();
    finishSelection();
}

void GhosttyTerminalItem::updateDragSelection()
{
    if (m_session == nullptr)
        return;
    const QPoint cell = cellAt(m_dragPos);
    m_session->selectScreen(m_anchor.x(), m_anchor.y(), cell.x(),
                            cell.y() + m_session->scrollbarOffset(), m_dragRectangle);
    update();
}

void GhosttyTerminalItem::updateAutoScroll()
{
    const bool outside =
        m_rows > 0 && (m_dragPos.y() < 0 || m_dragPos.y() >= m_rows * m_cellHeight);
    if (!m_selecting || !outside) {
        m_autoScrollTimer->stop();
        return;
    }
    if (m_autoScrollTimer->isActive())
        return;
    m_autoScrollRows = 0;
    m_autoScrollClock.start();
    m_autoScrollTimer->start();
}

void GhosttyTerminalItem::autoScrollStep()
{
    if (!m_selecting || m_session == nullptr) {
        m_autoScrollTimer->stop();
        return;
    }
    const qreal bottom = m_rows * m_cellHeight;
    const qreal distance = m_dragPos.y() < 0 ? -m_dragPos.y() : m_dragPos.y() - bottom;
    if (distance < 0) {
        m_autoScrollTimer->stop();
        return;
    }
    const qreal cells = distance / m_cellHeight;
    const qreal speed =
        qMin(AUTO_SCROLL_MAX_ROWS_PER_SEC,
             AUTO_SCROLL_MIN_ROWS_PER_SEC + AUTO_SCROLL_ROWS_PER_SEC_PER_CELL * cells);
    // Elapsed time rather than ticks: a late timer scrolls further instead of slowing down.
    m_autoScrollRows += speed * m_autoScrollClock.restart() / 1000.0;
    const int rows = static_cast<int>(m_autoScrollRows);
    if (rows == 0)
        return;
    m_autoScrollRows -= rows;
    const int before = m_session->scrollbarOffset();
    m_session->scrollBy(m_dragPos.y() < 0 ? -rows : rows);
    if (m_session->scrollbarOffset() == before)
        return; // already at the top or the bottom of the scrollback
    updateScrollbar();
    updateDragSelection();
}

void GhosttyTerminalItem::mouseDoubleClickEvent(QMouseEvent *event)
{
    if (m_session == nullptr) {
        QQuickPaintedItem::mouseDoubleClickEvent(event);
        return;
    }
    const QPoint cell = cellAt(event->position());
    m_session->selectWord(cell.x(), cell.y());
    finishSelection();
    event->accept();
}

void GhosttyTerminalItem::wheelEvent(QWheelEvent *event)
{
    if (m_session == nullptr) {
        QQuickPaintedItem::wheelEvent(event);
        return;
    }
    // Qt's positive delta is a turn away from the user, up. A horizontal turn has no meaning here
    // (it used to scroll down).
    const int delta = event->angleDelta().y();
    event->accept();
    if (delta == 0)
        return;
    if ((delta > 0) != (m_wheelDelta > 0))
        m_wheelDelta = 0; // a reversal does not first use up the other direction's remainder
    m_wheelDelta += delta;
    const int modifiers = static_cast<int>(event->modifiers());
    const int target = m_session->wheelTarget(modifiers);
    // A notch (120) is one report to a program that tracks the mouse, three lines otherwise.
    const int unit = target == GhosttySession::WheelReport ? 120 : 40;
    const int steps = m_wheelDelta / unit;
    if (steps == 0)
        return;
    m_wheelDelta -= steps * unit;
    if (target == GhosttySession::WheelScrollback) {
        m_session->scrollBy(-steps);
        updateScrollbar();
        if (m_selecting)
            updateDragSelection();
        return;
    }
    const QPoint cell = cellAt(event->position());
    m_session->sendBytes(m_session->encodeWheel(cell.x(), cell.y(), -steps, modifiers,
                                                static_cast<int>(m_cellWidth),
                                                static_cast<int>(m_cellHeight)));
}

void GhosttyTerminalItem::requestFrame()
{
    m_snapshotStale = true;
    if (!m_refreshTimer->isActive())
        m_refreshTimer->start();
}

void GhosttyTerminalItem::refresh()
{
    if (!m_snapshotStale)
        return;
    parseSnapshot();
    m_snapshotStale = false;
    if (m_fullRedraw || !m_dirtyRows.isEmpty())
        update();
}

void GhosttyTerminalItem::ensureSnapshot()
{
    if (!m_snapshotStale && m_valid)
        return;
    parseSnapshot();
    m_snapshotStale = false;
}

void GhosttyTerminalItem::parseSnapshot()
{
    if (m_session == nullptr)
        return;
    const QByteArray data = m_session->snapshotBinary();
    QDataStream stream(data);
    stream.setByteOrder(QDataStream::BigEndian);

    quint32 magic = 0;
    quint16 cols = 0;
    quint16 rows = 0;
    quint16 cursorX = 0;
    quint16 cursorY = 0;
    quint8 visible = 0;
    quint8 shape = 0;
    quint8 blinking = 0;
    quint8 damage = 0;
    quint32 arenaLength = 0;
    quint32 rowCount = 0;
    stream >> magic >> cols >> rows >> cursorX >> cursorY >> visible >> shape >> blinking
        >> damage;
    bool colorOk = false;
    const QColor fg = readColor(stream, colorOk);
    const QColor bg = readColor(stream, colorOk);
    quint8 hasCursor = 0;
    stream >> hasCursor;
    const bool hasCursorColor = hasCursor != 0;
    const QColor cursorColor = readColor(stream, colorOk);
    stream >> arenaLength >> rowCount;
    if (magic != SNAPSHOT_MAGIC || stream.status() != QDataStream::Ok)
        return;

    Q_UNUSED(damage);
    QVector<RowData> parsed;
    parsed.reserve(static_cast<int>(rowCount));
    for (quint32 y = 0; y < rowCount; ++y) {
        quint8 wrapped = 0;
        quint32 cellCount = 0;
        stream >> wrapped >> cellCount;
        RowData row;
        row.wrapped = wrapped != 0;
        row.cells.reserve(static_cast<int>(cellCount));
        for (quint32 i = 0; i < cellCount; ++i) {
            CellData cell;
            quint16 x = 0;
            quint8 width = 1;
            quint8 hasFg = 0;
            quint8 hasBg = 0;
            stream >> x >> width >> cell.flags;
            stream >> hasFg;
            if (hasFg != 0)
                cell.fg = readColor(stream, colorOk);
            stream >> hasBg;
            if (hasBg != 0)
                cell.bg = readColor(stream, colorOk);
            stream >> cell.textOffset >> cell.textLength;
            cell.x = x;
            cell.width = width;
            row.cells.push_back(cell);
        }
        parsed.push_back(row);
        if (stream.status() != QDataStream::Ok)
            return;
    }

    QByteArray arena(static_cast<int>(arenaLength), Qt::Uninitialized);
    if (arenaLength > 0
        && stream.readRawData(arena.data(), static_cast<int>(arenaLength))
               != static_cast<int>(arenaLength)) {
        return;
    }
    for (RowData &row : parsed) {
        for (CellData &cell : row.cells) {
            cell.text = QString::fromUtf8(arena.constData() + cell.textOffset,
                                          static_cast<int>(cell.textLength));
        }
    }

    const QColor oldBg = m_bg;
    const int oldLines = m_rows;
    const bool metadataChanged =
        !m_valid || m_cols != cols || m_rows != rows || m_fg != fg || m_bg != bg
        || m_hasCursorColor != hasCursorColor || m_cursorColor != cursorColor;
    const bool oldCursorVisible = m_cursorVisible;
    const int oldCursorY = m_cursorY;
    m_dirtyRows.clear();
    if (metadataChanged || parsed.size() != m_rowsData.size()) {
        m_fullRedraw = true;
    } else {
        for (int y = 0; y < parsed.size(); ++y) {
            const RowData &a = parsed[y];
            const RowData &b = m_rowsData[y];
            bool same = a.wrapped == b.wrapped && a.cells.size() == b.cells.size();
            if (same) {
                for (int i = 0; i < a.cells.size(); ++i) {
                    const CellData &ca = a.cells[i];
                    const CellData &cb = b.cells[i];
                    if (ca.x != cb.x || ca.width != cb.width || ca.flags != cb.flags
                        || ca.fg != cb.fg || ca.bg != cb.bg || ca.text != cb.text) {
                        same = false;
                        break;
                    }
                }
            }
            if (!same)
                m_dirtyRows.push_back(y);
        }
        // The old cursor cell must be repainted even when the text did not change.
        if (oldCursorVisible)
            m_dirtyRows.push_back(oldCursorY);
        if (visible != 0)
            m_dirtyRows.push_back(cursorY);
    }

    m_cols = cols;
    m_rows = rows;
    m_cursorX = cursorX;
    m_cursorY = cursorY;
    m_cursorVisible = visible != 0;
    m_cursorShape = shape;
    m_cursorBlinking = blinking != 0;
    m_fg = fg;
    m_bg = bg;
    m_hasCursorColor = hasCursorColor;
    m_cursorColor = cursorColor;
    m_rowsData = parsed;
    updateBlink();
    m_valid = true;
    if (m_bg != oldBg) {
        emit fillColorChanged();
        emit colorSchemeChanged();
    }
    if (m_rows != oldLines)
        emit linesChanged();
}

void GhosttyTerminalItem::paint(QPainter *painter)
{
    QElapsedTimer timer;
    timer.start();
    ensureSnapshot();
    // QQuickPaintedItem clears its texture before paint, so every frame draws the whole
    // viewport. Dirty rows only decide whether a repaint is needed at all.
    QColor defaultBg = m_bg;
    defaultBg.setAlphaF(static_cast<float>(m_backgroundOpacity));
    // Source, not over: the see-through background replaces what the texture held.
    painter->setCompositionMode(QPainter::CompositionMode_Source);
    painter->fillRect(boundingRect(), defaultBg);
    painter->setCompositionMode(QPainter::CompositionMode_SourceOver);

    const qreal cellW = m_cellWidth;
    const qreal cellH = m_cellHeight;

    // Pass 1: every cell's background first. Konsole paints the whole background before any
    // glyph, so a glyph wider or taller than its cell spills over neighbouring backgrounds
    // instead of being covered by them and cut off.
    for (int y = 0; y < m_rowsData.size(); ++y) {
        const RowData &row = m_rowsData[y];
        for (const CellData &cell : row.cells) {
            const int flags = cell.flags;
            QColor bg = cell.bg.isValid() ? cell.bg : m_bg;
            if (flags & (1 << 4)) // inverse swaps fg and bg
                bg = cell.fg.isValid() ? cell.fg : m_fg;
            const QRectF rect(cell.x * cellW, y * cellH, cell.width * cellW, cellH);
            if (bg != m_bg)
                painter->fillRect(rect, bg);
            // Over the cell's own background: on mc's blue panels it was painted out.
            if (flags & (1 << 9)) // selected
                painter->fillRect(rect, QColor(70, 110, 160, 150));
        }
    }

    // Pass 2: glyphs.
    for (int y = 0; y < m_rowsData.size(); ++y) {
        const RowData &row = m_rowsData[y];
        int runX = 0;
        int runWidth = 0;
        QString runText;
        QColor runFg;
        int runFlags = 0;
        bool runActive = false;
        auto flush = [&] {
            if (!runActive)
                return;
            QFont font = m_font;
            font.setBold(runFlags & (1 << 0));
            font.setItalic(runFlags & (1 << 1));
            painter->setFont(font);
            painter->setPen(runFg);
            const QRectF rect(runX * cellW, y * cellH, runWidth * cellW, cellH);
            painter->drawText(rect, Qt::AlignLeft | Qt::AlignVCenter | Qt::TextDontClip, runText);
            if (runFlags & (1 << 8)) // underline
                painter->drawLine(QPointF(rect.left(), rect.bottom() - 1),
                                  QPointF(rect.right(), rect.bottom() - 1));
            if (runFlags & (1 << 6)) // strikethrough
                painter->drawLine(QPointF(rect.left(), rect.center().y()),
                                  QPointF(rect.right(), rect.center().y()));
            runActive = false;
            runText.clear();
            runWidth = 0;
        };

        for (const CellData &cell : row.cells) {
            const int flags = cell.flags;
            QColor fg = cell.fg.isValid() ? cell.fg : m_fg;
            QColor bg = cell.bg.isValid() ? cell.bg : m_bg;
            if (flags & (1 << 4)) // inverse
                std::swap(fg, bg);
            const QRectF rect(cell.x * cellW, y * cellH, cell.width * cellW, cellH);
            if (flags & (1 << 5)) { // invisible
                flush();
                continue;
            }
            // Box drawing and block elements as shapes on the grid, so frames meet exactly.
            if (cell.text.size() == 1 && cell.width == 1 && isBoxChar(cell.text.at(0).unicode())) {
                flush();
                drawBoxChar(painter, rect, cell.text.at(0).unicode(), fg);
                continue;
            }
            if (!cell.text.isEmpty() && needsOwnCell(cell.text, cell.width)) {
                flush();
                QFont font = m_font;
                font.setBold(flags & (1 << 0));
                font.setItalic(flags & (1 << 1));
                painter->save();
                painter->setFont(font);
                painter->setPen(fg);
                painter->drawText(rect, Qt::AlignLeft | Qt::AlignVCenter | Qt::TextDontClip,
                                  cell.text);
                if (flags & (1 << 8))
                    painter->drawLine(QPointF(rect.left(), rect.bottom() - 1),
                                      QPointF(rect.right(), rect.bottom() - 1));
                if (flags & (1 << 6))
                    painter->drawLine(QPointF(rect.left(), rect.center().y()),
                                      QPointF(rect.right(), rect.center().y()));
                painter->restore();
                continue;
            }
            // A blank cell (no text, as mc's coloured ED/ECH panels, or a space) ends the run: the
            // next glyph starts on its own cell, whatever a space advances in the font (a missing
            // family falls back to a proportional one). Only its underline / strikethrough is drawn.
            if (cell.text.isEmpty() || cell.text == QLatin1String(" ")) {
                flush();
                painter->setPen(fg);
                if (flags & (1 << 8)) // underline
                    painter->drawLine(QPointF(rect.left(), rect.bottom() - 1),
                                      QPointF(rect.right(), rect.bottom() - 1));
                if (flags & (1 << 6)) // strikethrough
                    painter->drawLine(QPointF(rect.left(), rect.center().y()),
                                      QPointF(rect.right(), rect.center().y()));
                continue;
            }
            // Monospace runs: one drawText for consecutive cells with the same pen and style.
            if (runActive && cell.x == runX + runWidth && fg == runFg && flags == runFlags) {
                runText += cell.text;
                runWidth += cell.width;
            } else {
                flush();
                runX = cell.x;
                runWidth = cell.width;
                runText = cell.text;
                runFg = fg;
                runFlags = flags;
                runActive = true;
            }
        }
        flush();
    }
    painter->setFont(m_font);

    if (m_cursorVisible && (!(m_cursorBlinking || m_blinkingCursor) || m_cursorOn)) {
        QColor fill = m_hasCursorColor ? m_cursorColor : m_fg;
        fill.setAlpha(160);
        const QRectF rect(m_cursorX * cellW, m_cursorY * cellH, cellW, cellH);
        if (!m_hasFocus) {
            painter->setPen(fill);
            painter->drawRect(rect.adjusted(0.5, 0.5, -0.5, -0.5));
        } else {
        switch (m_cursorShape) {
        case 1: // bar
            painter->fillRect(QRectF(rect.left(), rect.top(), 2, cellH), fill);
            break;
        case 2: // underline
            painter->fillRect(QRectF(rect.left(), rect.bottom() - 2, cellW, 2), fill);
            break;
        case 3: // hollow block
            painter->setPen(m_hasCursorColor ? m_cursorColor : m_fg);
            painter->drawRect(rect.adjusted(0.5, 0.5, -0.5, -0.5));
            break;
        default: // block
            painter->fillRect(rect, fill);
            break;
        }
        }
    }

    if (!m_preedit.isEmpty()) {
        const QRectF rect(m_cursorX * cellW, m_cursorY * cellH, m_preedit.size() * cellW, cellH);
        painter->setPen(m_fg);
        painter->drawText(rect, Qt::AlignLeft | Qt::AlignVCenter | Qt::TextDontClip, m_preedit);
        painter->drawLine(QPointF(rect.left(), rect.bottom() - 1),
                          QPointF(rect.right(), rect.bottom() - 1));
    }

    m_dirtyRows.clear();
    m_fullRedraw = false;

    static const bool logPaint = qEnvironmentVariableIsSet("GHOSTTY_SPIKE_LOG");
    if (logPaint) {
        std::fprintf(stderr, "ghostty spike: paint %dx%d in %.2f ms\n", m_cols, m_rows,
                     static_cast<double>(timer.nsecsElapsed()) / 1e6);
    }
}

} // namespace termigator::term
