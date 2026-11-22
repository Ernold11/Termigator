// Static replacement for the old QML plugin registration.

#include "termigator-term-bridge/src/register.h"

#include <QtCore/QtResource>
#include <QtQml/qqml.h>

#include "termigator-term-bridge/src/multiexec.h"
#include "termigator-term-bridge/src/ghostty_session.h"
#include "termigator-term-bridge/src/terminal_item.h"

// Q_INIT_RESOURCE must be used outside any namespace. The colour schemes live in a static library,
// so nothing would pull them into the binary without this explicit reference.
static void initTermResources()
{
    Q_INIT_RESOURCE(termigator_qrc);
}

namespace termigator::term {

void registerGhosttyTypes()
{
    initTermResources();
    // The application's QML names, on the Ghostty engine.
    const char *uri = "Termigator.Terminal";
    qmlRegisterType<GhosttyTerminalItem>(uri, 1, 0, "QMLTermWidget");
    qmlRegisterType<GhosttySession>(uri, 1, 0, "ExternalSession");
    qmlRegisterSingletonInstance(uri, 1, 0, "MultiExec", MultiExecHub::instance());
}

void registerSpikeTypes()
{
    initTermResources();
    qmlRegisterType<GhosttySession>("Termigator.Terminal.Spike", 1, 0, "GhosttySession");
    qmlRegisterType<GhosttyTerminalItem>("Termigator.Terminal.Spike", 1, 0, "GhosttySpike");
}

} // namespace termigator::term
