// See session_native.h.

#include "termigator/src/session_native.h"

#include <QtCore/QTimer>
#include <QtGui/QGuiApplication>
#if QT_CONFIG(sessionmanager)
#include <QtGui/QSessionManager>
#endif

namespace termigator {

SessionEnd::SessionEnd(QObject *parent)
    : QObject(parent)
    , m_reset(new QTimer(this))
{
    m_reset->setSingleShot(true);
    m_reset->setInterval(60 * 1000);
    connect(m_reset, &QTimer::timeout, this, [this] {
        m_ending = false;
        Q_EMIT endingChanged();
    });
#if QT_CONFIG(sessionmanager)
    // Direct: the flag must be set before the close events of the same shutdown arrive.
    if (auto *app = qobject_cast<QGuiApplication *>(QCoreApplication::instance()))
        connect(app, &QGuiApplication::commitDataRequest, this, [this](QSessionManager &) { begin(); }, Qt::DirectConnection);
#endif
}

void SessionEnd::simulate()
{
    begin();
}

void SessionEnd::begin()
{
    m_reset->start();
    if (m_ending)
        return;
    m_ending = true;
    Q_EMIT endingChanged();
}

} // namespace termigator
