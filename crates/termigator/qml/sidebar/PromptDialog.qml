import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts

// QInputDialog.getText / getItem (editable) for the host list: open with `ask()`; `done(text)`
// runs after OK (or Enter in the field).
QQC2.Dialog {
    id: dlg

    property string label
    // null: a text field; an array: an editable combo box with these choices.
    property var choices: null
    property var done: null
    // A password: the field shows dots and is emptied afterwards.
    property bool secret: false
    readonly property alias field: fieldInput
    readonly property alias combo: comboInput

    function ask(title, label, text, choices, current, done, secret) {
        dlg.secret = secret === true;
        dlg.title = title;
        dlg.label = label;
        dlg.choices = choices;
        dlg.done = done;
        fieldInput.text = text;
        if (choices !== null) {
            comboInput.model = choices;
            comboInput.currentIndex = current;
            comboInput.editText = choices[current] ?? "";
        }
        open();
    }

    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    modal: true
    standardButtons: QQC2.Dialog.Ok | QQC2.Dialog.Cancel
    width: Math.min(Math.max(implicitWidth, 360), parent ? parent.width - 40 : 360)

    onOpened: {
        const input = choices === null ? fieldInput : comboInput;
        input.forceActiveFocus();
        if (choices === null)
            fieldInput.selectAll();
        else
            comboInput.contentItem.selectAll();
    }
    onAccepted: {
        const fn = done;
        done = null;
        const value = choices === null ? fieldInput.text : comboInput.editText;
        if (secret)
            fieldInput.text = "";
        if (fn)
            fn(value);
    }
    onRejected: done = null

    ColumnLayout {
        anchors.fill: parent
        spacing: 6

        QQC2.Label {
            Layout.fillWidth: true
            text: dlg.label
            wrapMode: Text.Wrap
        }

        QQC2.TextField {
            id: fieldInput

            Layout.fillWidth: true
            visible: dlg.choices === null
            echoMode: dlg.secret ? TextInput.Password : TextInput.Normal
            Accessible.name: dlg.label
            onAccepted: dlg.accept()
        }

        QQC2.ComboBox {
            id: comboInput

            Layout.fillWidth: true
            visible: dlg.choices !== null
            editable: true
            Accessible.name: dlg.label
            onAccepted: dlg.accept()
        }
    }
}
