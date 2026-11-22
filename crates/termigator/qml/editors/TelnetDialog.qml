import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Add / Edit Telnet connection. No password field on
// purpose: Telnet is clear text, the device prompts in the tab.
EditorDialog {
    id: dlg

    // EditorBackend.prepare("telnet", …)
    required property var ctx

    function load() {
        const f = ctx.form;
        label.text = f.label;
        host.text = f.host;
        port.value = f.port;
        user.text = f.user;
        group.items = ["", ...ctx.groups];
        group.setText(f.group);
        terminal.setValue(f.terminal);
        label.forceActiveFocus();
    }

    function collect() {
        return {
            "label": label.text,
            "host": host.text,
            "port": port.value,
            "user": user.text,
            "group": group.text,
            "terminal": terminal.currentValue
        };
    }

    title: ctx.title
    sizeName: "TelnetConnectionDialog"
    testable: true
    minWidth: 440
    Component.onCompleted: load()

    Kirigami.FormLayout {
        Layout.fillWidth: true

        EdField {
            id: label

            Kirigami.FormData.label: qsTr("Name:")
            placeholderText: qsTr("e.g. Core switch")
        }
        EdField {
            id: host

            Kirigami.FormData.label: qsTr("Host:")
            placeholderText: "192.168.1.1"
        }
        PortSpin {
            id: port

            Kirigami.FormData.label: qsTr("Port:")
            value: 23
        }
        EdField {
            id: user

            Kirigami.FormData.label: qsTr("User:")
            placeholderText: qsTr("optional — most devices ask anyway")
        }
        EdCombo {
            id: group

            Kirigami.FormData.label: qsTr("Folder:")
            tip: qsTr("Folder/group for the connection list. Leave empty for top level.")
        }
        TerminalCombo {
            id: terminal

            Kirigami.FormData.label: qsTr("Terminal:")
        }
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        visible: true
        type: Kirigami.MessageType.Warning
        text: qsTr("Telnet is not encrypted — use it only on trusted networks.")
    }
}
