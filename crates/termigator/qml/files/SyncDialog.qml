import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// Files panel → Synchronize…: compare a local
// folder with a remote one, show the plan, copy what the user keeps ticked. Bound to the host the
// panel showed when it opened (the side panel follows the active terminal tab, and the
// plan's files must not go to whichever server was clicked meanwhile). The logic and the commands
// are `sync` (FileSyncCore, files_sync_qobj.rs); the copies go to the panel's transfer queue.
QQC2.Dialog {
    id: dlg

    required property var filesPanel
    // `FilesCore.bindingJson()`: the host this dialog works on.
    required property string binding
    property string localDir: ""
    readonly property var info: JSON.parse(binding)
    // The plan: one row per action, `checked` is the tick.
    readonly property alias plan: plan
    readonly property alias sync: syncCore
    readonly property alias localField: localPathField
    readonly property alias remoteField: remotePathField
    readonly property alias direction: directionCombo
    readonly property alias message: msg
    readonly property alias compareButton: compareButton
    readonly property alias runButton: runButton
    property bool runnable: false

    function checkedRows() {
        const out = [];
        for (let i = 0; i < plan.count; ++i)
            if (plan.get(i).checkable && plan.get(i).checked)
                out.push(i);
        return out;
    }

    function invalidate() {
        plan.clear();
        runnable = false;
        syncCore.invalidate();
    }

    function compare() {
        syncCore.compare(localPathField.text, remotePathField.text, directionCombo.currentValue);
    }

    // Copies that overwrite existing files are counted and confirmed before the run.
    function synchronize(sure) {
        const rows = checkedRows();
        if (rows.length === 0)
            return;
        const replacing = rows.filter(i => plan.get(i).replaces);
        if (replacing.length > 0 && !sure) {
            const newer = replacing.filter(i => plan.get(i).risky).length;
            confirmBox.text = qsTr("Copy %1 file(s)? %2 of them overwrite an existing file%3.").arg(rows.length).arg(replacing.length).arg(newer > 0 ? qsTr(", %1 of those a newer one").arg(newer) : "");
            confirmBox.open();
            return;
        }
        syncCore.run(JSON.stringify(rows), localPathField.text, remotePathField.text);
    }

    MessageBox {
        id: confirmBox

        buttons: MessageDialog.Yes | MessageDialog.No
        onButtonClicked: button => {
            if (button === MessageDialog.Yes)
                dlg.synchronize(true);
        }
    }

    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    modal: false
    closePolicy: QQC2.Popup.CloseOnEscape
    title: qsTr("Synchronize — %1").arg(info.label)
    width: Math.min(760, parent ? parent.width - 40 : 760)
    height: Math.min(520, parent ? parent.height - 40 : 520)
    standardButtons: QQC2.Dialog.Close
    onClosed: {
        syncCore.shutdown();
        destroy();
    }

    FileSyncCore {
        id: syncCore

        onMessage: (level, text) => {
            msg.text = text;
            msg.type = level === "error" ? Kirigami.MessageType.Error : (level === "positive" ? Kirigami.MessageType.Positive : (level === "warning" ? Kirigami.MessageType.Warning : Kirigami.MessageType.Information));
        }
        onPlanReady: json => {
            plan.clear();
            for (const r of JSON.parse(json).rows)
                plan.append({
                    "opLabel": r.opLabel,
                    "rel": r.rel,
                    "size": r.size,
                    "reason": r.reason,
                    "checkable": r.checkable,
                    "checked": r.checked,
                    "risky": r.risky,
                    "replaces": r.replaces
                });
            dlg.runnable = JSON.parse(json).rows.some(r => r.checkable);
        }
        onQueued: {
            plan.clear();
            dlg.runnable = false;
        }
    }

    ListModel {
        id: plan
    }

    Component.onCompleted: {
        syncCore.bind(binding, filesPanel.files.transfersId);
        localPathField.text = localDir !== "" ? localDir : helper.homeDir();
        remotePathField.text = info.cwd;
    }

    FilesHelper {
        id: helper
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 6

        GridLayout {
            Layout.fillWidth: true
            columns: 2

            QQC2.Label {
                text: qsTr("Local folder:")
            }
            RowLayout {
                Layout.fillWidth: true

                QQC2.TextField {
                    id: localPathField

                    objectName: "syncLocal"
                    Layout.fillWidth: true
                    onTextChanged: dlg.invalidate()
                }
                QQC2.Button {
                    text: qsTr("Browse…")
                    onClicked: {
                        browse.currentFolder = dlg.filesPanel.pathToUrl(localPathField.text || helper.homeDir());
                        browse.open();
                    }
                }
            }

            QQC2.Label {
                text: qsTr("Remote folder:")
            }
            QQC2.TextField {
                id: remotePathField

                objectName: "syncRemote"
                Layout.fillWidth: true
                onTextChanged: dlg.invalidate()
            }

            QQC2.Label {
                text: qsTr("Direction:")
            }
            QQC2.ComboBox {
                id: directionCombo

                objectName: "syncDirection"
                Layout.fillWidth: true
                textRole: "text"
                valueRole: "value"
                model: [
                    {
                        "text": qsTr("Local → server (upload what is new here)"),
                        "value": "upload"
                    },
                    {
                        "text": qsTr("Server → local (download what is new there)"),
                        "value": "download"
                    },
                    {
                        "text": qsTr("Both ways (the newer file wins)"),
                        "value": "both"
                    }
                ]
                onActivated: dlg.invalidate()
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            opacity: 0.7
            text: qsTr("Compared by size and modification time, recursively. Nothing is ever deleted; copies keep their modification time.")
        }

        Kirigami.InlineMessage {
            id: msg

            objectName: "syncMessage"
            Layout.fillWidth: true
            visible: text !== ""
        }

        // Action · Path · Size · Why
        RowLayout {
            Layout.fillWidth: true
            spacing: 6

            QQC2.Label {
                Layout.preferredWidth: 140
                text: qsTr("Action")
                font.weight: Font.DemiBold
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: qsTr("Path")
                font.weight: Font.DemiBold
            }
            QQC2.Label {
                Layout.preferredWidth: 70
                text: qsTr("Size")
                font.weight: Font.DemiBold
            }
            QQC2.Label {
                Layout.preferredWidth: 180
                text: qsTr("Why")
                font.weight: Font.DemiBold
            }
        }

        ListView {
            id: table

            Kirigami.WheelHandler {
                target: table
            }

            objectName: "syncTable"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: plan
            QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

            delegate: RowLayout {
                id: row

                required property int index
                required property string opLabel
                required property string rel
                required property string size
                required property string reason
                required property bool checkable
                required property bool checked
                required property bool risky

                width: ListView.view.width
                spacing: 6

                QQC2.CheckBox {
                    objectName: "syncCheck" + row.index
                    Layout.preferredWidth: 140
                    enabled: row.checkable
                    checked: row.checked
                    text: row.opLabel
                    onToggled: row.ListView.view.model.setProperty(row.index, "checked", checked)
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: row.rel
                    elide: Text.ElideMiddle
                    QQC2.ToolTip.visible: hover.hovered
                    QQC2.ToolTip.text: row.rel
                    HoverHandler {
                        id: hover
                    }
                }
                QQC2.Label {
                    Layout.preferredWidth: 70
                    text: row.size
                }
                QQC2.Label {
                    Layout.preferredWidth: 180
                    text: row.reason
                    elide: Text.ElideRight
                    color: row.risky ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                    QQC2.ToolTip.visible: reasonHover.hovered && truncated
                    QQC2.ToolTip.text: row.reason
                    HoverHandler {
                        id: reasonHover
                    }
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true

            QQC2.Button {
                id: compareButton

                objectName: "syncCompare"
                text: qsTr("Compare")
                enabled: !syncCore.busy
                onClicked: dlg.compare()
            }
            Item {
                Layout.fillWidth: true
            }
            QQC2.Button {
                id: runButton

                objectName: "syncRun"
                text: qsTr("Synchronize")
                enabled: dlg.runnable
                onClicked: dlg.synchronize()
            }
        }
    }

    FolderDialog {
        id: browse

        title: qsTr("Local folder")
        onAccepted: localPathField.text = dlg.filesPanel.urlToPath(selectedFolder).replace(/\/+$/, "")
    }
}
