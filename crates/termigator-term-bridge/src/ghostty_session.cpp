#include "termigator-term-bridge/src/ghostty_session.h"

#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QFileInfo>
#include <QtCore/QHash>
#include <QtCore/QStringList>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QCoreApplication>
#include <QtCore/QMetaObject>
#include <QtCore/QSocketNotifier>
#include <QtCore/QtDebug>

#include <cerrno>
#include <cstring>
#include <stdexcept>
#include <string>
#ifndef _WIN32
#include <fcntl.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>
#endif

#include "termigator-term-bridge/src/lib.rs.h"

namespace termigator::term {

namespace {

// Live sessions by id; GUI thread only.
QHash<quint64, GhosttySession *> &registry()
{
    static QHash<quint64, GhosttySession *> sessions;
    return sessions;
}

constexpr int READ_CHUNK = 16384;

} // namespace

GhosttySession::GhosttySession(QObject *parent)
    : QObject(parent)
    , m_engine(new_session_engine(80, 24, -1))
{
    // Konsole's profile decides the first palette (Breeze by default); QML may override it.
    m_schemeName =
        QString::fromStdString(std::string(m_engine->apply_konsole_default_scheme()));
}

GhosttySession::~GhosttySession()
{
    detach();
    if (m_id != 0 && registry().value(m_id) == this)
        registry().remove(m_id);
}

GhosttySession *GhosttySession::find(quint64 id)
{
    return registry().value(id);
}

void GhosttySession::setSessionId(quint64 id)
{
    if (id == m_id)
        return;
    if (m_id != 0) {
        qWarning("GhosttySession: sessionId is set once (%llu), ignoring %llu",
                 static_cast<unsigned long long>(m_id), static_cast<unsigned long long>(id));
        return;
    }
    if (id == 0 || registry().contains(id)) {
        qWarning("GhosttySession: sessionId %llu is zero or already in use",
                 static_cast<unsigned long long>(id));
        return;
    }
    m_id = id;
    registry().insert(id, this);
    emit sessionIdChanged();
}

void GhosttySession::setColorScheme(const QString &name)
{
    if (name == m_schemeName)
        return;
    if (name.isEmpty()) {
        m_schemeName = QString::fromStdString(
            std::string(m_engine->apply_konsole_default_scheme()));
        emit colorSchemeChanged();
        emit updated();
        return;
    }
    const QByteArray utf8 = name.toUtf8();
    std::string schemeName(utf8.constData(), static_cast<size_t>(utf8.size()));
    bool applied = m_engine->set_scheme(schemeName);
    if (!applied) {
        // The built-in schemes live in this crate's resource, not on disk.
        QFile file(QStringLiteral(":/termigator/color-schemes/%1.colorscheme").arg(name));
        if (file.open(QIODevice::ReadOnly)) {
            const QByteArray text = file.readAll();
            applied = m_engine->set_scheme_text(
                std::string(text.constData(), static_cast<size_t>(text.size())));
        }
    }
    if (!applied)
        return;
    m_schemeName = name;
    emit colorSchemeChanged();
    emit updated();
}

void GhosttySession::setHistorySize(int lines)
{
    if (lines == m_historySize)
        return;
    m_historySize = lines;
    const std::int64_t limit = lines < 0 ? -1 : lines;
    m_engine->set_history(limit);
    emit historySizeChanged();
}

void GhosttySession::setMonitorActivity(bool monitor)
{
    if (monitor == m_monitorActivity)
        return;
    m_monitorActivity = monitor;
    // Activity fires once per arming; writing true again re-arms it (same as Konsole).
    m_activitySent = false;
    emit monitorActivityChanged();
}

void GhosttySession::setFinished(bool finished)
{
    if (finished == m_finished)
        return;
    m_finished = finished;
    emit finishedChanged();
}

void GhosttySession::attachFd(int fd)
{
#ifdef _WIN32
    Q_UNUSED(fd)
    throw std::runtime_error("attachFd: a pty has no descriptor on Windows, use attachFeed");
#else
    const int own = fcntl(fd, F_DUPFD_CLOEXEC, 0);
    if (own < 0)
        throw std::runtime_error(std::string("cannot duplicate the pty fd: ") + std::strerror(errno));
    detach();

    // Writes queue instead of blocking the GUI thread on a child that does not read. The flag is
    // on the shared open file description, so Rust's own copy becomes non-blocking too.
    fcntl(own, F_SETFL, fcntl(own, F_GETFL) | O_NONBLOCK);
    // UTF-8 line editing erases whole characters, like Konsole's Pty.
    termios tio{};
    if (tcgetattr(own, &tio) == 0) {
        tio.c_iflag |= IUTF8;
        tcsetattr(own, TCSANOW, &tio);
    }
    m_fd = own;
    resizeTerminal(m_cols, m_rows, m_cellWidth, m_cellHeight);

    m_readNotifier = new QSocketNotifier(own, QSocketNotifier::Read, this);
    connect(m_readNotifier, &QSocketNotifier::activated, this, [this] { onReadable(); });
    m_writeNotifier = new QSocketNotifier(own, QSocketNotifier::Write, this);
    m_writeNotifier->setEnabled(false);
    connect(m_writeNotifier, &QSocketNotifier::activated, this, [this] { onWritable(); });
    emit attachedChanged();
    emit updated();
#endif
}

void GhosttySession::attachFeed()
{
    detach();
    m_feed = true;
    resizeTerminal(m_cols, m_rows, m_cellWidth, m_cellHeight);
    emit attachedChanged();
    emit updated();
}

void GhosttySession::detach()
{
    if (!attached())
        return;
#ifndef _WIN32
    if (m_fd >= 0) {
        // Notifiers stop before the fd is closed (and its number reused); deleteLater because this
        // may run inside the read notifier's own activated() signal.
        for (QSocketNotifier *notifier : {m_readNotifier, m_writeNotifier}) {
            notifier->setEnabled(false);
            notifier->deleteLater();
        }
        m_readNotifier = m_writeNotifier = nullptr;
        ::close(m_fd);
        m_fd = -1;
    }
#endif
    m_feed = false;
    m_pending.clear();
    emit attachedChanged();
    emit updated();
}

void GhosttySession::handleOutput(const char *data, qsizetype size)
{
    // Rust first: it may re-attach or tear down; the engine is still ours.
    if (m_id != 0) {
        on_output(m_id, rust::Slice<const std::uint8_t>(
                             reinterpret_cast<const std::uint8_t *>(data),
                             static_cast<std::size_t>(size)));
    }
    m_engine->feed(rust::Slice<const std::uint8_t>(
        reinterpret_cast<const std::uint8_t *>(data), static_cast<std::size_t>(size)));
    if (m_monitorActivity && !m_activitySent) {
        m_activitySent = true;
        emit activity();
    }
    drainEvents();
    emit updated();
}

void GhosttySession::handleEof()
{
    // Both the reader thread and the app's exit check may report it; the second one is a no-op.
    if (!attached() && m_finished)
        return;
    detach();
    if (m_id != 0)
        on_eof(m_id);
    setFinished(true);
}

void GhosttySession::onReadable()
{
#ifndef _WIN32
    char buffer[READ_CHUNK];
    const ssize_t n = ::read(m_fd, buffer, sizeof buffer);
    if (n > 0) {
        handleOutput(buffer, static_cast<qsizetype>(n));
        return;
    }
    if (n < 0 && (errno == EAGAIN || errno == EINTR))
        return;
    // 0 or EIO: every slave fd is closed, the child is gone or has let go of its terminal.
    handleEof();
#endif
}

void GhosttySession::drainEvents()
{
    const QByteArray json = QByteArray::fromStdString(std::string(m_engine->events_json()));
    const QJsonDocument document = QJsonDocument::fromJson(json);
    for (const QJsonValue &value : document.array()) {
        const QJsonObject event = value.toObject();
        const QString type = event.value(QStringLiteral("t")).toString();
        if (type == QStringLiteral("pty")) {
            m_pending.append(QByteArray::fromHex(
                event.value(QStringLiteral("d")).toString().toLatin1()));
            flushPending();
        } else if (type == QStringLiteral("bell")) {
            emit bell(QStringLiteral("Bell in session '%1'").arg(m_title));
        } else if (type == QStringLiteral("title")) {
            const QString title = event.value(QStringLiteral("v")).toString();
            if (title != m_title) {
                m_title = title;
                emit titleChanged();
            }
        } else if (type == QStringLiteral("pwd")) {
            const QString cwd = event.value(QStringLiteral("v")).toString();
            if (cwd != m_cwd) {
                m_cwd = cwd;
                emit cwdChanged();
            }
        } else if (type == QStringLiteral("clip")) {
            emit clipboardWrite(event.value(QStringLiteral("sel")).toBool(),
                                event.value(QStringLiteral("v")).toString());
        } else if (type == QStringLiteral("notify")) {
            const QString text = event.value(QStringLiteral("v")).toString();
            const qsizetype split = text.indexOf(QLatin1Char('\n'));
            emit notification(text.left(split), split >= 0 ? text.mid(split + 1) : QString());
        }
    }
}

void GhosttySession::onWritable()
{
    flushPending();
}

void GhosttySession::flushPending()
{
    if (m_feed) {
        // Rust owns the writer and must not block the GUI thread; it queues the bytes itself.
        if (!m_pending.isEmpty() && m_id != 0) {
            on_write(m_id, rust::Slice<const std::uint8_t>(
                               reinterpret_cast<const std::uint8_t *>(m_pending.constData()),
                               static_cast<std::size_t>(m_pending.size())));
        }
        m_pending.clear();
        return;
    }
#ifdef _WIN32
    m_pending.clear();
#else
    const int fd = m_fd;
    while (!m_pending.isEmpty() && fd >= 0) {
        const ssize_t n = ::write(fd, m_pending.constData(), static_cast<size_t>(m_pending.size()));
        if (n > 0) {
            m_pending.remove(0, n);
        } else if (n < 0 && errno == EINTR) {
            continue;
        } else if (n < 0 && errno == EAGAIN) {
            if (m_writeNotifier != nullptr)
                m_writeNotifier->setEnabled(true);
            return;
        } else {
            // The child side is gone; the read notifier reports EOF.
            m_pending.clear();
        }
    }
    if (m_writeNotifier != nullptr)
        m_writeNotifier->setEnabled(false);
#endif
}

void GhosttySession::sendText(const QString &text)
{
    if (!attached())
        return;
    m_pending.append(text.toUtf8());
    flushPending();
}

void GhosttySession::sendBytes(const QByteArray &data)
{
    if (!attached())
        return;
    m_pending.append(data);
    flushPending();
}

void GhosttySession::pasteText(const QString &text)
{
    if (!attached())
        return;
    const QByteArray utf8 = text.toUtf8();
    const rust::Vec<std::uint8_t> bytes = m_engine->paste(
        std::string(utf8.constData(), static_cast<size_t>(utf8.size())));
    m_pending.append(reinterpret_cast<const char *>(bytes.data()),
                     static_cast<qsizetype>(bytes.size()));
    flushPending();
}

void GhosttySession::pasteTextMode(const QString &text, bool bracketed)
{
    if (!attached())
        return;
    const QByteArray utf8 = text.toUtf8();
    const rust::Vec<std::uint8_t> bytes = m_engine->paste_mode(
        std::string(utf8.constData(), static_cast<size_t>(utf8.size())), bracketed);
    m_pending.append(reinterpret_cast<const char *>(bytes.data()),
                     static_cast<qsizetype>(bytes.size()));
    flushPending();
}

bool GhosttySession::pasteSafe(const QString &text)
{
    const QByteArray utf8 = text.toUtf8();
    return paste_is_safe(std::string(utf8.constData(), static_cast<size_t>(utf8.size())));
}

bool GhosttySession::bracketedPaste()
{
    return m_engine->bracketed_paste();
}

void GhosttySession::sendKey(int key, int modifiers, const QString &text)
{
    if (!attached())
        return;
    const QByteArray utf8 = text.toUtf8();
    const rust::Vec<std::uint8_t> bytes = m_engine->encode_qt_key(
        key, static_cast<std::uint32_t>(modifiers),
        std::string(utf8.constData(), static_cast<size_t>(utf8.size())));
    m_pending.append(reinterpret_cast<const char *>(bytes.data()),
                     static_cast<qsizetype>(bytes.size()));
    flushPending();
}

void GhosttySession::resizeTerminal(int cols, int rows, int cellWidth, int cellHeight)
{
    cols = qMax(1, cols);
    rows = qMax(1, rows);
    m_cols = cols;
    m_rows = rows;
    m_cellWidth = qBound(1, cellWidth, 0xffff);
    m_cellHeight = qBound(1, cellHeight, 0xffff);
    m_engine->resize(static_cast<std::uint16_t>(cols), static_cast<std::uint16_t>(rows),
                     static_cast<std::uint16_t>(m_cellWidth),
                     static_cast<std::uint16_t>(m_cellHeight));
    if (m_feed) {
        // Rust resizes the pty (ResizePseudoConsole / TIOCSWINSZ) in its on_resize handler.
        if (m_id != 0)
            on_resize(m_id, static_cast<std::uint16_t>(cols), static_cast<std::uint16_t>(rows));
        return;
    }
#ifndef _WIN32
    if (m_fd < 0)
        return;
    winsize ws{};
    ws.ws_row = static_cast<unsigned short>(rows);
    ws.ws_col = static_cast<unsigned short>(cols);
    // What TIOCGWINSZ tells image tools (chafa, sixel, the kitty protocol) about the pixels.
    ws.ws_xpixel = static_cast<unsigned short>(qMin(0xffff, cols * m_cellWidth));
    ws.ws_ypixel = static_cast<unsigned short>(qMin(0xffff, rows * m_cellHeight));
    if (ioctl(m_fd, TIOCSWINSZ, &ws) == 0 && m_id != 0)
        on_resize(m_id, ws.ws_col, ws.ws_row);
#endif
}

void GhosttySession::clear()
{
    m_engine->clear();
    emit historyChanged();
    emit updated();
}

void GhosttySession::scrollBy(int rows)
{
    m_engine->scroll(rows);
    emit updated();
}

void GhosttySession::scrollToRow(int row)
{
    m_engine->scroll_to_row(row);
    emit updated();
}

void GhosttySession::select(int startCol, int startRow, int endCol, int endRow, bool rectangle)
{
    m_engine->select(static_cast<std::uint16_t>(qMax(0, startCol)),
                     static_cast<std::uint16_t>(qMax(0, startRow)),
                     static_cast<std::uint16_t>(qMax(0, endCol)),
                     static_cast<std::uint16_t>(qMax(0, endRow)), rectangle);
    emit updated();
}

void GhosttySession::selectScreen(int startCol, int startRow, int endCol, int endRow,
                                  bool rectangle)
{
    m_engine->select_screen(static_cast<std::uint16_t>(qMax(0, startCol)),
                            static_cast<std::uint32_t>(qMax(0, startRow)),
                            static_cast<std::uint16_t>(qMax(0, endCol)),
                            static_cast<std::uint32_t>(qMax(0, endRow)), rectangle);
    emit updated();
}

void GhosttySession::selectWord(int col, int row)
{
    m_engine->select_word(static_cast<std::uint16_t>(qMax(0, col)),
                          static_cast<std::uint16_t>(qMax(0, row)));
    emit updated();
}

void GhosttySession::selectLine(int col, int row)
{
    m_engine->select_line(static_cast<std::uint16_t>(qMax(0, col)),
                          static_cast<std::uint16_t>(qMax(0, row)));
    emit updated();
}

void GhosttySession::clearSelection()
{
    m_engine->clear_selection();
    emit updated();
}

QString GhosttySession::selectedText()
{
    return QString::fromStdString(std::string(m_engine->selected_text()));
}

int GhosttySession::scrollbarTotal()
{
    return static_cast<int>(m_engine->scrollbar_total());
}

int GhosttySession::scrollbarOffset()
{
    return static_cast<int>(m_engine->scrollbar_offset());
}

int GhosttySession::scrollbarLen()
{
    return static_cast<int>(m_engine->scrollbar_len());
}

QByteArray GhosttySession::encodeMouse(int x, int y, int button, int action, int modifiers,
                                       int cellWidth, int cellHeight)
{
    const rust::Vec<std::uint8_t> bytes = m_engine->encode_mouse(
        static_cast<std::uint16_t>(qMax(0, x)), static_cast<std::uint16_t>(qMax(0, y)), button,
        action, static_cast<std::uint32_t>(modifiers),
        (static_cast<std::uint32_t>(qMax(1, cellWidth)) << 16)
            | static_cast<std::uint32_t>(qMax(1, cellHeight)));
    return QByteArray(reinterpret_cast<const char *>(bytes.data()),
                      static_cast<qsizetype>(bytes.size()));
}

int GhosttySession::wheelTarget(int modifiers)
{
    return m_engine->wheel_target(static_cast<std::uint32_t>(modifiers));
}

QByteArray GhosttySession::encodeWheel(int x, int y, int steps, int modifiers, int cellWidth,
                                       int cellHeight)
{
    const rust::Vec<std::uint8_t> bytes = m_engine->encode_wheel(
        static_cast<std::uint16_t>(qMax(0, x)), static_cast<std::uint16_t>(qMax(0, y)), steps,
        static_cast<std::uint32_t>(modifiers),
        (static_cast<std::uint32_t>(qMax(1, cellWidth)) << 16)
            | static_cast<std::uint32_t>(qMax(1, cellHeight)));
    return QByteArray(reinterpret_cast<const char *>(bytes.data()),
                      static_cast<qsizetype>(bytes.size()));
}

QString GhosttySession::snapshotJson()
{
    return QString::fromStdString(std::string(m_engine->snapshot_json()));
}

QByteArray GhosttySession::snapshotBinary()
{
    const rust::Vec<std::uint8_t> bytes = m_engine->snapshot_binary();
    return QByteArray(reinterpret_cast<const char *>(bytes.data()),
                      static_cast<qsizetype>(bytes.size()));
}

bool GhosttySession::findNext(const QString &needle, bool caseSensitive, bool regex, bool forward,
                              bool current)
{
    const QByteArray utf8 = needle.toUtf8();
    return m_engine->find_next(
        std::string(utf8.constData(), static_cast<size_t>(utf8.size())), caseSensitive, regex,
        forward, current);
}

QString GhosttySession::urlAt(int col, int row)
{
    return QString::fromStdString(
        std::string(m_engine->url_at(static_cast<std::uint16_t>(qMax(0, col)),
                                     static_cast<std::uint16_t>(qMax(0, row)))));
}

QStringList GhosttySession::availableSchemes()
{
    QStringList names;
    for (const rust::String &name : available_schemes())
        names.append(QString::fromStdString(std::string(name)));
    // The schemes this build carries (our own definitions, MIT).
    const QDir resources(QStringLiteral(":/termigator/color-schemes"));
    for (const QString &file : resources.entryList({QStringLiteral("*.colorscheme")}, QDir::Files)) {
        const QString name = QFileInfo(file).completeBaseName();
        if (!names.contains(name))
            names.append(name);
    }
    names.sort(Qt::CaseInsensitive);
    return names;
}

QString GhosttySession::historyText()
{
    return QString::fromStdString(std::string(m_engine->history_text()));
}

QString GhosttySession::scrollbarJson()
{
    return QString::fromStdString(std::string(m_engine->scrollbar_json()));
}

void GhosttySession::feedBytes(const QByteArray &data)
{
    m_engine->feed(rust::Slice<const std::uint8_t>(
        reinterpret_cast<const std::uint8_t *>(data.constData()),
        static_cast<std::size_t>(data.size())));
    drainEvents();
    emit updated();
}

void attachPtyGhostty(std::uint64_t id, int fd)
{
    GhosttySession *session = registry().value(id);
    if (session == nullptr)
        throw std::runtime_error("no GhosttySession with sessionId " + std::to_string(id));
    session->attachFd(fd);
}

void attachPtyFeed(std::uint64_t id)
{
    GhosttySession *session = registry().value(id);
    if (session == nullptr)
        throw std::runtime_error("no GhosttySession with sessionId " + std::to_string(id));
    session->attachFeed();
}

void postFeedPtyGhostty(std::uint64_t id, rust::Slice<const std::uint8_t> data)
{
    QCoreApplication *app = QCoreApplication::instance();
    if (app == nullptr)
        return;
    const QByteArray bytes(reinterpret_cast<const char *>(data.data()),
                           static_cast<qsizetype>(data.size()));
    QMetaObject::invokeMethod(
        app,
        [id, bytes] {
            if (GhosttySession *session = registry().value(id))
                session->handleOutput(bytes.constData(), bytes.size());
        },
        Qt::QueuedConnection);
}

void postEofPtyGhostty(std::uint64_t id)
{
    QCoreApplication *app = QCoreApplication::instance();
    if (app == nullptr)
        return;
    QMetaObject::invokeMethod(
        app,
        [id] {
            if (GhosttySession *session = registry().value(id))
                session->handleEof();
        },
        Qt::QueuedConnection);
}

} // namespace termigator::term
