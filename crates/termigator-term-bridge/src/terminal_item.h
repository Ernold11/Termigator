#pragma once

// The terminal view: a QQuickPaintedItem that draws a GhosttySession (registered as QMLTermWidget).
//
// It paints from the session's JSON snapshot (text, colours, styles, cursor). Born as the first
// prototype ("spike"); a packed buffer with partial updates would be the next step if painting
// ever shows up in profiles.

#include <QtCore/QElapsedTimer>
#include <QtCore/QPoint>
#include <QtCore/QPointF>
#include <QtCore/QPointer>
#include <QtCore/QString>
#include <QtCore/QStringList>
#include <QtCore/QVector>
#include <QtGui/QColor>
#include <QtGui/QFont>
#include <QtQuick/QQuickPaintedItem>

#include "termigator-term-bridge/src/ghostty_session.h"

class QFocusEvent;
class QInputMethodEvent;
class QKeyEvent;
class QMouseEvent;
class QTimer;
class QWheelEvent;

namespace termigator::term {

class GhosttyTerminalItem : public QQuickPaintedItem
{
    Q_OBJECT
    Q_PROPERTY(GhosttySession *session READ session WRITE setSession NOTIFY sessionChanged)
    Q_PROPERTY(QString source READ source WRITE setSource NOTIFY sourceChanged)
    Q_PROPERTY(qreal cellWidth READ cellWidth CONSTANT)
    Q_PROPERTY(qreal cellHeight READ cellHeight CONSTANT)
    Q_PROPERTY(int scrollbarCurrentValue READ scrollbarCurrentValue NOTIFY scrollbarChanged)
    Q_PROPERTY(int scrollbarMaximum READ scrollbarMaximum NOTIFY scrollbarChanged)
    Q_PROPERTY(int scrollbarMinimum READ scrollbarMinimum NOTIFY scrollbarChanged)
    Q_PROPERTY(bool copyOnSelect READ copyOnSelect WRITE setCopyOnSelect NOTIFY copyOnSelectChanged)
    Q_PROPERTY(QFont font READ font WRITE setFont NOTIFY fontChanged)
    Q_PROPERTY(QString colorScheme READ colorScheme WRITE setColorScheme NOTIFY colorSchemeChanged)
    Q_PROPERTY(bool blinkingCursor READ blinkingCursor WRITE setBlinkingCursor NOTIFY blinkingCursorChanged)
    Q_PROPERTY(bool bracketedPasteDisabled READ bracketedPasteDisabled WRITE setBracketedPasteDisabled NOTIFY bracketedPasteDisabledChanged)
    Q_PROPERTY(bool confirmMultilinePaste READ confirmMultilinePaste WRITE setConfirmMultilinePaste NOTIFY confirmMultilinePasteChanged)
    Q_PROPERTY(bool pasteOnRightClick READ pasteOnRightClick WRITE setPasteOnRightClick NOTIFY pasteOnRightClickChanged)
    Q_PROPERTY(bool pasteOnMiddleClick READ pasteOnMiddleClick WRITE setPasteOnMiddleClick NOTIFY pasteOnMiddleClickChanged)
    Q_PROPERTY(QColor fillColor READ fillColor NOTIFY fillColorChanged)
    Q_PROPERTY(int lines READ lines NOTIFY linesChanged)
    Q_PROPERTY(QStringList availableColorSchemes READ availableColorSchemes CONSTANT)

public:
    explicit GhosttyTerminalItem(QQuickItem *parent = nullptr);
    ~GhosttyTerminalItem() override;

    void paint(QPainter *painter) override;

    GhosttySession *session() const { return m_session; }
    void setSession(GhosttySession *session);
    QString source() const { return m_source; }
    void setSource(const QString &path);
    qreal cellWidth() const { return m_cellWidth; }
    qreal cellHeight() const { return m_cellHeight; }
    int scrollbarCurrentValue() const { return m_scrollValue; }
    int scrollbarMaximum() const { return m_scrollMaximum; }
    int scrollbarMinimum() const { return 0; }
    bool copyOnSelect() const { return m_copyOnSelect; }
    void setCopyOnSelect(bool copy);
    QFont font() const { return m_font; }
    void setFont(const QFont &font);
    QString colorScheme() const;
    void setColorScheme(const QString &name);
    bool blinkingCursor() const { return m_blinkingCursor; }
    void setBlinkingCursor(bool blink);
    bool bracketedPasteDisabled() const { return m_bracketedPasteDisabled; }
    void setBracketedPasteDisabled(bool disabled);
    bool confirmMultilinePaste() const { return m_confirmMultilinePaste; }
    void setConfirmMultilinePaste(bool confirm);
    bool pasteOnRightClick() const { return m_pasteOnRightClick; }
    void setPasteOnRightClick(bool paste);
    bool pasteOnMiddleClick() const { return m_pasteOnMiddleClick; }
    void setPasteOnMiddleClick(bool paste);
    QColor fillColor() const { return m_bg; }
    int lines() const { return m_rows; }
    QStringList availableColorSchemes() const;

