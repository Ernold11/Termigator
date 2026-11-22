// See shell_native.h.

#include "termigator/src/shell_native.h"

#include <QtCore/QFileInfo>
#include <QtCore/QSize>
#include <QtCore/QStringList>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtCore/QCoreApplication>
#include <QtCore/QDir>
#include <QtCore/QResource>
#include <QtQuickControls2/QQuickStyle>
#include <QtCore/QStandardPaths>
#include <QtGui/QFont>
#include <QtGui/QFontDatabase>
#include <QtGui/QStyleHints>
#include <QtWidgets/QApplication>
#include <QtGui/QDesktopServices>
#include <QtCore/QUrl>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QStyle>
#include <QtWidgets/QStyleFactory>

#if __has_include(<BreezeIcons/BreezeIcons>)
#include <BreezeIcons/BreezeIcons>
#define TERMIGATOR_HAVE_BREEZE_ICONS 1
#elif defined(TERMIGATOR_BREEZE_ICONS_RUNTIME)
// Only the runtime library is installed (no -dev headers). The icons are compiled into it and
// build.rs links that library, so declaring its one exported entry point registers the theme in
// this build too: icons do not depend on a system icon theme being installed.
namespace BreezeIcons
{
void initIcons();
}
#define TERMIGATOR_HAVE_BREEZE_ICONS 1
#else
#define TERMIGATOR_HAVE_BREEZE_ICONS 0
#endif
#if __has_include(<KColorScheme/KColorScheme>) && __has_include(<KConfigCore/KSharedConfig>)
#include <KColorScheme/KColorScheme>
#include <KConfigCore/KSharedConfig>
#define TERMIGATOR_HAVE_KCOLORSCHEME 1
#else
#define TERMIGATOR_HAVE_KCOLORSCHEME 0
#endif
#if __has_include(<KIconThemes/KIconTheme>)
#include <KIconThemes/KIconTheme>
#define TERMIGATOR_HAVE_KICONTHEMES 1
#else
#define TERMIGATOR_HAVE_KICONTHEMES 0
#endif

