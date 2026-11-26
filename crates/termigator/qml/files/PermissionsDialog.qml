pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts

// rwx check boxes for owner / group / others, kept in step with an octal field.
// `ask(what, mode, anyDir, done)`; `done(mode, recursive)`
// runs after OK. setuid / setgid / sticky are kept as they were.
QQC2.Dialog {
    id: dlg

    property string what: ""
    property bool anyDir: false
    property var done: null
    // The bits above 0o777 of the mode the dialog opened with.
    property int special: 0
    property bool syncing: false
    readonly property alias octal: octalField
    readonly property alias recursive: recursiveBox
    readonly property alias boxes: boxes

    // The octal field's value, -1 when it is not a number.
    function mode() {
        const t = octalField.text.trim();
        return /^[0-7]+$/.test(t) ? parseInt(t, 8) : -1;
    }

    function recursiveWanted() {
        return recursiveBox.visible && recursiveBox.checked;
    }

    function ask(what, mode, anyDir, done) {
        dlg.what = what;
        dlg.anyDir = anyDir;
        dlg.done = done;
        dlg.special = mode & 3584;
        syncing = true;
        for (let i = 0; i < 9; ++i)
            boxes.itemAt(i).checked = (mode & (1 << (8 - i))) !== 0;
        octalField.text = (mode & 4095).toString(8).padStart(4, "0");
        recursiveBox.checked = false;
        syncing = false;
        open();
    }

    function fromBoxes() {
        if (syncing)
            return;
        let bits = 0;
        for (let i = 0; i < 9; ++i)
            if (boxes.itemAt(i).checked)
                bits |= 1 << (8 - i);
        octalField.text = (special | bits).toString(8).padStart(4, "0");
    }

    function fromOctal() {
        const v = mode();
        if (v < 0 || v > 4095)
            return;
        special = v & 3584;
        syncing = true;
        for (let i = 0; i < 9; ++i)
            boxes.itemAt(i).checked = (v & (1 << (8 - i))) !== 0;
        syncing = false;
    }

    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    modal: true
    title: qsTr("Permissions")
    standardButtons: QQC2.Dialog.Ok | QQC2.Dialog.Cancel
    onAccepted: {
        const fn = done;
        done = null;
        if (fn)
            fn(mode(), recursiveWanted());
    }
    onRejected: done = null

    ColumnLayout {
        anchors.fill: parent
        spacing: 6

        QQC2.Label {
            text: qsTr("Permissions of %1").arg(dlg.what)
        }

        GridLayout {
            columns: 4
            rowSpacing: 2

            Item {}
            Repeater {
                model: [qsTr("Read"), qsTr("Write"), qsTr("Execute")]

                QQC2.Label {
                    required property string modelData

                    text: modelData
                }
            }

            Repeater {
                id: boxes

                model: 9

                // Row-major: owner r w x, group r w x, others r w x.
                QQC2.CheckBox {
                    required property int index

                    objectName: "perm" + index
                    Layout.row: Math.floor(index / 3) + 1
                    Layout.column: (index % 3) + 1
                    onCheckedChanged: dlg.fromBoxes()
                }
            }

            Repeater {
                model: [qsTr("Owner"), qsTr("Group"), qsTr("Others")]

                QQC2.Label {
                    required property int index
                    required property string modelData

                    Layout.row: index + 1
                    Layout.column: 0
                    text: modelData
                }
            }
        }

        RowLayout {
            QQC2.Label {
                text: qsTr("Octal:")
            }
            QQC2.TextField {
                id: octalField

                objectName: "permOctal"
                Layout.preferredWidth: 80
                onTextEdited: dlg.fromOctal()
            }
        }

        QQC2.CheckBox {
            id: recursiveBox

            objectName: "permRecursive"
            visible: dlg.anyDir
            text: qsTr("Apply recursively to folder contents")
        }
    }
}
