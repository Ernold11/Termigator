import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// The splitter handle of the window and of the terminal panes: 8 px (a minimum width counts it),
// lit on hover and while dragged, with a thin line in the middle.
Rectangle {
    id: grip

    property bool horizontal: true // the splitter's orientation (its attached properties are not reachable here)

    implicitWidth: horizontal ? 8 : 0
    implicitHeight: horizontal ? 0 : 8
    // Opaque: the window is transparent while the terminals are see-through.
    color: QQC2.SplitHandle.pressed ? Kirigami.Theme.highlightColor : QQC2.SplitHandle.hovered ? Qt.tint(Kirigami.Theme.backgroundColor, Qt.alpha(Kirigami.Theme.highlightColor, 0.45)) : Kirigami.Theme.backgroundColor

    Rectangle {
        anchors.centerIn: parent
        width: grip.horizontal ? 1 : parent.width
        height: grip.horizontal ? parent.height : 1
        color: Kirigami.Theme.textColor
        opacity: 0.25
    }
}
