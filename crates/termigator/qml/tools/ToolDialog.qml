pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import Termigator

// The frame of the tool dialogs (a QDialog with a QDialogButtonBox): content, then [OK] [Cancel]
// when `okCancel`. `validate` (returns true to go on) is asked before OK closes the dialog, so a
// bad form stays open. `warn(title, text)` is a QMessageBox.warning over it.
// The size is kept in window.dialog_sizes.<sizeName>. A hidden dialog destroys itself unless
// `keepAlive` (the Tunnels window is one object for the whole run).
Window {
    id: dlg

    required property var controller
    property string sizeName: ""
    property bool okCancel: true
    property bool keepAlive: false
    property bool modal: true
    property var validate: null
    property int minWidth: 460
    property int minHeight: 0
    default property alias content: body.data
    readonly property int margin: Kirigami.Units.largeSpacing * 2
    readonly property alias messageBox: box

    signal accepted

    function accept() {
        if (validate && !validate())
            return;
        close();
        accepted();
    }

    function reject() {
        close();
    }

    function warn(title, text) {
        box.title = title;
        box.text = text;
        box.open();
    }

    width: Math.max(minWidth, layout.implicitWidth + 2 * margin)
    height: Math.max(minHeight, layout.implicitHeight + 2 * margin)
    minimumWidth: Math.max(minWidth, layout.Layout.minimumWidth + 2 * margin)
    minimumHeight: Math.max(minHeight, layout.Layout.minimumHeight + 2 * margin)
    flags: Qt.Dialog
    modality: modal ? Qt.ApplicationModal : Qt.NonModal
    color: Kirigami.Theme.backgroundColor

    onVisibleChanged: {
        if (!visible && !keepAlive)
            Qt.callLater(dlg.destroy);
    }

    Component {
        id: sizeKeeper

        DialogSize {
            controller: dlg.controller
            window: dlg
            name: dlg.sizeName
        }
    }

    Component.onCompleted: {
        if (sizeName !== "")
            sizeKeeper.createObject(dlg);
    }

    MessageBox {
        id: box

        buttons: MessageDialog.Ok
    }

    ColumnLayout {
        id: layout

        anchors.fill: parent
        anchors.margins: dlg.margin
        spacing: Kirigami.Units.largeSpacing

        Keys.onEscapePressed: dlg.reject()

        ColumnLayout {
            id: body

            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Kirigami.Units.largeSpacing
        }

        QQC2.DialogButtonBox {
            Layout.fillWidth: true
            visible: dlg.okCancel
            standardButtons: QQC2.DialogButtonBox.Ok | QQC2.DialogButtonBox.Cancel
            onAccepted: dlg.accept()
            onRejected: dlg.reject()
        }
    }

    AppPalette {
        target: dlg
    }
}
