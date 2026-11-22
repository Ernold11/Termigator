import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Generate SSH key dialog. After accepted() the caller runs `command`
// ({inner, title} from VaultBackend.keygenCommand) in a terminal tab.
VaultDialog {
    id: dlg

    property var command: null
    readonly property alias typeCombo: type
    readonly property alias pathField: path
    readonly property alias commentField: comment
    readonly property alias errorMessage: error

    modality: Qt.ApplicationModal
    minWidth: 480
    validator: () => {
        const r = JSON.parse(host.backend.keygenCommand(type.currentValue, path.text, comment.text));
        if (r.error) {
            error.text = r.error;
            error.visible = true;
            return false;
        }
        command = r;
        return true;
    }
    Component.onCompleted: {
        path.text = host.backend.freeKeyPath(type.currentValue);
        comment.text = host.backend.defaultComment();
        path.forceActiveFocus();
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true

        QQC2.ComboBox {
            id: type

            Kirigami.FormData.label: qsTr("Type:")
            Layout.fillWidth: true
            textRole: "text"
            valueRole: "value"
            model: [
                {
                    "text": qsTr("Ed25519 (recommended)"),
                    "value": "ed25519"
                },
                {
                    "text": qsTr("RSA 4096 (old servers)"),
                    "value": "rsa"
                },
                {
                    "text": qsTr("ECDSA 521"),
                    "value": "ecdsa"
                }
            ]
            onActivated: path.text = dlg.host.backend.freeKeyPath(currentValue)
        }
        QQC2.TextField {
            id: path

            Kirigami.FormData.label: qsTr("File:")
            Layout.fillWidth: true
        }
        QQC2.TextField {
            id: comment

            Kirigami.FormData.label: qsTr("Comment:")
            Layout.fillWidth: true
        }
    }

    QQC2.Label {
        Layout.fillWidth: true
        text: qsTr("ssh-keygen opens in a terminal tab and asks for the passphrase there.")
        wrapMode: Text.Wrap
        font: Kirigami.Theme.smallFont
        color: Qt.alpha(Kirigami.Theme.textColor, 0.75)
    }

    Kirigami.InlineMessage {
        id: error

        Layout.fillWidth: true
        visible: false
        type: Kirigami.MessageType.Error
    }
}
