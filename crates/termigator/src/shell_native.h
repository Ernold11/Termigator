#pragma once

// The few QtWidgets / QtGui calls QML has no API for. GUI thread only, after the
// QApplication exists.

#include <QtCore/QString>

namespace termigator::shell {

// Help → About Qt (QMessageBox::aboutQt, modal).
void aboutQt();

// Help → About Termigator: same layout as QMessageBox::about (app icon, rich text), no notification sound.
void aboutTermigator(const QString &title, const QString &html);

// qVersion() at run time, e.g. "6.12.0".
QString qtVersion();

// The first name of "a|b|c" that the icon theme has, else the last one.
QString themeIcon(const QString &names);

// Whether the icon theme has this icon (QIcon::hasThemeIcon).
bool hasThemeIcon(const QString &name);

// Window and taskbar icon: the theme icon "termigator" (installed build), else
// <dir>/icons/termigator-<size>.png, else <dir>/termigator-icon.png, else the theme's
// utilities-terminal.
void setAppIcon(const QString &assetsDir);

// The same look on every desktop (AppImage and other non-KDE sessions): Breeze widget style, Breeze
// colour scheme (light or dark), Breeze icons and, when bundled, Noto Sans / Hack. `mode` is
// "auto" (follows the desktop's light/dark preference; a KDE Plasma session is left alone),
// "light" or "dark". Everything is set in this process only, no environment variable, so the
// shells and ssh started from here inherit nothing. Call right after the QApplication exists.
void applyFixedLook(const QString &mode);

// The config could not be loaded: a critical box with the error and a button that opens the
// config folder, before the app quits (the files are left as they are).
void startupError(const QString &text, const QString &folder);

} // namespace termigator::shell
