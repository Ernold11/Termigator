import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Add / Edit favorite command.
EditorDialog {
    id: dlg

    // EditorBackend.prepare("favorite", …)
    required property var ctx

    function load() {
        const f = ctx.form;
        label.text = f.label;
        command.text = f.command;
        group.items = ["", ...ctx.groups];
        group.setText(f.group);
        terminal.setValue(f.terminal);
        label.forceActiveFocus();
    }

    function collect() {
        return {
            "label": label.text,
            "command": command.text,
            "group": group.text,
            "terminal": terminal.currentValue
        };
    }

    title: ctx.title
    sizeName: "FavoriteDialog"
    minWidth: 420
    Component.onCompleted: load()

    Kirigami.FormLayout {
        Layout.fillWidth: true

        EdField {
            id: label

            Kirigami.FormData.label: qsTr("Name:")
            placeholderText: dlg.ctx.isNew ? qsTr("e.g. Local shell") : ""
        }
        EdField {
            id: command

            Kirigami.FormData.label: qsTr("Command:")
            placeholderText: dlg.ctx.isNew ? qsTr("e.g. bash") : ""
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
