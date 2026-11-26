import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// Add / Edit tunnel `original` is the name of the tunnel
// being edited ("" = a new one); `tunnel` its saved fields. OK asks the backend whether the form
// may be saved and keeps the dialog open with the warning when not.
// `keys` is the VaultHost (null without one), for the key field (KeyPathField): a tunnel has no
// terminal to ask for a passphrase, so such a key has to be in the agent.
ToolDialog {
    id: dlg

    required property var backend
    property var keys: null
    property string original: ""
    property var tunnel: ({})
    readonly property alias identityField: identity.field
    readonly property bool keyLocked: identity.locked

    function form() {
        return {
            "name": name.text,
            "via": via.currentText,
            "forwards": forwards.text,
            "autostart": autostart.checked,
            "identityFile": identity.text.trim()
        };
    }

    title: original === "" ? qsTr("Add tunnel") : qsTr("Edit tunnel")
    sizeName: "TunnelEditDialog"
    minWidth: 520
    validate: () => {
        const err = backend.tunnelError(original, JSON.stringify(form()));
        if (err !== "")
            warn(qsTr("Tunnel"), err);
        return err === "";
    }

    Component.onCompleted: {
        const labels = JSON.parse(backend.sshLabelsJson());
        const t = tunnel;
        name.text = t.name || "";
        if (t.via && labels.indexOf(t.via) < 0)
            labels.push(t.via);
        via.model = labels;
        via.currentIndex = t.via ? labels.indexOf(t.via) : (labels.length > 0 ? 0 : -1);
        forwards.text = (t.forwards || []).join("\n");
        autostart.checked = t.autostart === true;
        identity.text = t.identityFile || "";
        name.forceActiveFocus();
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true

        EdField {
            id: name

            Kirigami.FormData.label: qsTr("Name:")
            placeholderText: qsTr("e.g. prod DB")
        }
        EdCombo {
            id: via

            Kirigami.FormData.label: qsTr("Through SSH entry:")
            editable: false
            tip: qsTr("The saved SSH entry the tunnel runs through (its login, key and jump host).")
        }
        KeyPathField {
            id: identity

            Kirigami.FormData.label: qsTr("SSH key:")
            Kirigami.FormData.labelAlignment: Qt.AlignTop
            backend: dlg.backend
            keys: dlg.keys
            placeholderText: qsTr("optional — empty: the key of the SSH entry")
            tip: qsTr("A private key for this tunnel only, used instead of the SSH entry's key. Leave empty to log in like the SSH entry (its key, or ssh's default keys).")
            passphraseText: qsTr("This key has a passphrase. A tunnel runs without a terminal to ask for it, so add the key to the SSH agent before starting the tunnel.")
        }
        EdArea {
            id: forwards

            Kirigami.FormData.label: qsTr("Forwards:")
            lines: 5
            placeholderText: qsTr("One per line:\nL 15432:db.internal:5432   (local port → remote host)\nR 9000:localhost:22        (remote port → this machine)\nD 1080                     (SOCKS proxy)")
        }
        EdCheck {
            id: autostart

            text: qsTr("Start with Termigator")
        }
    }
}
