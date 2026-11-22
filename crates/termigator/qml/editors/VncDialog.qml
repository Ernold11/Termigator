import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Add / Edit VNC connection. No password field on
// purpose — the viewer prompts in its own window.
EditorDialog {
    id: dlg

    // EditorBackend.prepare("vnc", …)
    required property var ctx

    function load() {
        const f = ctx.form;
        label.text = f.label;
        host.text = f.host;
        port.value = f.port;
        sshGateway.reload(ctx.gateways, f.sshGateway);
        extra.text = f.extra;
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
            "sshGateway": sshGateway.currentValue,
            "extra": extra.text,
            "group": group.text,
            "terminal": terminal.currentValue
        };
    }

    title: ctx.title
    sizeName: "VncConnectionDialog"
    testable: true
    minWidth: 460
    Component.onCompleted: load()

    Kirigami.FormLayout {
        Layout.fillWidth: true

        EdField {
            id: label

            Kirigami.FormData.label: qsTr("Name:")
            placeholderText: dlg.ctx.isNew ? qsTr("e.g. Lab desktop") : ""
        }
        EdField {
            id: host

            Kirigami.FormData.label: qsTr("Host:")
            placeholderText: dlg.ctx.isNew ? "192.168.1.30" : ""
        }
        PortSpin {
            id: port

            Kirigami.FormData.label: qsTr("Port:")
            value: 5900
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: qsTr("Raw TCP port (5900 + display number).")
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
        }
        GatewayCombo {
            id: sshGateway

            Kirigami.FormData.label: qsTr("SSH gateway:")
        }
        EdField {
            id: extra

            Kirigami.FormData.label: qsTr("Extra arguments:")
            placeholderText: qsTr("e.g. -FullScreen -QualityLevel=9")
            tip: qsTr("Extra viewer arguments, space-separated.")
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
}
