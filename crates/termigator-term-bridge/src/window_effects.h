#pragma once

// See-through terminal backgrounds (termigator_core::terminal_background): windows with an alpha
// channel and KWin's blur behind them. GUI thread only.

class QWindow;

namespace termigator::term {

// Before any window exists: every Qt Quick window gets an alpha channel, so a window whose colour
// is transparent shows what is behind it. Opaque windows look the same as without it.
void enableAlphaWindows();

// KWin blurs what shows through `window` (KWindowEffects); nothing off Linux / without KWin.
void setBlurBehindWindow(QWindow *window, bool enabled);

} // namespace termigator::term
