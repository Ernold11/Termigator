#pragma once

// MultiExec hub: a key typed in the view of one participating session is replayed into every other
// participant's engine, which encodes it with its own modes (Up is ESC [ A in the source and
// ESC O A in a target running vim with application cursor keys).
//
// Rust picks the participants (set_multiexec_targets; later core::multiexec); an empty set turns
// MultiExec off. A key from a session outside the set is not replayed.
// Reentrancy: the replay goes to Emulation::sendKeyEvent, which never re-emits termKeyPressed, and
// keys raised while a replay runs (a view's simulateKeyPress, a nested event loop) are dropped, so
// a replay never reaches its source again and never loops. GUI thread only.

#include <QtCore/QObject>
#include <QtCore/QSet>
#include <QtCore/QVariantList>
#include <cstdint>

#include "rust/cxx.h"

class QKeyEvent;

namespace termigator::term {

class MultiExecHub : public QObject
{
    Q_OBJECT
    // Participating sessionIds; for QML spikes and tests (the app sets them from Rust).
    Q_PROPERTY(QVariantList targets READ targetList WRITE setTargetList NOTIFY targetsChanged)

public:
    static MultiExecHub *instance();

    void setTargets(QSet<quint64> ids);
    QVariantList targetList() const;
    void setTargetList(const QVariantList &ids);

    // Every key a view of session `source` sends (the Ghostty view).
    void keyPressedGhostty(quint64 source, int key, int modifiers, const QString &text);
    // A paste (bracketed when `bracketed` and the target asks for it) and committed IME text from
    // session `source`: replayed like keys, so MultiExec covers everything that reaches the PTY.
    void pastedGhostty(quint64 source, const QString &text, bool bracketed);
    void textCommittedGhostty(quint64 source, const QString &text);

signals:
    void targetsChanged();

private:
    QSet<quint64> m_targets;
    bool m_replaying = false;
};

void setMultiExecTargets(rust::Slice<const std::uint64_t> ids);

} // namespace termigator::term