    Q_INVOKABLE void feed(const QByteArray &data);
    Q_INVOKABLE void feedFile(const QString &path);
    Q_INVOKABLE QString text() const;
    Q_INVOKABLE void scrollBy(int rows);
    Q_INVOKABLE void sendText(const QString &text);
    Q_INVOKABLE void sendKey(int key, int modifiers, const QString &text);
    Q_INVOKABLE void select(int startCol, int startRow, int endCol, int endRow, bool rectangle);
    Q_INVOKABLE void selectWord(int col, int row);
    Q_INVOKABLE void selectLine(int col, int row);
    Q_INVOKABLE QString selectedText();
    Q_INVOKABLE QString selectedText(bool trim);
    Q_INVOKABLE void copyClipboard();
    Q_INVOKABLE void pasteClipboard();
    Q_INVOKABLE void pasteText(const QString &text, bool appendReturn);
    Q_INVOKABLE bool findNext(const QString &needle, bool caseSensitive, bool regex, bool forward,
                              bool current);
    Q_INVOKABLE QString hotspotUrlAt(int x, int y);
    Q_INVOKABLE void pasteSelection();
    Q_INVOKABLE void setScrollbarValue(int value);
    Q_INVOKABLE void zoomIn();
    Q_INVOKABLE void zoomOut();
    Q_INVOKABLE void resetZoom();

protected:
    void keyPressEvent(QKeyEvent *event) override;
    void inputMethodEvent(QInputMethodEvent *event) override;
    QVariant inputMethodQuery(Qt::InputMethodQuery query) const override;
    void focusInEvent(QFocusEvent *event) override;
    void focusOutEvent(QFocusEvent *event) override;
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void mouseUngrabEvent() override;
    void wheelEvent(QWheelEvent *event) override;

signals:
    void sessionChanged();
    void sourceChanged();
    void scrollbarChanged();
    void copyOnSelectChanged();
    void selectionChanged();
    void fontChanged();
    void colorSchemeChanged();
    void blinkingCursorChanged();
    void bracketedPasteDisabledChanged();
    void confirmMultilinePasteChanged();
    void pasteOnRightClickChanged();
    void pasteOnMiddleClickChanged();
    void fillColorChanged();
    void linesChanged();
    void searchResult(bool found);
    void multilinePasteRequested(const QString &text, bool appendReturn);
    void linkActivated(const QString &url);
    void configureRequest(const QPoint &position);

private:
    // Every paste goes here: replayed to MultiExec targets, then written (bracketed if allowed).
    void paste(const QString &text);
    struct CellData {
        int x = 0;
        int width = 1;
        quint16 flags = 0;
        QColor fg;
        QColor bg;
        quint32 textOffset = 0;
        quint16 textLength = 0;
        QString text;
    };
    struct RowData {
        bool wrapped = false;
        QVector<CellData> cells;
    };

    void ensureSnapshot();
    void parseSnapshot();
    void updateBlink();
    void refresh();
    void requestFrame();
    void updateGrid();
    void updateScrollbar();
    QPoint cellAt(const QPointF &pos) const;
    void finishSelection();
    // Extends the drag selection from the anchor to the pointer's cell in the current view.
    void updateDragSelection();
    // Starts or stops the auto-scroll timer as the pointer leaves or re-enters the rows.
    void updateAutoScroll();
    void autoScrollStep();
    void stopSelecting();

    // The tab owns the session and may destroy it before this view (closing a tab, quitting);
    // a pending refresh must then see null instead of a freed session.
    QPointer<GhosttySession> m_session;
    QString m_source;
    QFont m_font;
    qreal m_cellWidth = 8;
    qreal m_cellHeight = 16;
    bool m_snapshotStale = true;
    bool m_valid = false;
    int m_cols = 0;
    int m_rows = 0;
    int m_cursorX = 0;
    int m_cursorY = 0;
    int m_cursorShape = 0;
    bool m_cursorVisible = false;
    bool m_cursorBlinking = false;
    bool m_cursorOn = true;
    bool m_hasCursorColor = false;
    QColor m_fg = Qt::white;
    QColor m_bg = Qt::black;
    QColor m_cursorColor = Qt::white;
    QVector<RowData> m_rowsData;
    int m_scrollValue = 0;
    int m_scrollMaximum = 0;
    bool m_copyOnSelect = true;
    bool m_hasFocus = false;
    bool m_blinkingCursor = false;
    bool m_bracketedPasteDisabled = false;
    bool m_confirmMultilinePaste = true;
    bool m_pasteOnRightClick = false;
    bool m_pasteOnMiddleClick = true;
    QString m_preedit;
    bool m_selecting = false;
    // The cell a drag selection started at; y is a screen row (0 = the oldest scrollback row),
    // so the anchor stays put while the view scrolls under the drag.
    QPoint m_anchor;
    QPointF m_dragPos;
    bool m_dragRectangle = false;
    QTimer *m_autoScrollTimer = nullptr;
    QElapsedTimer m_autoScrollClock;
    qreal m_autoScrollRows = 0;
    // Wheel turn (eighths of a degree) not yet a whole step: touchpads send small deltas.
    int m_wheelDelta = 0;
    qreal m_basePointSize = 10.5;
    QTimer *m_blinkTimer = nullptr;
    QTimer *m_refreshTimer = nullptr;
    QVector<int> m_dirtyRows;
    bool m_fullRedraw = true;
};

} // namespace termigator::term
