// See-through terminal backgrounds: alpha windows and blur. See window_effects.h.

#include "termigator-term-bridge/src/window_effects.h"

#include <QtCore/QtGlobal>
#include <QtGui/QWindow>
#include <QtQuick/QQuickWindow>

#ifdef Q_OS_LINUX
#include <KWindowEffects>
#endif

namespace termigator::term {

void enableAlphaWindows()
{
    QQuickWindow::setDefaultAlphaBuffer(true);
}

void setBlurBehindWindow(QWindow *window, bool enabled)
{
#ifdef Q_OS_LINUX
    if (window != nullptr)
        KWindowEffects::enableBlurBehind(window, enabled);
#else
    Q_UNUSED(window);
    Q_UNUSED(enabled);
#endif
}

} // namespace termigator::term
