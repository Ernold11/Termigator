// MultiExec hub: replays keys into the other participating sessions. See multiexec.h.

#include "termigator-term-bridge/src/multiexec.h"

#include <QtCore/QScopedValueRollback>
#include <QtGui/QKeyEvent>

#include "termigator-term-bridge/src/ghostty_session.h"

namespace termigator::term {

MultiExecHub *MultiExecHub::instance()
{
    static MultiExecHub hub;
    return &hub;
}

void MultiExecHub::setTargets(QSet<quint64> ids)
{
    if (ids == m_targets)
        return;
    m_targets = std::move(ids);
    emit targetsChanged();
}

QVariantList MultiExecHub::targetList() const
{
    QVariantList ids;
    for (quint64 id : m_targets)
        ids.append(id);
    return ids;
}

void MultiExecHub::setTargetList(const QVariantList &ids)
{
    QSet<quint64> set;
    for (const QVariant &id : ids)
        set.insert(id.toULongLong());
    setTargets(std::move(set));
}

void MultiExecHub::keyPressedGhostty(quint64 source, int key, int modifiers, const QString &text)
{
    if (m_replaying || !m_targets.contains(source))
        return;
    const QScopedValueRollback<bool> guard(m_replaying, true);
    const QSet<quint64> targets = m_targets;
    for (quint64 id : targets) {
        if (id == source)
            continue;
        if (GhosttySession *target = GhosttySession::find(id))
            target->sendKey(key, modifiers, text);
    }
}

template <typename Fn> static void replay(const QSet<quint64> &targets, quint64 source, Fn send)
{
    for (quint64 id : targets) {
        if (id == source)
            continue;
        if (GhosttySession *target = GhosttySession::find(id))
            send(target);
    }
}

void MultiExecHub::pastedGhostty(quint64 source, const QString &text, bool bracketed)
{
    if (m_replaying || !m_targets.contains(source))
        return;
    const QScopedValueRollback<bool> guard(m_replaying, true);
    replay(QSet<quint64>(m_targets), source,
           [&](GhosttySession *t) { t->pasteTextMode(text, bracketed && t->bracketedPaste()); });
}

void MultiExecHub::textCommittedGhostty(quint64 source, const QString &text)
{
    if (m_replaying || !m_targets.contains(source))
        return;
    const QScopedValueRollback<bool> guard(m_replaying, true);
    replay(QSet<quint64>(m_targets), source, [&](GhosttySession *t) { t->sendText(text); });
}

void setMultiExecTargets(rust::Slice<const std::uint64_t> ids)
{
    MultiExecHub::instance()->setTargets(QSet<quint64>(ids.begin(), ids.end()));
}

} // namespace termigator::term
