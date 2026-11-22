import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// The transfer queue under the file list:
// File · Progress · Speed · State, Cancel / Clear finished, a context menu per job (Resume,
// Retry, Cancel, Remove from list). The queue is `transfers` (TransfersCore, transfers_qobj.rs);
// this view only mirrors its signals into `jobs`, and starts from `jobsJson()` when it is made
// again (a tab that moved gets a new view, the queue stays).
ColumnLayout {
    id: view

    required property var transfers
    readonly property alias jobs: jobs
    readonly property alias list: list
    // Ids of the selected jobs (ExtendedSelection).
    property var selection: []
    property int anchorId: -1

    // Whole view is hidden while the queue is empty.
    visible: jobs.count > 0
    spacing: 4

    function indexOfJob(id) {
        for (let i = 0; i < jobs.count; ++i)
            if (jobs.get(i).id === id)
                return i;
        return -1;
    }

    function row(json) {
        const j = JSON.parse(json);
        return {
            "id": j.id,
            "label": j.label,
            "tooltip": j.tooltip,
            "state": j.state,
            "percent": j.percent,
            "speed": j.speed,
            "stateText": j.stateText,
            "upload": j.upload
        };
    }

    function reload() {
        jobs.clear();
        for (const j of JSON.parse(transfers.jobsJson()))
            jobs.append(row(JSON.stringify(j)));
    }

    function activeCount() {
        let n = 0;
        for (let i = 0; i < jobs.count; ++i)
            if (jobs.get(i).state === "running" || jobs.get(i).state === "queued")
                ++n;
        return n;
    }

    // With nothing selected the button cancels every active job, so it asks first.
    function cancelSelected(sure) {
        const n = activeCount();
        if (selection.length === 0 && n > 1 && !sure) {
            cancelAllBox.text = qsTr("Cancel all %1 active transfers?").arg(n);
            cancelAllBox.open();
            return;
        }
        transfers.cancelIds(JSON.stringify(selection));
    }

    function select(id, mods) {
        if ((mods & Qt.ShiftModifier) && anchorId >= 0) {
            const a = indexOfJob(anchorId), b = indexOfJob(id);
            const out = [];
            for (let i = Math.min(a, b); i <= Math.max(a, b); ++i)
                out.push(jobs.get(i).id);
            selection = out;
            return;
        }
        if (mods & Qt.ControlModifier)
            selection = selection.includes(id) ? selection.filter(x => x !== id) : selection.concat([id]);
        else
            selection = [id];
        anchorId = id;
    }

    function menu(id) {
        const i = indexOfJob(id);
        if (i < 0)
            return;
        const j = jobs.get(i);
        const finished = j.state === "done" || j.state === "failed" || j.state === "cancelled";
        const interrupted = j.state === "failed" || j.state === "cancelled";
        while (contextMenu.count > 0)
            contextMenu.takeItem(0).destroy();
        const add = (text, fn) => {
            const it = itemComponent.createObject(null, {
                "text": text,
                "run": fn
            });
            contextMenu.addItem(it);
        };
        if (interrupted && !j.upload)
            add(qsTr("Resume"), () => transfers.resume(id));
        if (interrupted && j.upload)
            add(qsTr("Retry"), () => transfers.retry(id));
        if (!finished)
            add(qsTr("Cancel"), () => transfers.cancelIds(JSON.stringify([id])));
        else
            add(qsTr("Remove from list"), () => transfers.remove(id));
        contextMenu.popup();
    }

    Component.onCompleted: reload()

    ListModel {
        id: jobs
    }

    Connections {
        target: view.transfers

        function onJobAdded(json) {
            jobs.append(view.row(json));
        }

        function onJobUpdated(json) {
            const r = view.row(json);
            const i = view.indexOfJob(r.id);
            if (i >= 0)
                jobs.set(i, r);
        }

        function onJobRemoved(id) {
            const i = view.indexOfJob(id);
            if (i >= 0)
                jobs.remove(i);
            view.selection = view.selection.filter(x => x !== id);
        }
    }

    RowLayout {
        Layout.fillWidth: true

        QQC2.Label {
            objectName: "transfersTitle"
            text: {
                if (view.transfers === null)
                    return "";
                view.transfers.activeCount; // re-read the title when the count changes
                return view.transfers.title();
            }
            font.weight: Font.DemiBold
        }

        Item {
            Layout.fillWidth: true
        }

        QQC2.Button {
            objectName: "transfersCancel"
            text: view.selection.length > 0 ? qsTr("Cancel") : qsTr("Cancel all")
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: view.selection.length > 0 ? qsTr("Cancel the selected transfers") : qsTr("Cancel all active transfers")
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            onClicked: view.cancelSelected()
        }

        QQC2.Button {
            objectName: "transfersClear"
            text: qsTr("Clear finished")
            onClicked: view.transfers.clearFinished()
        }
    }

    // The header: File · Progress · Speed · State.
    RowLayout {
        Layout.fillWidth: true
        spacing: 6

        QQC2.Label {
            Layout.fillWidth: true
            text: qsTr("File")
            opacity: 0.7
        }
        QQC2.Label {
            Layout.preferredWidth: 80
            text: qsTr("Progress")
            opacity: 0.7
        }
        QQC2.Label {
            Layout.preferredWidth: 70
            text: qsTr("Speed")
            opacity: 0.7
        }
        QQC2.Label {
            Layout.preferredWidth: 110
            text: qsTr("State")
            opacity: 0.7
        }
    }

    ListView {
        id: list

        Kirigami.WheelHandler {
            target: list
        }

        objectName: "transfersList"
        Layout.fillWidth: true
        Layout.preferredHeight: Math.min(140, contentHeight)
        clip: true
        model: jobs
        boundsBehavior: Flickable.StopAtBounds
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

        delegate: Rectangle {
            id: item

            required property int index
            required property int id
            required property string label
            required property string tooltip
            required property string state
            required property int percent
            required property string speed
            required property string stateText

            readonly property bool selected: view.selection.includes(id)

            width: ListView.view.width
            height: Math.max(22, rowLayout.implicitHeight + 4)
            color: selected ? Kirigami.Theme.highlightColor : "transparent"

            RowLayout {
                id: rowLayout

                anchors.fill: parent
                anchors.leftMargin: 2
                anchors.rightMargin: 2
                spacing: 6

                QQC2.Label {
                    Layout.fillWidth: true
                    text: item.label
                    elide: Text.ElideMiddle
                    color: item.selected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
                QQC2.ProgressBar {
                    objectName: "transferBar"
                    Layout.preferredWidth: 80
                    Layout.preferredHeight: 16
                    from: 0
                    to: 100
                    value: item.percent
                }
                QQC2.Label {
                    Layout.preferredWidth: 70
                    text: item.speed
                    elide: Text.ElideRight
                    color: item.selected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
                QQC2.Label {
                    objectName: "transferState"
                    Layout.preferredWidth: 110
                    text: item.stateText
                    elide: Text.ElideRight
                    color: item.selected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                hoverEnabled: true
                onPressed: mouse => {
                    if (mouse.button === Qt.RightButton && !item.selected)
                        view.select(item.id, 0);
                    else if (mouse.button === Qt.LeftButton)
                        view.select(item.id, mouse.modifiers);
                    if (mouse.button === Qt.RightButton)
                        view.menu(item.id);
                }
                QQC2.ToolTip.visible: containsMouse
                QQC2.ToolTip.text: item.tooltip + (item.state === "failed" ? "\n" + item.stateText : "")
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            }
        }
    }

    QQC2.Menu {
        id: contextMenu
    }

    Component {
        id: itemComponent

        QQC2.MenuItem {
            property var run: null

            onTriggered: {
                if (run)
                    run();
            }
        }
    }

    MessageBox {
        id: cancelAllBox

        buttons: MessageDialog.Yes | MessageDialog.No
        onButtonClicked: button => {
            if (button === MessageDialog.Yes)
                view.cancelSelected(true);
        }
    }
}
