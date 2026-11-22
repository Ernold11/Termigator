// KDE integration: KGlobalAccel action and activation token. See kde.h.

#include "termigator-term-bridge/src/kde.h"

#include <QtCore/QtGlobal>
#include <QtCore/QCoreApplication>
#include <QtCore/QString>
#include <QtGui/QAction>
#include <QtGui/QKeySequence>

#ifdef Q_OS_LINUX
#include <KGlobalAccel>
#include <KWindowSystem>
#endif

#include "termigator-term-bridge/src/lib.rs.h"

namespace termigator::term {

bool registerToggleShortcut()
{
#ifndef Q_OS_LINUX
    // Windows: RegisterHotKey in win.cpp; until then no global shortcut.
    return false;
#else
    static QAction *action = nullptr;
    if (action)
        return true;
    auto *created = new QAction(QStringLiteral("Show / hide Termigator"), QCoreApplication::instance());
    // KGlobalAccel keys the shortcut by component and object name; they must never change, or the
    // user's assigned key is lost. The component matches the desktop file (termigator.desktop).
    created->setObjectName(QStringLiteral("toggle"));
    created->setProperty("componentName", QStringLiteral("termigator"));
    created->setProperty("componentDisplayName", QStringLiteral("Termigator"));
    QObject::connect(created, &QAction::triggered, created, [] { on_toggle_shortcut(); });
    if (!KGlobalAccel::setGlobalShortcut(created, QList<QKeySequence>{})) {
        delete created;
        return false;
    }
    action = created;
    return true;
#endif
}

void setActivationToken(rust::Str token)
{
#ifndef Q_OS_LINUX
    Q_UNUSED(token)
    return;
#else
    if (token.empty())
        return;
    KWindowSystem::setCurrentXdgActivationToken(
        QString::fromUtf8(token.data(), static_cast<qsizetype>(token.size())));
#endif
}

} // namespace termigator::term
