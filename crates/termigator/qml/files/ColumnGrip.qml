import QtQuick
import org.kde.kirigami as Kirigami

// A drag handle on a header cell's right edge: dragging right widens that column (the columns to
// its right keep their widths and move along; Name, which fills, takes the difference). Positions
// are measured in `header`, which does not move while the cell resizes.
MouseArea {
    id: grip

    required property Item headerItem
    property int value: 0
    property int minimum: 40
    property real startX: 0
    property int startW: 0

    signal resized(int width)
    signal committed

    function dragTo(x) {
        resized(Math.max(minimum, Math.round(startW + (x - startX))));
    }

    width: 8
    cursorShape: Qt.SplitHCursor
    preventStealing: true
    hoverEnabled: true
    onPressed: mouse => {
        startX = mapToItem(headerItem, mouse.x, 0).x;
        startW = value;
    }
    onPositionChanged: mouse => {
        if (pressed)
            dragTo(mapToItem(headerItem, mouse.x, 0).x);
    }
    onReleased: committed()

    // Shown only while the grip is hovered or used: the header style draws the separators (a
    // second, always visible line drifts off the outer sections, which reach past the cell).
    Rectangle {
        objectName: "gripLine"
        anchors.horizontalCenter: parent.horizontalCenter
        width: 1
        height: parent.height
        color: Kirigami.Theme.highlightColor
        visible: grip.pressed || grip.containsMouse
    }
}
