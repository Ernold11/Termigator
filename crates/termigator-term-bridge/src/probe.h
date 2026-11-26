#pragma once

// Probe: what headless QML checks need and QML cannot do itself. Registered only by
// registerProbeTypes() (examples and tests), never by the app.

#include <QtCore/QObject>
#include <QtCore/QString>

class QWindow;

namespace termigator::term {

class Probe : public QObject
{
    Q_OBJECT

public:
    using QObject::QObject;

    // Presses and releases `key` on `window` the way the platform plugin does, synchronously:
    // the shortcut map first (ShortcutOverride to the focus item, then Shortcuts), then the focus
    // item. `text` is what the keyboard layout would produce (Ctrl+L: "\x0c").
    Q_INVOKABLE void key(QWindow *window, int key, int modifiers, const QString &text) const;

    // Moves a wheel one notch over `window` at (x, y), the way the platform plugin does:
    // `dx` / `dy` are the angle deltas in eighths of a degree (a notch is 120), positive up.
    Q_INVOKABLE void wheel(QWindow *window, qreal x, qreal y, int dx, int dy, int modifiers) const;

    // The pointer moves from (x1, y1) to (x2, y2) in `steps` mouse moves, no button held (hover).
    Q_INVOKABLE void move(QWindow *window, qreal x1, qreal y1, qreal x2, qreal y2, int steps) const;

    // Presses `button` at (x1, y1) in `window`, drags to (x2, y2) in five moves and releases.
    // Offscreen checks use it for scrollbar thumbs and column grips.
    Q_INVOKABLE void drag(QWindow *window, qreal x1, qreal y1, qreal x2, qreal y2, int button,
                          int modifiers) const;

    // Like drag, but the button stays down at (x2, y2) (an auto-scrolling selection, say);
    // release() lets it go.
    Q_INVOKABLE void hold(QWindow *window, qreal x1, qreal y1, qreal x2, qreal y2, int button,
                          int modifiers) const;
    Q_INVOKABLE void release(QWindow *window, qreal x, qreal y, int button, int modifiers) const;

    // Clicks `button` at (x, y) in `window` `count` times in a row (2 = a double click): each
    // click is a press and a release, 50 ms apart, so Qt detects the double click itself.
    Q_INVOKABLE void click(QWindow *window, qreal x, qreal y, int button, int count,
                           int modifiers) const;

    // Resident set size of this process in KiB, from /proc/self/statm; -1 when unreadable.
    Q_INVOKABLE qint64 rssKiB() const;
};

// Registers the singleton `Probe` in the QML module Termigator.Terminal.Probe 1.0.
void registerProbeTypes();

} // namespace termigator::term
