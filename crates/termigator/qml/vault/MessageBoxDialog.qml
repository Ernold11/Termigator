import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts

// QMessageBox.warning / information (OK) and QMessageBox.question (Yes / No) for the vault dialogs.
// A window of its own, like the other dialogs here: the MessageDialog of QtQuick.Dialogs keeps a
// raw pointer into the window it was opened over and crashes once that window is gone. Accepted =
// OK / Yes; the caller of a question connects `accepted`.
VaultDialog {
    id: dlg

    property string text
    property bool question: false

    modality: Qt.ApplicationModal
    minWidth: 360
    standardButtons: question ? (QQC2.DialogButtonBox.Yes | QQC2.DialogButtonBox.No) : QQC2.DialogButtonBox.Ok
    onVisibleChanged: {
        if (!visible && host.messageWindow === dlg)
            host.messageWindow = null;
    }
    Component.onCompleted: buttons.forceActiveFocus()

    QQC2.Label {
        Layout.fillWidth: true
        Layout.maximumWidth: 640
        text: dlg.text
        wrapMode: Text.Wrap
    }
}
