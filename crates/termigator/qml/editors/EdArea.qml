import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A multi-line field with a fixed height of `lines` lines (QPlainTextEdit.setFixedHeight).
// Enter adds a line; Tab moves on, as in a plain-text edit that does not take tabs.
QQC2.ScrollView {
    id: view

    property alias text: area.text
    property alias placeholderText: area.placeholderText
    property int lines: 4
    property string tip: ""

    Kirigami.FormData.buddyFor: area
    Layout.fillWidth: true
    Layout.preferredHeight: lines * area.font.pixelSize * 1.5 + Kirigami.Units.largeSpacing * 2

    QQC2.TextArea {
        id: area

        wrapMode: TextEdit.NoWrap
        placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
        Keys.onTabPressed: nextItemInFocusChain().forceActiveFocus(Qt.TabFocusReason)
        Keys.onBacktabPressed: nextItemInFocusChain(false).forceActiveFocus(Qt.BacktabFocusReason)
        QQC2.ToolTip.visible: hovered && view.tip !== ""
        QQC2.ToolTip.text: view.tip
        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay

        Kirigami.WheelHandler {
            target: view.contentItem
        }
    }
}
