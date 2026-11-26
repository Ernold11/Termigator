// Probe for headless QML checks. See probe.h.

#include "termigator-term-bridge/src/probe.h"

#include <QtCore/QEvent>
#include <QtCore/QPointF>
#include <QtGui/QWindow>
#include <QtQml/qqml.h>

#include <fstream>
#ifndef _WIN32
#include <unistd.h>
#endif

// Exported by QtGui for QtTest (qtestkeyboard.h): delivers through QWindowSystemInterface with
// synchronous delivery, so shortcuts are matched exactly as for a key from the platform.
QT_BEGIN_NAMESPACE
Q_GUI_EXPORT void qt_handleKeyEvent(QWindow *w, QEvent::Type t, int k, Qt::KeyboardModifiers mods,
                                    const QString &text = QString(), bool autorep = false,
                                    ushort count = 1);

// Exported for QTest (qtestmouse.h): same synchronous delivery as the platform plugin.
Q_GUI_EXPORT void qt_handleMouseEvent(QWindow *w, const QPointF &local, const QPointF &global,
                                      Qt::MouseButtons state, Qt::MouseButton button,
                                      QEvent::Type type, Qt::KeyboardModifiers mods,
                                      int timestamp);

Q_GUI_EXPORT void qt_handleWheelEvent(QWindow *w, const QPointF &local, const QPointF &global,
                                      QPoint pixelDelta, QPoint angleDelta,
                                      Qt::KeyboardModifiers mods, Qt::ScrollPhase phase);
QT_END_NAMESPACE

namespace termigator::term {

void Probe::key(QWindow *window, int key, int modifiers, const QString &text) const
{
    if (!window)
        return;
    const auto mods = Qt::KeyboardModifiers::fromInt(modifiers);
    qt_handleKeyEvent(window, QEvent::KeyPress, key, mods, text);
    qt_handleKeyEvent(window, QEvent::KeyRelease, key, mods, text);
}

void Probe::wheel(QWindow *window, qreal x, qreal y, int dx, int dy, int modifiers) const
{
    if (!window)
        return;
    const QPointF local(x, y);
    const QPointF global = window->mapToGlobal(local);
    qt_handleWheelEvent(window, local, global, QPoint(), QPoint(dx, dy),
                        Qt::KeyboardModifiers::fromInt(modifiers), Qt::NoScrollPhase);
}

void Probe::drag(QWindow *window, qreal x1, qreal y1, qreal x2, qreal y2, int button,
                 int modifiers) const
{
    if (!window)
        return;
    hold(window, x1, y1, x2, y2, button, modifiers);
    release(window, x2, y2, button, modifiers);
}

void Probe::hold(QWindow *window, qreal x1, qreal y1, qreal x2, qreal y2, int button,
                 int modifiers) const
{
    if (!window)
        return;
    const auto mods = Qt::KeyboardModifiers::fromInt(modifiers);
    const auto pressed = Qt::MouseButtons(Qt::MouseButton(button));
    const auto deliver = [&](qreal x, qreal y, QEvent::Type type, Qt::MouseButton which) {
        const QPointF local(x, y);
        qt_handleMouseEvent(window, local, window->mapToGlobal(local), pressed, which, type, mods,
                            0);
    };
    deliver(x1, y1, QEvent::MouseButtonPress, Qt::MouseButton(button));
    for (int i = 1; i <= 5; ++i) {
        const qreal t = i / 5.0;
        deliver(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t, QEvent::MouseMove, Qt::NoButton);
    }
}

void Probe::release(QWindow *window, qreal x, qreal y, int button, int modifiers) const
{
    if (!window)
        return;
    const QPointF local(x, y);
    qt_handleMouseEvent(window, local, window->mapToGlobal(local), Qt::NoButton,
                        Qt::MouseButton(button), QEvent::MouseButtonRelease,
                        Qt::KeyboardModifiers::fromInt(modifiers), 0);
}

void Probe::move(QWindow *window, qreal x1, qreal y1, qreal x2, qreal y2, int steps) const
{
    if (!window)
        return;
    steps = qMax(1, steps);
    for (int i = 1; i <= steps; ++i) {
        const qreal t = i / qreal(steps);
        const QPointF local(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
        qt_handleMouseEvent(window, local, window->mapToGlobal(local), Qt::NoButton, Qt::NoButton,
                            QEvent::MouseMove, Qt::NoModifier, 0);
    }
}

void Probe::click(QWindow *window, qreal x, qreal y, int button, int count, int modifiers) const
{
    if (!window)
        return;
    const auto mods = Qt::KeyboardModifiers::fromInt(modifiers);
    const auto which = Qt::MouseButton(button);
    const QPointF local(x, y);
    const QPointF global = window->mapToGlobal(local);
    for (int i = 0; i < count; ++i) {
        // Qt itself turns a second press at the same spot within the interval into a double click.
        qt_handleMouseEvent(window, local, global, which, which, QEvent::MouseButtonPress, mods,
                            1000 + i * 50);
        qt_handleMouseEvent(window, local, global, Qt::NoButton, which, QEvent::MouseButtonRelease,
                            mods, 1010 + i * 50);
    }
}

qint64 Probe::rssKiB() const
{
#ifdef _WIN32
    return -1; // test probe only; /proc does not exist on Windows
#else
    std::ifstream statm("/proc/self/statm");
    long long size = 0;
    long long resident = 0;
    if (!(statm >> size >> resident))
        return -1;
    return resident * sysconf(_SC_PAGESIZE) / 1024;
#endif
}

void registerProbeTypes()
{
    qmlRegisterSingletonType<Probe>("Termigator.Terminal.Probe", 1, 0, "Probe",
                                    [](QQmlEngine *, QJSEngine *) -> QObject * { return new Probe; });
}

} // namespace termigator::term
