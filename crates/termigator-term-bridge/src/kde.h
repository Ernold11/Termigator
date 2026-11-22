#pragma once

// KDE integration (ARCHITECTURE.md ADR-9): the global "Show / hide Termigator" shortcut through
// KGlobalAccel and the Wayland activation token for the next requestActivate(). GUI thread only,
// after the QGuiApplication exists.

#include "rust/cxx.h"

namespace termigator::term {

// Registers the action "Show / hide Termigator" with KGlobalAccel (component "termigator"), with no
// default key: the user assigns one in System Settings → Shortcuts, and KGlobalAccel loads it on
// the next start. Each press calls on_toggle_shortcut() in Rust. Repeated calls do nothing.
// Returns false when KGlobalAccel refused the action (no kglobalaccel on the session bus).
bool registerToggleShortcut();

// Hands `token` (XDG_ACTIVATION_TOKEN from `termigator --toggle`/`--show`) to the windowing system,
// so the next QWindow::requestActivate() raises the window on Wayland; on X11 it becomes the
// startup id. An empty token does nothing.
void setActivationToken(rust::Str token);

} // namespace termigator::term
