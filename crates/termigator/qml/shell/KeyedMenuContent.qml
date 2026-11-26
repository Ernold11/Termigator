import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A context-menu entry's icon and text with its key at the right edge: the contentItem of a
// MenuItem with `shortcutText`. Not an Action shortcut: that one would be live while the menu is
// open (Enter, Delete) and how it is drawn depends on the style.
RowLayout {
    id: keyedRow

    property var menuItem: null
    readonly property color textColor: !menuItem ? Kirigami.Theme.textColor : menuItem.highlighted ? menuItem.palette.highlightedText : menuItem.enabled ? menuItem.palette.windowText : Kirigami.Theme.disabledTextColor

    spacing: Kirigami.Units.largeSpacing

    Item {
        Layout.preferredWidth: Kirigami.Units.smallSpacing
    }
    Kirigami.Icon {
        Layout.alignment: Qt.AlignVCenter
        Layout.preferredWidth: Kirigami.Units.iconSizes.small
        Layout.preferredHeight: Kirigami.Units.iconSizes.small
        source: keyedRow.menuItem ? keyedRow.menuItem.icon.name : ""
        visible: source !== ""
    }
    QQC2.Label {
        Layout.fillWidth: true
        text: keyedRow.menuItem ? keyedRow.menuItem.text : ""
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
        color: keyedRow.textColor
    }
    QQC2.Label {
        text: keyedRow.menuItem ? keyedRow.menuItem.shortcutText : ""
        verticalAlignment: Text.AlignVCenter
        color: keyedRow.textColor
    }
    Item {
        Layout.preferredWidth: Kirigami.Units.smallSpacing
    }
}
