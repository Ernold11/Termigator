#pragma once

// GhosttySession: one terminal session on the Rust engine.
//
// PTY ownership stays in Rust (`termigator_core::proc::pty`): `attachFd` duplicates the master,
// reads it on the GUI thread, calls `on_output` (Rust taps the raw bytes first) and then feeds the
// engine. Program replies (DSR/DA, OSC 52 answers) come back from the engine as queued bytes and
// are written to the pty. Views draw the packed snapshot (`snapshotBinary`).
//
// Registered as the application's `ExternalSession` (registerGhosttyTypes) and, under its own
// name, in the spike module (`registerSpikeTypes`).

#include <QtCore/QByteArray>
#include <QtCore/QObject>
#include <QtCore/QStringList>

#include "rust/cxx.h"

class QSocketNotifier;

namespace termigator::term {

struct SessionEngine;

class GhosttySession : public QObject
{
    Q_OBJECT
    Q_PROPERTY(quint64 sessionId READ sessionId WRITE setSessionId NOTIFY sessionIdChanged)
    Q_PROPERTY(bool attached READ attached NOTIFY attachedChanged)
    Q_PROPERTY(bool finished READ finished NOTIFY finishedChanged)
    Q_PROPERTY(QString title READ title NOTIFY titleChanged)
    Q_PROPERTY(QString cwd READ cwd NOTIFY cwdChanged)
    Q_PROPERTY(QString history READ historyText NOTIFY historyChanged)
    Q_PROPERTY(QString colorScheme READ colorScheme WRITE setColorScheme NOTIFY colorSchemeChanged)
    Q_PROPERTY(int historySize READ historySize WRITE setHistorySize NOTIFY historySizeChanged)
    Q_PROPERTY(bool monitorActivity READ monitorActivity WRITE setMonitorActivity NOTIFY monitorActivityChanged)

public:
    explicit GhosttySession(QObject *parent = nullptr);
    ~GhosttySession() override;

    static GhosttySession *find(quint64 id);
    quint64 sessionId() const { return m_id; }
    void setSessionId(quint64 id);

    bool attached() const { return m_fd >= 0 || m_feed; }
    bool finished() const { return m_finished; }
    QString title() const { return m_title; }
    QString cwd() const { return m_cwd; }
    QString colorScheme() const { return m_schemeName; }
    void setColorScheme(const QString &name);
    int historySize() const { return m_historySize; }
    void setHistorySize(int lines);
    bool monitorActivity() const { return m_monitorActivity; }
    void setMonitorActivity(bool monitor);

    Q_INVOKABLE void attachFd(int fd);
    /// Feed mode (always on Windows): no descriptor here. Rust reads the pty on a thread and calls
    /// `postFeedPtyGhostty`; input goes back through `on_write`, resizes through `on_resize`.
    Q_INVOKABLE void attachFeed();
    Q_INVOKABLE void detach();
    Q_INVOKABLE void sendText(const QString &text);
    Q_INVOKABLE void sendBytes(const QByteArray &data);
    Q_INVOKABLE void pasteText(const QString &text);
    Q_INVOKABLE void pasteTextMode(const QString &text, bool bracketed);
    Q_INVOKABLE bool pasteSafe(const QString &text);
    Q_INVOKABLE bool bracketedPaste();
    Q_INVOKABLE bool findNext(const QString &needle, bool caseSensitive, bool regex, bool forward,
                              bool current);
    Q_INVOKABLE QString urlAt(int col, int row);
    static QStringList availableSchemes();
    Q_INVOKABLE void sendKey(int key, int modifiers, const QString &text);
    Q_INVOKABLE void resizeTerminal(int cols, int rows);
    Q_INVOKABLE void clear();
    Q_INVOKABLE void scrollBy(int rows);
    Q_INVOKABLE void scrollToRow(int row);
    Q_INVOKABLE void select(int startCol, int startRow, int endCol, int endRow, bool rectangle);
    Q_INVOKABLE void selectWord(int col, int row);
    Q_INVOKABLE void selectLine(int col, int row);
    Q_INVOKABLE void clearSelection();
    Q_INVOKABLE QString selectedText();
    Q_INVOKABLE int scrollbarTotal();
    Q_INVOKABLE int scrollbarOffset();
    Q_INVOKABLE int scrollbarLen();
    Q_INVOKABLE QByteArray encodeMouse(int x, int y, int button, int action, int modifiers,
                                       int cellWidth, int cellHeight);
    Q_INVOKABLE QString snapshotJson();
    Q_INVOKABLE QByteArray snapshotBinary();
    Q_INVOKABLE QString historyText();
    Q_INVOKABLE QString scrollbarJson();
    Q_INVOKABLE void feedBytes(const QByteArray &data);

    /// Output of the pty, from the GUI thread: Rust taps it, then the engine eats it.
    void handleOutput(const char *data, qsizetype size);
    /// The child side is closed: detaches and tells Rust.
    void handleEof();

signals:
    void sessionIdChanged();
    void attachedChanged();
    void finishedChanged();
    void titleChanged();
    void cwdChanged();
    void historyChanged();
    void colorSchemeChanged();
    void historySizeChanged();
    void monitorActivityChanged();
    void bell(const QString &message);
    void activity();
    void clipboardWrite(bool selection, const QString &text);
    void notification(const QString &title, const QString &body);
    void updated();

private:
    void onReadable();
    void onWritable();
    void flushPending();
    void drainEvents();
    void setFinished(bool finished);

    quint64 m_id = 0;
    int m_fd = -1;
    bool m_feed = false;
    int m_cols = 80;
    int m_rows = 24;
    int m_historySize = -1;
    bool m_finished = false;
    bool m_monitorActivity = false;
    bool m_activitySent = false;
    QString m_title;
    QString m_cwd;
    QString m_schemeName;
    QByteArray m_pending;
    QSocketNotifier *m_readNotifier = nullptr;
    QSocketNotifier *m_writeNotifier = nullptr;
    rust::Box<SessionEngine> m_engine;
};

/// Attaches a pty master to the live session with this id; throws `std::runtime_error` otherwise.
void attachPtyGhostty(std::uint64_t id, int fd);

/// Puts the live session with this id in feed mode; throws `std::runtime_error` otherwise.
void attachPtyFeed(std::uint64_t id);

/// From any thread: output of the session's pty, delivered in order on the GUI thread (dropped
/// when the session is gone).
void postFeedPtyGhostty(std::uint64_t id, rust::Slice<const std::uint8_t> data);

/// From any thread: the pty is closed; delivered after the output posted before it.
void postEofPtyGhostty(std::uint64_t id);

} // namespace termigator::term
