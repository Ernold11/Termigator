#pragma once

// The terminal bell's sound for terminal_qobj.rs (defined in message_native.cpp, next to the
// message box sounds): Plasma's "beep" event (KNotifications), Windows' default beep, else
// QApplication::beep(). GUI thread only. No Qt types here: the cxx bridge includes this header.

namespace termigator {

void playBell();

} // namespace termigator
