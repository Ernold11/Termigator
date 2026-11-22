import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// File → Quick connect… an ad-hoc SSH target that is not
// saved. The host reads `target` and `terminal` after accepted().
EditorDialog {
    id: dlg

    // The config's default terminal ("embedded" / "external").
    property string defaultTerminal: "embedded"
    readonly property alias target: targetField.text
    readonly property string terminal: terminalCombo.currentValue

    function collect() {
        return {
            "target": target,
            "terminal": terminal
        };
    }

    title: qsTr("Quick SSH")
    minWidth: 420
    Component.onCompleted: {
        terminalCombo.setValue(defaultTerminal);
        targetField.forceActiveFocus();
    }

    // Labels above the fields, in a stacked layout.
    Kirigami.FormLayout {
        Layout.fillWidth: true
        wideMode: false

        EdField {
            id: targetField

            Kirigami.FormData.label: qsTr("Target (user@host, user@host:port or user@[ipv6]:port):")
            placeholderText: qsTr("e.g. ubuntu@10.0.0.5:2222 or root@[2001:db8::1]:2222")
        }
        TerminalCombo {
            id: terminalCombo

            Kirigami.FormData.label: qsTr("Terminal:")
        }
    }
}
