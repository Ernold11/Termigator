import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts

// Delete a folder that is not empty: where its entries and subfolders go. `ask()` opens it;
// `done(mode, dest)` runs after OK with mode "parent", "move" (into `dest`) or "delete".
QQC2.Dialog {
    id: dlg

    property string path
    property string question
    property string parentPath
    property var done: null
    readonly property alias toParent: parentChoice
    readonly property alias toOther: otherChoice
    readonly property alias deleteAll: deleteChoice
    readonly property alias target: targetInput

    function ask(path, question, parentPath, targets, done) {
        dlg.path = path;
        dlg.question = question;
        dlg.parentPath = parentPath;
        dlg.done = done;
        // The parent has its own choice.
        const others = targets.filter(t => t !== parentPath);
        targetInput.model = (parentPath === "" ? [] : ["(top level)"]).concat(others);
        targetInput.currentIndex = 0;
        otherChoice.enabled = targetInput.count > 0;
        parentChoice.checked = true;
        open();
    }

    function mode() {
        return parentChoice.checked ? "parent" : otherChoice.checked ? "move" : "delete";
    }

    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    modal: true
    title: qsTr("Delete folder")
    standardButtons: QQC2.Dialog.Ok | QQC2.Dialog.Cancel
    width: Math.min(Math.max(implicitWidth, 420), parent ? parent.width - 40 : 420)

    onOpened: parentChoice.forceActiveFocus()
    onAccepted: {
        const fn = done;
        done = null;
        if (fn)
            fn(mode(), otherChoice.checked ? targetInput.currentText : parentPath);
    }
    onRejected: done = null

    // The choices sit in different layouts, so they need a group to be exclusive.
    QQC2.ButtonGroup {
        id: choices
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 6

        QQC2.Label {
            Layout.fillWidth: true
            text: dlg.question
            wrapMode: Text.Wrap
        }

        QQC2.RadioButton {
            id: parentChoice

            QQC2.ButtonGroup.group: choices
            text: dlg.parentPath === "" ? qsTr("Move them to the top level") : qsTr("Move them to the parent folder “%1”").arg(dlg.parentPath)
        }

        RowLayout {
            Layout.fillWidth: true

            QQC2.RadioButton {
                id: otherChoice

                QQC2.ButtonGroup.group: choices
                text: qsTr("Move them to:")
            }

            QQC2.ComboBox {
                id: targetInput

                Layout.fillWidth: true
                enabled: otherChoice.enabled
                Accessible.name: qsTr("Destination folder")
                onActivated: otherChoice.checked = true
            }
        }

        QQC2.RadioButton {
            id: deleteChoice

            QQC2.ButtonGroup.group: choices
            text: qsTr("Delete them too (cannot be undone)")
        }
    }
}
