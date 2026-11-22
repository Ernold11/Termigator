import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Asks for a password (twice with `confirm`). `error` is shown
// above the field (a wrong master password, asked again). The caller reads `entered(password)`.
VaultDialog {
    id: dlg

    property string prompt
    property string error
    property bool confirm: false
    readonly property alias field: pw
    readonly property alias repeat: pw2
    readonly property alias problem: msg

    signal entered(string password)

    modality: Qt.ApplicationModal
    minWidth: 380
    validator: () => {
        if (pw.text === "")
            msg.text = qsTr("The password must not be empty.");
        else if (confirm && pw2.text !== pw.text)
            msg.text = qsTr("The passwords do not match.");
        else
            return true;
        return false;
    }
    onAccepted: entered(pw.text)
    Component.onCompleted: pw.forceActiveFocus()

    QQC2.Label {
        Layout.fillWidth: true
        visible: dlg.error !== ""
        text: dlg.error
        wrapMode: Text.Wrap
        color: Kirigami.Theme.negativeTextColor
    }

    QQC2.Label {
        Layout.fillWidth: true
        text: dlg.prompt
        wrapMode: Text.Wrap
    }

    QQC2.TextField {
        id: pw

        Layout.fillWidth: true
        echoMode: TextInput.Password
        Accessible.name: dlg.prompt.replace(/[\n:]+/g, " ").trim()
    }

    QQC2.Label {
        Layout.fillWidth: true
        visible: dlg.confirm
        text: qsTr("Repeat:")
    }

    QQC2.TextField {
        id: pw2

        Layout.fillWidth: true
        visible: dlg.confirm
        echoMode: TextInput.Password
        Accessible.name: qsTr("Repeat")
    }

    QQC2.Label {
        id: msg

        Layout.fillWidth: true
        visible: text !== ""
        wrapMode: Text.Wrap
        color: Kirigami.Theme.negativeTextColor
    }
}
