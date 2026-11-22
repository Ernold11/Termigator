import QtQuick
import QtQuick.Layouts
import org.kde.qqc2desktopstyle.private as StylePrivate

// One column header painted by the widget style (QStyle CE_Header), like QHeaderView and
// org.kde.desktop's own HorizontalHeaderView: section background and separators, left-aligned
// text, the sort arrow at the section's right edge. Children (a MouseArea, a ColumnGrip) go on top.
Item {
    id: cell

    property alias text: section.text
    // 0: not the sort column, 1: ascending, 2: descending.
    property int sortState: 0
    // "beginning", "middle", "end" or "only": how the style joins it to its neighbours.
    property string position: "middle"
    // The section is painted this far past the cell's left / right edge: the outer sections cover
    // the view's margins, so the header runs from frame to frame as a QHeaderView does.
    property real extendLeft: 0
    property real extendRight: 0

    implicitWidth: section.implicitWidth
    implicitHeight: section.implicitHeight
    // Not fillHeight: a row with a filling cell fills its column too and takes the list's room.
    Layout.preferredHeight: implicitHeight

    StylePrivate.StyleItem {
        id: section

        anchors.fill: parent
        anchors.leftMargin: -cell.extendLeft
        anchors.rightMargin: -cell.extendRight
        elementType: "header"
        // Ascending shows the down-pointing arrow, as QHeaderView does; StyleItem's "up"
        // control paints that one.
        activeControl: cell.sortState === 1 ? "up" : (cell.sortState === 2 ? "down" : "")
        properties: {
            "headerpos": cell.position,
            "textalignment": Text.AlignVCenter | Text.AlignLeft,
            "orientation": Qt.Horizontal
        }
    }
}
