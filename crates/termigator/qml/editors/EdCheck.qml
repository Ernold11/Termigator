import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

QQC2.CheckBox {
    property string tip: ""

    QQC2.ToolTip.visible: hovered && tip !== ""
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
}