namespace termigator::shell {

void aboutQt()
{
    QMessageBox::aboutQt(nullptr);
}

void startupError(const QString &text, const QString &folder)
{
    QMessageBox box(QMessageBox::Critical, QStringLiteral("Termigator"),
                    QStringLiteral("Termigator cannot start: the configuration could not be read."),
                    QMessageBox::Close, nullptr);
    box.setInformativeText(text + QStringLiteral("\n\nFix or move the file, then start Termigator again. "
                                                 "Nothing was changed."));
    QPushButton *open = box.addButton(QStringLiteral("Open folder"), QMessageBox::ActionRole);
    box.exec();
    if (box.clickedButton() == open)
        QDesktopServices::openUrl(QUrl::fromLocalFile(folder));
}

void aboutTermigator(const QString &title, const QString &html)
{
    QMessageBox box(QMessageBox::NoIcon, title, QString(), QMessageBox::Ok, nullptr);
    box.setTextFormat(Qt::RichText);
    box.setText(html);
    const QIcon icon = QGuiApplication::windowIcon();
    if (!icon.isNull()) {
        const int px = box.style()->pixelMetric(QStyle::PM_LargeIconSize, nullptr, &box);
        box.setIconPixmap(icon.pixmap(px, px));
    }
    box.exec();
}

QString qtVersion()
{
    return QString::fromLatin1(qVersion());
}

static QString bundledShare();

// "breeze-dark" when `dark` and that theme exists, else "breeze": KDE Craft (Windows) compiles only
// the light Breeze into KF6BreezeIcons, and a missing theme leaves every themed icon blank (tool
// buttons then fall back to their text). Monochrome Breeze icons are recoloured to the palette,
// but only by org.kde.desktop: other styles (Fusion, FluentWinUI3) drew them dark on dark, so the
// Windows installer carries the dark set as share/icons/breeze-dark.rcc (scripts/windows-share.sh).
static QString breezeThemeFor(bool dark)
{
    static const bool registered = [] {
        const QString share = bundledShare();
        return !share.isEmpty()
            && QResource::registerResource(share + QStringLiteral("/icons/breeze-dark.rcc")); // :/icons/breeze-dark
    }();
    Q_UNUSED(registered);
    if (dark) {
        for (const QString &dir : QIcon::themeSearchPaths()) {
            if (QFileInfo::exists(dir + QStringLiteral("/breeze-dark/index.theme")))
                return QStringLiteral("breeze-dark");
        }
    }
    return QStringLiteral("breeze");
}

// Another desktop's icon theme (Adwaita, Mint-Y) lacks icons like view-split-left-right: Breeze,
// which the AppImage carries and many desktops have, fills the gaps. Before the first icon lookup
// (the action list is built before the window icon is set).
static void ensureIconTheme()
{
    static bool done = false;
    if (done)
        return;
    done = true;
    QIcon::setFallbackThemeName(QStringLiteral("breeze"));
#if defined(Q_OS_WIN) && TERMIGATOR_HAVE_BREEZE_ICONS
    // Windows has no icon theme of its own: Breeze, compiled into KF6BreezeIcons, is the theme.
    BreezeIcons::initIcons();
    QIcon::setThemeName(breezeThemeFor(QGuiApplication::styleHints()->colorScheme() == Qt::ColorScheme::Dark));
#endif
}

QString themeIcon(const QString &names)
{
    ensureIconTheme();
    // Styles other than org.kde.desktop (FluentWinUI3) tint every icon one colour, which
    // turns Breeze's coloured app / place icons into filled squares: their monochrome -symbolic
    // variants first.
    static const bool symbolic = QQuickStyle::name() != QLatin1String("org.kde.desktop");
    const QStringList parts = names.split(QLatin1Char('|'), Qt::SkipEmptyParts);
    for (const QString &name : parts) {
        if (symbolic && QIcon::hasThemeIcon(name + QStringLiteral("-symbolic")))
            return name + QStringLiteral("-symbolic");
        if (QIcon::hasThemeIcon(name))
            return name;
    }
    return parts.isEmpty() ? QString() : parts.last();
}

bool hasThemeIcon(const QString &name)
{
    ensureIconTheme();
    return QIcon::hasThemeIcon(name);
}

void setAppIcon(const QString &assetsDir)
{
    ensureIconTheme();
    // Installed build: the hicolor theme icon "termigator" (install.sh); the build tree's assets
    // directory is only the development fallback.
    if (QIcon::hasThemeIcon(QStringLiteral("termigator"))) {
        QGuiApplication::setWindowIcon(QIcon::fromTheme(QStringLiteral("termigator")));
        return;
    }
    static const int sizes[] = {16, 24, 32, 48, 64, 128, 256, 512};
    QIcon icon;
    for (int s : sizes) {
        const QString path = assetsDir + QStringLiteral("/icons/termigator-%1.png").arg(s);
        if (QFileInfo(path).isFile())
            icon.addFile(path, QSize(s, s));
    }
    const QString single = assetsDir + QStringLiteral("/termigator-icon.png");
    if (icon.isNull() && QFileInfo(single).isFile())
        icon = QIcon(single);
    if (icon.isNull())
        icon = QIcon::fromTheme(QStringLiteral("utilities-terminal"));
    if (!icon.isNull())
        QGuiApplication::setWindowIcon(icon);
}

// The AppImage keeps its data next to the loader: <app dir>/../share (qt.conf sits there too).
static QString bundledShare()
{
    const QDir share(QCoreApplication::applicationDirPath() + QStringLiteral("/../share"));
    return share.exists(QStringLiteral("color-schemes")) ? share.absolutePath() : QString();
}

void applyFixedLook(const QString &mode)
{
    // FluentWinUI3 (and Fusion on Windows) take light or dark from the colour scheme, not from the
    // palette; Plasma's platform theme ignores the request, AppPalette covers the palette there.
    QGuiApplication::styleHints()->setColorScheme(mode == QLatin1String("dark") ? Qt::ColorScheme::Dark
                                                  : mode == QLatin1String("light") ? Qt::ColorScheme::Light
                                                                                   : Qt::ColorScheme::Unknown);
    const bool plasma = QApplication::style()->name().compare(QStringLiteral("breeze"), Qt::CaseInsensitive) == 0;
    if (mode == QLatin1String("auto") && plasma)
        return;
    const bool dark = mode == QLatin1String("dark")
        || (mode != QLatin1String("light") && QGuiApplication::styleHints()->colorScheme() == Qt::ColorScheme::Dark);

    if (QStyleFactory::keys().contains(QStringLiteral("Breeze"), Qt::CaseInsensitive))
        QApplication::setStyle(QStringLiteral("Breeze"));

    const QString file = QStringLiteral("color-schemes/%1.colors").arg(dark ? QStringLiteral("BreezeDark") : QStringLiteral("BreezeLight"));
    QString scheme;
    if (const QString share = bundledShare(); !share.isEmpty())
        scheme = share + QLatin1Char('/') + file;
    else
        scheme = QStandardPaths::locate(QStandardPaths::GenericDataLocation, file);
    if (!scheme.isEmpty()) {
        // Kirigami.Theme and the Qt palette read the same file; kdeglobals is ignored.
        qApp->setProperty("KDE_COLOR_SCHEME_PATH", scheme);
#if TERMIGATOR_HAVE_KCOLORSCHEME
        qApp->setPalette(KColorScheme::createApplicationPalette(KSharedConfig::openConfig(scheme)));
#endif
    }

#if TERMIGATOR_HAVE_BREEZE_ICONS
    BreezeIcons::initIcons(); // the theme compiled into libKF6BreezeIcons
#endif
    const QString icons = breezeThemeFor(dark);
    QIcon::setThemeName(icons);
#if TERMIGATOR_HAVE_KICONTHEMES
    KIconTheme::forceThemeForTests(icons); // KIconLoader would read [Icons] from kdeglobals
#endif

    if (const QString share = bundledShare(); !share.isEmpty()) {
        for (const QString &f : {QStringLiteral("NotoSans-Regular.ttf"), QStringLiteral("NotoSans-Bold.ttf"),
                                 QStringLiteral("Hack-Regular.ttf"), QStringLiteral("Hack-Bold.ttf")})
            QFontDatabase::addApplicationFont(share + QStringLiteral("/fonts/") + f);
        if (QFontDatabase::families().contains(QStringLiteral("Noto Sans")))
            QApplication::setFont(QFont(QStringLiteral("Noto Sans"), 10));
    }
}

} // namespace termigator::shell
