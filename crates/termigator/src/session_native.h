#pragma once

// The desktop session is ending (logout, shutdown, a Windows WM_QUERYENDSESSION): the session
// manager's commitDataRequest. qml/shell/ShellWindow.qml then closes for good without asking, so a
// system close never waits on the close dialog. GUI thread only, after the QApplication exists.

#include <QtCore/QObject>
#include <QtQml/qqmlregistration.h>

class QTimer;

namespace termigator {

class SessionEnd : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    // True from the session manager's request until the session goes on after all (a shutdown
    // another program refused): cleared again after a minute.
    Q_PROPERTY(bool ending READ isEnding NOTIFY endingChanged)

public:
    explicit SessionEnd(QObject *parent = nullptr);

    bool isEnding() const { return m_ending; }

    // Test hook: what the session manager's request does.
    Q_INVOKABLE void simulate();

Q_SIGNALS:
    void endingChanged();

private:
    void begin();

    bool m_ending = false;
    QTimer *m_reset = nullptr;
};

} // namespace termigator
