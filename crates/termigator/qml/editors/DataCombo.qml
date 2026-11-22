import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A fixed-choice combo box over [{text, value}]; setValue() ignores a value it does not offer
// and stays on the first item.
QQC2.ComboBox {
    property string tip: ""

    function setValue(value) {
        const i = indexOfValue(value);
        currentIndex = i >= 0 ? i : 0;
    }

    textRole: "text"
    valueRole: "value"
    Layout.fillWidth: true
    QQC2.ToolTip.visible: hovered && tip !== ""
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
}
