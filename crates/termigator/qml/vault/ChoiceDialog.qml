import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts

// QInputDialog.getItem (not editable): the caller reads `index` after accepted().
VaultDialog {
    id: dlg

    property string label
    property var choices: []
    readonly property alias combo: choiceCombo
    readonly property int index: choiceCombo.currentIndex

    modality: Qt.ApplicationModal
    minWidth: 420
    Component.onCompleted: choiceCombo.forceActiveFocus()

    QQC2.Label {
        Layout.fillWidth: true
        text: dlg.label
        wrapMode: Text.Wrap
    }

    QQC2.ComboBox {
        id: choiceCombo

        Layout.fillWidth: true
        model: dlg.choices
        Accessible.name: dlg.label
    }
}
