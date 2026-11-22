import QtQuick
import Termigator

// What one Files panel keeps: the listing and its commands (FilesCore), the transfer
// queue (TransfersCore) and the timers that drive them. It belongs to the tab (or the side panel),
// not to a view: a tab that moves gets a new FilesPanel, and the running transfers stay.
// Made by FilesHost.newSession().
Item {
    id: s

    required property var host
    // The name this queue is registered under (transfers_qobj.rs `queue_scp` finds it by it).
    required property string key
    // `launch.<key>`: the side panel and the Files tabs remember "two panes" apart.
    property string twoPaneKey: "files_two_pane_side"
    readonly property alias files: filesCore
    readonly property alias transfers: transfersCore

    visible: false

    // Window or tab closing: stops the transfers and a listing in flight.
    function shutdown() {
        filesCore.shutdown();
        transfersCore.shutdown();
        host.reportTransfers(key, 0);
    }

    FilesCore {
        id: filesCore

        transfersId: s.key
        onWarning: text => s.host.warn(text)
        onPersistRequested: s.host.shell.controller.persist()
        onOpenUrl: path => Qt.openUrlExternally(s.host.fileUrl(path))
    }

    TransfersCore {
        id: transfersCore

        onJobFinished: json => filesCore.transferFinished(json)
        onActiveCountChanged: s.host.reportTransfers(s.key, transfersCore.activeCount)
    }

    // The queue's events are read, not pushed (transfers_qobj.rs): while something runs.
    Timer {
        interval: 100
        running: transfersCore.busy
        repeat: true
        onTriggered: transfersCore.poll()
    }

    // The working copies of "Edit" are looked at while there are any.
    Timer {
        interval: 500
        running: filesCore.editing
        repeat: true
        onTriggered: filesCore.pollEdits()
    }

    Component.onCompleted: transfersCore.registerQueue(key)
}
