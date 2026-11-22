#pragma once

// The two lookups the settings dialog needs from Qt: QML has no list of fixed-pitch font
// families, and the compiled-in colour schemes live in the terminal bridge's resource file.
// GUI thread only, after the QGuiApplication exists.

#include <QtCore/QStringList>

namespace termigator::preferences {

// Installed font families that are fixed-pitch (as a QFontComboBox with MonospacedFonts lists them).
QStringList monospaceFamilies();

// File names without ".colorscheme" of the schemes compiled into the terminal bridge.
QStringList builtinColorSchemes();

// Qt Quick Controls styles that can be loaded: org.kde.desktop (when qqc2-desktop-style is
// installed) first, then QQuickStyle::availableStyles(). Needs the QGuiApplication.
QStringList availableQuickStyles();

} // namespace termigator::preferences
