#pragma once

// The two lookups the terminal tab needs from Qt and that QML / Rust do not have.
// GUI thread only, after the QGuiApplication exists.

#include <QtCore/QString>
#include <QtCore/QStringList>
#include <cstdint>

namespace termigator::terminal {

// The local wall-clock time as the number YYYYMMDDHHMMSS for log file
// names and line stamps.
std::int64_t localStamp();

// Every installed font family (`QFontDatabase::families()`), fixed-pitch or not.
QStringList installedFamilies();

} // namespace termigator::terminal
