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
    Layout.preferredHeight: area.linesHeight + area.topPadding + area.bottomPadding

    QQC2.TextArea {
        id: area

        readonly property real linesHeight: view.lines * lineMetrics.lineSpacing

        // The frame is `lines` lines high even with fewer typed (not just the text's height, which
        // left an empty strip below it); more text scrolls.
        implicitHeight: Math.max(contentHeight, linesHeight) + topPadding + bottomPadding
        wrapMode: TextEdit.NoWrap
        placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
        Keys.onTabPressed: nextItemInFocusChain().forceActiveFocus(Qt.TabFocusReason)
        Keys.onBacktabPressed: nextItemInFocusChain(false).forceActiveFocus(Qt.BacktabFocusReason)
        QQC2.ToolTip.visible: hovered && view.tip !== ""
        QQC2.ToolTip.text: view.tip
        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay

        FontMetrics {
            id: lineMetrics

            font: area.font
        }

        Kirigami.WheelHandler {
            target: view.contentItem
        }
    }
}
