#include "termigator/src/terminal_native.h"

#include <QtCore/QDateTime>
#include <QtGui/QFontDatabase>

namespace termigator::terminal {

std::int64_t localStamp()
{
    const QDateTime now = QDateTime::currentDateTime();
    const QDate d = now.date();
    const QTime t = now.time();
    return std::int64_t(d.year()) * 10000000000LL + std::int64_t(d.month()) * 100000000LL + std::int64_t(d.day()) * 1000000LL
        + std::int64_t(t.hour()) * 10000LL + std::int64_t(t.minute()) * 100LL + std::int64_t(t.second());
}

QStringList installedFamilies()
{
    return QFontDatabase::families();
}

} // namespace termigator::terminal
