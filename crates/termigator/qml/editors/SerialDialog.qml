import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Add / Edit serial connection: device, speed and line
// settings, run by picocom / tio / screen in a tab. No Test button (nothing to probe on the
// network); a device we cannot open gets a warning under the form instead.
EditorDialog {
    id: dlg

    // EditorBackend.prepare("serial", …)
    required property var ctx

    function load() {
        const f = ctx.form;
        label.text = f.label;
        device.items = ctx.devices;
        device.setText(f.device);
        baud.items = ctx.baudRates.map(String);
        baud.setText(f.baud);
        dataBits.setValue(f.dataBits);
        parity.setValue(f.parity);
        stopBits.setValue(f.stopBits);
        flow.setValue(f.flow);
        group.items = ["", ...ctx.groups];
        group.setText(f.group);
        terminal.setValue(f.terminal);
        label.forceActiveFocus();
    }

    function collect() {
        return {
            "label": label.text,
            "device": device.text,
            "baud": baud.text,
            "dataBits": dataBits.currentValue,
            "parity": parity.currentValue,
            "stopBits": stopBits.currentValue,
            "flow": flow.currentValue,
            "group": group.text,
            "terminal": terminal.currentValue
        };
    }

    function choices(values) {
        return values.map(v => ({
                    "text": v,
                    "value": v
                }));
    }

    title: ctx.title
    sizeName: "SerialConnectionDialog"
    minWidth: 460
    Component.onCompleted: load()

    Kirigami.FormLayout {
        Layout.fillWidth: true

        EdField {
            id: label

            Kirigami.FormData.label: qsTr("Name:")
            placeholderText: qsTr("e.g. Router console")
        }
        EdCombo {
            id: device

            Kirigami.FormData.label: qsTr("Device:")
            tip: qsTr("/dev/serial/by-id/… names stay the same when the adapter is re-plugged.")
            Component.onCompleted: {
                if (contentItem && "placeholderText" in contentItem)
                    contentItem.placeholderText = "/dev/ttyUSB0";
            }
        }
        EdCombo {
            id: baud

            Kirigami.FormData.label: qsTr("Speed (baud):")
        }
        RowLayout {
            Kirigami.FormData.label: qsTr("Line:")
            Layout.fillWidth: true

            QQC2.Label {
                text: qsTr("Data")
            }
            DataCombo {
                id: dataBits

                Layout.fillWidth: false
                model: dlg.choices(["8", "7", "6", "5"])
                Accessible.name: qsTr("Data")
            }
            QQC2.Label {
                text: qsTr("Parity")
            }
            DataCombo {
                id: parity

                Layout.fillWidth: false
                model: dlg.choices(["none", "even", "odd"])
                Accessible.name: qsTr("Parity")
            }
            QQC2.Label {
                text: qsTr("Stop")
            }
            DataCombo {
                id: stopBits

                Layout.fillWidth: false
                model: dlg.choices(["1", "2"])
                Accessible.name: qsTr("Stop")
            }
            Item {
                Layout.fillWidth: true
            }
        }
        DataCombo {
            id: flow

            Kirigami.FormData.label: qsTr("Flow control:")
            model: dlg.choices(["none", "hardware", "software"])
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

    // The access problem of the typed device, if any (serial_access_problem).
    Kirigami.InlineMessage {
        Layout.fillWidth: true
        visible: text !== ""
        type: Kirigami.MessageType.Warning
        text: dlg.backend ? dlg.backend.serialProblem(device.text) : ""
    }
}
