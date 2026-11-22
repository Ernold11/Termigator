#pragma once

namespace termigator::term {

// The application's QML names (QMLTermWidget, ExternalSession, MultiExec) on the Ghostty
// engine, plus the embedded colour schemes. Call before a QML engine loads a component using them.
void registerGhosttyTypes();

// Spike types: GhosttySession and GhosttySpike (their own module).
void registerSpikeTypes();

} // namespace termigator::term
