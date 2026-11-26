import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Termigator

// One host-list row: icon · title (DemiBold) over a muted subtitle
// (red for an entry error) · a right-aligned chip, dropped before it squeezes the title; the
// tree arrow sits in the indentation as in the QTreeWidget. Mouse handling is the list's
// (`arrowHit` tells it a press hit the arrow). While `editing` (F2) a field over the title edits
// the name: Enter asks for `commitRequested`, Escape for `cancelRequested`, leaving the field
// commits too (`blur`); the list shows a refused name in `renameError`.
Item {
    id: row

    required property int index
    required property int depth
    required property string kind
    required property string label
    required property string subtitle
    required property string badge
    required property string iconName
    required property string distroColor
    required property string distroGlyph
    required property string distroLogo
    required property bool error
    required property bool expanded
    required property bool hasChildren
    required property string tooltip
    // Recent and its children are no drag source.
    required property bool draggable
    // The environment id, its display name and colour; all "" when none.
    required property string environment
    required property string environmentName
    required property string environmentColor

    // One line (a smaller icon and tighter padding, no subtitle or chip; the tooltip has the rest).
    property bool compact: false
    property bool selected: false
    property bool current: false
    // -1 none, 0 above, 1 on, 2 below (SidebarTree's drop indicator).
    property int dropZone: -1
    // Waiting to be moved by Paste (Ctrl+X): drawn faded, as file managers do.
    property bool cut: false
    property bool editing: false
    property string renameError: ""
    readonly property bool hovered: hover.hovered
    readonly property alias renameField: renameField

    signal commitRequested(string text, bool blur)
    signal cancelRequested

    readonly property int indentation: 14
    readonly property int hPad: 10
    readonly property int vPad: compact ? 4 : 8
    readonly property int iconSize: compact ? 20 : 24
    readonly property int gap: 9
    readonly property int minTitle: 46
    readonly property int contentX: indentation * (depth + 1)
    readonly property color titleColor: selected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
    readonly property color mutedColor: Qt.rgba(titleColor.r, titleColor.g, titleColor.b, 160 / 255)
    readonly property font baseFont: Kirigami.Theme.defaultFont
    readonly property font titleFont: Qt.font({
        family: baseFont.family,
        pointSize: baseFont.pointSize,
        weight: row.compact ? Font.Normal : Font.DemiBold
    })
    readonly property font subFont: Qt.font({
        family: baseFont.family,
        pointSize: Math.max(7.5, baseFont.pointSize - 1)
    })
    readonly property font badgeFont: Qt.font({
        family: baseFont.family,
        pointSize: Math.max(7, baseFont.pointSize - 1.5)
    })
    readonly property bool showSubtitle: !compact && subtitle !== ""
    readonly property int textHeight: fm.height + (showSubtitle ? sfm.height + 2 : 0)
    // The chip only while the title keeps MIN_TITLE pixels.
    readonly property real chipWidth: bfm.advanceWidth(badge.trim()) + 14
    readonly property real textLeft: contentX + hPad + iconSize + gap
    readonly property bool chipShown: !compact && badge.trim() !== "" && (width - hPad - chipWidth - gap) - textLeft >= minTitle

    function arrowHit(px) {
        return hasChildren && px >= indentation * depth && px < contentX;
    }

    // The attached vertical scroll bar overlays the view: keep the badge out from under it.
    width: ListView.view ? ListView.view.width - (ListView.view.rightMargin || 0) : implicitWidth
    implicitHeight: Math.max(textHeight + 2 * vPad, iconSize + 2 * vPad - 2)

    Accessible.role: Accessible.ListItem
    Accessible.name: label
    Accessible.description: environmentName !== "" ? subtitle + " — " + environmentName : subtitle
    Accessible.selected: selected

    onEditingChanged: {
        renameError = "";
        if (editing) {
            renameField.text = label;
            renameField.selectAll();
            renameField.forceActiveFocus(Qt.ShortcutFocusReason);
        } else {
            // The list is a focus scope: it would hand its focus back to the hidden field.
            renameField.focus = false;
        }
    }

    FontMetrics {
        id: fm

        font: row.titleFont
    }
    FontMetrics {
        id: sfm

        font: row.subFont
    }
    FontMetrics {
        id: bfm

        font: row.badgeFont
    }

    HoverHandler {
        id: hover
    }

    QQC2.ToolTip.visible: hovered && tooltip !== "" && row.dropZone < 0
    QQC2.ToolTip.text: tooltip
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay

    // The environment's colour down the row's left edge.
    EnvMark {
        objectName: "envMark"
        environment: row.environment
        colorHint: row.environmentColor
        x: row.contentX
        y: 5
        z: 1
        width: 4
        height: row.height - 10
        radius: 2
    }

    Rectangle {
        id: background

        x: row.contentX + 3
        y: 2
        width: row.width - row.contentX - 6
        height: row.height - 4
        radius: 9
        visible: row.selected || row.hovered
        color: row.selected ? Kirigami.Theme.highlightColor : Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 38 / 255)
    }

    // Keyboard focus on a row that is not selected.
    Rectangle {
        anchors.fill: background
        radius: background.radius
        visible: row.current && !row.selected
        color: "transparent"
        border.width: 1
        border.color: Kirigami.Theme.focusColor
    }

    Kirigami.Icon {
        id: arrow

        x: row.indentation * row.depth + (row.indentation - width) / 2
        anchors.verticalCenter: parent.verticalCenter
        width: 12
        height: 12
        visible: row.hasChildren
        source: row.expanded ? "arrow-down" : (row.LayoutMirroring.enabled ? "arrow-left" : "arrow-right")
    }

    Kirigami.Icon {
        opacity: row.cut ? 0.5 : 1
        x: row.contentX + row.hPad
        anchors.verticalCenter: parent.verticalCenter
        width: row.iconSize
        height: row.iconSize
        visible: row.iconName !== ""
        source: row.iconName
        fallback: "network-server"
        selected: row.selected
    }

    // A distro badge: rounded brand colour, white logo or initial.
    Rectangle {
        opacity: row.cut ? 0.5 : 1
        x: row.contentX + row.hPad + row.iconSize * 0.06
        anchors.verticalCenter: parent.verticalCenter
        width: row.iconSize * 0.88
        height: width
        radius: row.iconSize * 0.22
        visible: row.distroColor !== ""
        color: row.distroColor || "transparent"

        Kirigami.Icon {
            anchors.centerIn: parent
            width: row.iconSize * 0.6
            height: width
            visible: row.distroLogo !== ""
            source: row.distroLogo
            isMask: true
            color: "#ffffff"
        }

        Text {
            anchors.centerIn: parent
            visible: row.distroLogo === ""
            text: row.distroGlyph
            color: "#ffffff"
            font.bold: true
            font.pixelSize: Math.round(row.iconSize * 0.54)
        }
    }

    Rectangle {
        id: chip

        visible: row.chipShown
        width: row.chipWidth
        height: bfm.height + 2
        radius: height / 2
        x: row.width - row.hPad - width
        anchors.verticalCenter: parent.verticalCenter
        color: row.selected ? Qt.rgba(Kirigami.Theme.highlightedTextColor.r, Kirigami.Theme.highlightedTextColor.g, Kirigami.Theme.highlightedTextColor.b, 40 / 255) : Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 28 / 255)

        Text {
            anchors.centerIn: parent
            text: row.badge.trim()
            font: row.badgeFont
            color: row.selected ? Kirigami.Theme.highlightedTextColor : row.mutedColor
        }
    }

    Column {
        opacity: row.cut ? 0.5 : 1
        visible: !row.editing
        x: row.textLeft
        width: Math.max(0, (row.chipShown ? chip.x - row.gap : row.width - row.hPad) - x)
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        Text {
            width: parent.width
            height: fm.height
            text: row.label
            font: row.titleFont
            color: row.titleColor
            elide: Text.ElideRight
            verticalAlignment: Text.AlignVCenter
        }

        Text {
            width: parent.width
            height: sfm.height
            visible: row.showSubtitle
            text: row.subtitle
            font: row.subFont
            color: row.selected ? Kirigami.Theme.highlightedTextColor : (row.error ? Kirigami.Theme.negativeTextColor : row.mutedColor)
            elide: Text.ElideRight
            verticalAlignment: Text.AlignVCenter
        }
    }

    // F2: the name in place.
    QQC2.TextField {
        id: renameField

        objectName: "renameField"
        visible: row.editing
        x: row.textLeft - leftPadding
        width: Math.max(row.minTitle, row.width - row.hPad - x)
        anchors.verticalCenter: parent.verticalCenter
        font: row.titleFont
        Accessible.name: qsTr("New name")
        QQC2.ToolTip.visible: row.editing && row.renameError !== ""
        QQC2.ToolTip.text: row.renameError
        onTextEdited: row.renameError = ""
        // Handled here, so Enter neither reaches the list (it would connect) nor Escape the panel.
        Keys.onReturnPressed: event => {
            event.accepted = true;
            row.commitRequested(text, false);
        }
        Keys.onEnterPressed: event => {
            event.accepted = true;
            row.commitRequested(text, false);
        }
        Keys.onEscapePressed: event => {
            event.accepted = true;
            row.cancelRequested();
        }
        onActiveFocusChanged: {
            if (!activeFocus && row.editing)
                row.commitRequested(text, true);
        }
    }

    // Drop indicator: a line above / below, a frame around the row for "on".
    Rectangle {
        x: background.x
        width: background.width
        height: 2
        y: row.dropZone === 0 ? 0 : row.height - height
        visible: row.dropZone === 0 || row.dropZone === 2
        color: Kirigami.Theme.highlightColor
    }

    Rectangle {
        anchors.fill: background
        radius: background.radius
        visible: row.dropZone === 1
        color: "transparent"
        border.width: 2
        border.color: Kirigami.Theme.highlightColor
    }
}
