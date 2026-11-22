import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// Add / Edit tunnel `original` is the name of the tunnel
// being edited ("" = a new one); `tunnel` its saved fields. OK asks the backend whether the form
// may be saved and keeps the dialog open with the warning when not.
ToolDialog {
    id: dlg

    required property var backend
    property string original: ""
    property var tunnel: ({})

    function form() {
        return {
            "name": name.text,
            "via": via.currentText,
            "forwards": forwards.text,
            "autostart": autostart.checked
        };
    }

    title: original === "" ? qsTr("Add tunnel") : qsTr("Edit tunnel")
    sizeName: "TunnelEditDialog"
    minWidth: 480
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
