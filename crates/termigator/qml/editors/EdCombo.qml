import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// An editable combo box (folder, resolution, device, baud rate, font): free text with
// suggestions. `text` is what is shown; setText() picks a suggestion or types the value.
QQC2.ComboBox {
    id: combo

    property var items: []
    property string tip: ""
    readonly property string text: editText

    function setText(value) {
        currentIndex = find(value);
        editText = value;
    }

    editable: true
    model: items
    currentIndex: -1
    Layout.fillWidth: true
    QQC2.ToolTip.visible: hovered && tip !== ""
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
}
