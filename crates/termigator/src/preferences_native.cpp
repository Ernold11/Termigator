#include "termigator/src/preferences_native.h"

#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QResource>
#include <QtGui/QFontDatabase>
#include <QtQml/QQmlEngine>

// Q_INIT_RESOURCE must be used outside any namespace. The resource (the built-in colour schemes)
// sits in the bridge's static library; initializing it more than once is harmless.
static void initTermResources()
{
    Q_INIT_RESOURCE(termigator_qrc);
}

namespace termigator::preferences {

QStringList monospaceFamilies()
{
    QStringList out;
    const QStringList all = QFontDatabase::families();
    for (const QString &family : all) {
        if (QFontDatabase::isFixedPitch(family)) {
            out.append(family);
        }
    }
    return out;
}

QStringList builtinColorSchemes()
{
    initTermResources();
    QStringList names;
    const QStringList files = QDir(QStringLiteral(":/termigator/color-schemes"))
                                  .entryList({QStringLiteral("*.colorscheme")}, QDir::Files);
    for (const QString &file : files) {
        names.append(file.chopped(QStringLiteral(".colorscheme").size()));
    }
    return names;
}

QStringList availableQuickStyles()
{
    QStringList styles;
    // Qt 6 has no QQuickStyle::availableStyles(): a style is a QtQuick/Controls/<Name> module of
    // a QML import path (Basic, Fusion, Material, ...); qqc2-desktop-style is org/kde/desktop.
    QQmlEngine engine;
    for (const QString &path : engine.importPathList()) {
        if (QFile::exists(path + QStringLiteral("/org/kde/desktop/qmldir"))
            && !styles.contains(QStringLiteral("org.kde.desktop"))) {
            styles.prepend(QStringLiteral("org.kde.desktop"));
        }
        const QStringList dirs = QDir(path + QStringLiteral("/QtQuick/Controls"))
                                     .entryList(QDir::Dirs | QDir::NoDotAndDotDot, QDir::Name);
        for (const QString &dir : dirs) {
            // Desktop styles only: Basic, Material, Universal and Imagine are touch styles with
            // their own light/dark that Kirigami does not follow; Windows draws through the native
            // QStyle (inverted tab colours, short text fields) — FluentWinUI3 is the Windows 11 look
            // (owner decisions 2026-11-21).
            static const QStringList desktop{QStringLiteral("Fusion"), QStringLiteral("FluentWinUI3")};
            const bool isStyle = desktop.contains(dir)
                && QFile::exists(path + QStringLiteral("/QtQuick/Controls/") + dir
                                 + QStringLiteral("/qmldir"));
            if (isStyle && !styles.contains(dir)) {
                styles.append(dir);
            }
        }
    }
    return styles;
}

} // namespace termigator::preferences
