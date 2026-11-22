pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// What dropping the dragged tab would do, over the whole pane area or a detached window
// (`host`: no cross and no edge arrows there). It only draws PanesArea.tabDrag (updateTabDrag
// does the hit-testing) and takes no input.
Item {
    id: overlay

    required property var area
    property Item host: area
    readonly property var drag: area.tabDrag && area.tabDrag.host === host ? area.tabDrag : null
    readonly property var target: drag ? drag.target : null
    readonly property real guide: area.dockGuide
    readonly property real step: guide * 1.25
    // The cross only where it fits, as in hit_zone.
    readonly property bool crossShown: drag !== null && drag.pane !== null && drag.pane.width / 2 - step - guide / 2 >= 0 && drag.pane.height / 2 - step - guide / 2 >= 0

    visible: drag !== null

    component Square: Rectangle {
        property string dock
        property bool edge: false
        property alias iconName: icon.source

        readonly property bool hot: overlay.target !== null && !overlay.target.line && overlay.target.dock === dock && !!overlay.target.edge === edge
        width: overlay.guide
        height: overlay.guide
        radius: Kirigami.Units.cornerRadius
        color: hot ? Kirigami.Theme.highlightColor : Kirigami.Theme.backgroundColor
        border.color: Kirigami.Theme.highlightColor

        Kirigami.Icon {
            id: icon

            anchors.centerIn: parent
            width: Kirigami.Units.iconSizes.smallMedium
            height: width
            color: parent.hot ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
        }
    }

    // The room the tab would take.
    Rectangle {
        readonly property var r: overlay.target && overlay.target.rect ? overlay.target.rect : null

        visible: r !== null
        x: r ? r.x : 0
        y: r ? r.y : 0
        width: r ? r.width : 0
        height: r ? r.height : 0
        color: Kirigami.Theme.highlightColor
        opacity: 0.3
    }

    // The insertion line on a tab bar.
    Rectangle {
        readonly property var r: overlay.target && overlay.target.line ? overlay.target.line : null

        visible: r !== null
        x: r ? r.x : 0
        y: r ? r.y : 0
        width: r ? r.width : 0
        height: r ? r.height : 0
        color: Kirigami.Theme.highlightColor
    }

    // The cross over the pane under the cursor.
    Repeater {
        model: [["center", 0, 0, "tab-new"], ["left", -1, 0, "go-previous"], ["right", 1, 0, "go-next"], ["top", 0, -1, "go-up"], ["bottom", 0, 1, "go-down"]]

        Square {
            required property var modelData

            visible: overlay.crossShown
            dock: modelData[0]
            iconName: modelData[3]
            x: overlay.crossShown ? overlay.drag.pane.x + overlay.drag.pane.width / 2 + modelData[1] * overlay.step - overlay.guide / 2 : 0
            y: overlay.crossShown ? overlay.drag.pane.y + overlay.drag.pane.height / 2 + modelData[2] * overlay.step - overlay.guide / 2 : 0
        }
    }

    // The arrows at the outer edges of the area.
    Repeater {
        model: overlay.host === overlay.area ? [["left", "go-previous"], ["right", "go-next"], ["top", "go-up"], ["bottom", "go-down"]] : []

        Square {
            required property var modelData
            readonly property rect r: overlay.area.edgeRect(modelData[0])

            edge: true
            dock: modelData[0]
            iconName: modelData[1]
            x: r.x
            y: r.y
        }
    }

    // The ghost: the tab's title at the cursor.
    QQC2.Label {
        visible: overlay.drag !== null && overlay.drag.pos !== null
        x: visible ? overlay.drag.pos.x + Kirigami.Units.largeSpacing : 0
        y: visible ? overlay.drag.pos.y + Kirigami.Units.largeSpacing : 0
        padding: Kirigami.Units.smallSpacing
        text: overlay.drag ? overlay.drag.tab.text : ""
        background: Rectangle {
            color: Kirigami.Theme.backgroundColor
            border.color: Kirigami.Theme.highlightColor
            radius: Kirigami.Units.cornerRadius
            opacity: 0.9
        }
    }
}
