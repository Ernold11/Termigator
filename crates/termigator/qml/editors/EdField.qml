import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A one-line field of the connection editors: fills the form's width, tooltip from `tip`.
QQC2.TextField {
    property string tip: ""

    // The style's disabled-text colour is too faint on a light background.
    placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
    Layout.fillWidth: true
    QQC2.ToolTip.visible: hovered && tip !== ""
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
}
