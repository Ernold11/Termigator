pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// The remote listing:
// five columns (Permissions and Owner opt-in through the header's context menu), sorted like a
// file manager (".." first, folders before files either way — FilesCore does the ordering),
// multi-selection, the context menu, drag out (rows carry the drag payload for the local pane)
// and drop in (files from a file manager upload). What the actions do is the panel's (`panel`).
FocusScope {
    id: rl

    required property var panel
    required property var files
    // Selected rows (sorted), the current row (the keyboard's / last click) and the shift anchor.
    property var selection: []
    property int currentRow: -1
    property int anchorRow: -1
    property bool showPermissions: false
    property bool showOwner: false
    property QQC2.Menu contextMenu: null
    readonly property alias list: listView
    // Column widths, dragged by the header's grips and remembered in launch.files_column_widths
    // (next to files_columns). A width set by hand is a MINIMUM: Name takes whatever room the
    // other columns leave, so the last column always meets the panel's edge; once the panel is
    // narrower than the minimums the list scrolls sideways with its header. A double click on a
    // grip fits the column to its widest cell (as in a spreadsheet). The header menu's "Reset
    // column widths" forgets them.
    readonly property var defaultWidths: ({
            "size": 74,
            "modified": 132,
            "permissions": 92,
            "owner": 112
        })
    property var widths: ({})

    // Name's own minimum: the default one, or what a grip drag / double click set.
    property int nameMinimumWidth: 80
    readonly property real nameMin: typeof widths.name === "number" ? Math.max(nameMinimumWidth, widths.name) : nameMinimumWidth
    // Name fills the room the fixed columns leave (a binding, so it can never lag a layout change).
    readonly property real nameWidth: Math.max(nameMin, Math.floor(columnsRoom - fixedWidth()))
    // The list's scroll bars float over it: the rows use the width left of the vertical one, like
    // a QTreeView's viewport, so no row runs under it.
    // Only as much as the handle covers: Breeze reports 21 px for a handle drawn ~8 px wide.
    readonly property real scrollBarWidth: vbar.visible ? Math.min(vbar.width, 10) : 0
    readonly property real viewWidth: listView.width - scrollBarWidth
    // Room for the columns inside a row's 4 px margins without scrolling.
    readonly property real columnsRoom: viewWidth - 8
    // A row's width: the view's, or more when the columns are wider, and then the list scrolls
    // sideways with its header.
    readonly property real rowWidth: Math.max(viewWidth, nameMin + fixedWidth() + 8)
    // The rightmost column shown: Name's room ends at the panel's edge, so does this one.
    readonly property int lastColumn: showOwner ? 4 : (showPermissions ? 3 : 2)
    readonly property string lastKey: showOwner ? "owner" : (showPermissions ? "permissions" : "modified")

    function widthFor(key) {
        const w = widths[key];
        return (typeof w === "number" && w >= 40) ? w : defaultWidths[key];
    }

    function fixedWidth() {
        let w = 0;
        for (const k of ["size", "modified", "permissions", "owner"]) {
            if (k === "size" || k === "modified" || (k === "permissions" ? showPermissions : showOwner))
                w += widthFor(k);
        }
        return w;
    }

    function setWidth(key, w) {
        const next = Object.assign({}, widths);
        next[key] = Math.max(key === "name" ? nameMinimumWidth : 40, Math.round(w));
        widths = next;
    }

    signal widthsStored

    function saveWidths() {
        for (const k of Object.keys(widths))
            files.setColumnWidth(k, widths[k]);
        widthsStored();
    }

    // "Reset column widths".
    function resetWidths() {
        files.resetColumnWidths();
        widths = ({});
        widthsStored();
    }

    // The width that shows the widest cell of a column (and its header) whole: a double click on
    // the column's grip. Candidates are the longest texts of the column (the model picks them by
    // length), measured with the labels' font; icon, margins and a gap are added.
    function fitWidth(column) {
        const labels = [qsTr("Name"), qsTr("Size"), qsTr("Modified"), qsTr("Permissions"), qsTr("Owner")];
        metrics.text = labels[column];
        let w = metrics.advanceWidth + 24; // the sort arrow
        for (const text of JSON.parse(files.columnTexts(column))) {
            metrics.text = text;
            w = Math.max(w, metrics.advanceWidth + (column === 0 ? 20 : 0) + 12);
        }
        return Math.ceil(w);
    }

    TextMetrics {
        id: metrics

        font: Kirigami.Theme.defaultFont
    }

    function loadWidths() {
        widths = JSON.parse(files.columnWidths());
    }

    Component.onCompleted: loadWidths()

    readonly property string remoteMime: "application/x-termigator-remote-entries"
    readonly property string emptyText: qsTr("Open an SSH session — its files will show up here.")

    objectName: "remoteList"

    function info(row) {
        return JSON.parse(files.rowInfo(row));
    }

    // The selected rows that are files or folders (not "..").
    function entryRows() {
        return selection.filter(r => info(r).kind === "entry");
    }

    function setCurrent(row, select) {
        currentRow = row;
        if (select) {
            selection = row >= 0 ? [row] : [];
            anchorRow = row;
        }
        if (row >= 0)
            listView.positionViewAtIndex(row, ListView.Contain);
    }

    function selectRange(from, to) {
        const out = [];
        for (let r = Math.min(from, to); r <= Math.max(from, to); ++r)
            out.push(r);
        selection = out;
    }

    function toggleSelected(row) {
        selection = selection.includes(row) ? selection.filter(r => r !== row) : selection.concat([row]).sort((a, b) => a - b);
        anchorRow = row;
    }

    function move(to, mods) {
        if (listView.count === 0)
            return;
        const row = Math.max(0, Math.min(listView.count - 1, to));
        if (mods & Qt.ShiftModifier) {
            selectRange(anchorRow < 0 ? row : anchorRow, row);
            currentRow = row;
            listView.positionViewAtIndex(row, ListView.Contain);
        } else if (mods & Qt.ControlModifier) {
            setCurrent(row, false);
        } else {
            setCurrent(row, true);
        }
    }

    // The rows Delete acts on: the selection when the current row is in it, else the current row.
    function keyRows() {
        if (selection.includes(currentRow))
            return entryRows();
        return currentRow >= 0 && info(currentRow).kind === "entry" ? [currentRow] : [];
    }

    // Enter: a folder or link opens, a file is downloaded (as a double click); ".." goes up.
    function activateKey() {
        if (currentRow < 0)
            return;
        if (info(currentRow).kind === "up")
            files.goUp();
        else
            panel.activate(currentRow);
    }

    // Shift+F10 / Menu: at the current row, else at the top-left corner.
    function menuKey() {
        const item = currentRow >= 0 ? listView.itemAtIndex(currentRow) : null;
        const p = item ? item.mapToItem(listView, item.width / 4, item.height / 2) : Qt.point(0, 0);
        openContextMenu(currentRow, p.x, p.y);
    }

    // Only for the list itself, never for a field or a dialog over it.
    function listKey(event) {
        if (listView.Window.activeFocusItem !== listView) {
            event.accepted = false;
            return;
        }
        const key = event.key, mods = event.modifiers & (Qt.ShiftModifier | Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier);
        const page = Math.max(1, Math.floor(listView.height / 22) - 1);
        const busy = !files.hasTarget || files.busy;
        if (key === Qt.Key_A && mods === Qt.ControlModifier)
            selectRange(0, listView.count - 1);
        else if ((key === Qt.Key_Return || key === Qt.Key_Enter) && mods === 0)
            activateKey();
        else if ((key === Qt.Key_Backspace && mods === 0) || (key === Qt.Key_Up && mods === Qt.AltModifier)) {
            if (!busy)
                files.goUp();
        } else if (key === Qt.Key_Delete && mods === 0)
            panel.deleteRows(keyRows());
        else if (key === Qt.Key_F2 && mods === 0) {
            if (currentRow >= 0 && info(currentRow).kind === "entry")
                panel.rename(currentRow);
        } else if (key === Qt.Key_F5 && mods === 0) {
            if (!busy)
                files.refresh();
        } else if (key === Qt.Key_N && mods === Qt.ControlModifier)
            panel.mkdir();
        else if ((key === Qt.Key_F10 && mods === Qt.ShiftModifier) || (key === Qt.Key_Menu && mods === 0))
            menuKey();
        else if (key === Qt.Key_Escape && mods === 0 && selection.length > 0)
            selection = [];
        else if (key === Qt.Key_Up)
            move(currentRow < 0 ? 0 : currentRow - 1, mods);
        else if (key === Qt.Key_Down)
            move(currentRow + 1, mods);
        else if (key === Qt.Key_Home)
            move(0, mods);
        else if (key === Qt.Key_End)
            move(listView.count - 1, mods);
        else if (key === Qt.Key_PageUp)
            move(currentRow - page, mods);
        else if (key === Qt.Key_PageDown)
            move(currentRow + page, mods);
        else {
            event.accepted = false;
            return;
        }
        event.accepted = true;
    }

    // ---- the context menu

    // [{text, fn}] and "-" for separators; `row` -1 = empty space.
    function menuEntries(row) {
        const out = [];
        if (!files.hasTarget)
            return out;
        const i = row >= 0 ? info(row) : {
            "kind": "none"
        };
        if (i.kind === "up") {
            out.push({
                "text": qsTr("Open parent directory"),
                "keys": "Backspace",
                "fn": () => files.goUp()
            });
            return out;
        }
        let entry = i.kind === "entry" ? i : null;
        const chosen = entryRows();
        if (entry !== null && chosen.length > 1 && selection.includes(row)) {
            // Right-click inside a multi-selection acts on all of it.
            out.push({
                "text": qsTr("Download %1 items…").arg(chosen.length),
                "fn": () => panel.downloadMany(chosen)
            });
            out.push({
                "text": qsTr("Delete %1 items…").arg(chosen.length),
                "keys": "Del",
                "fn": () => panel.deleteRows(chosen)
            });
            out.push({
                "text": qsTr("Permissions of %1 items…").arg(chosen.length),
                "fn": () => panel.permissions(chosen)
            });
            out.push("-");
            entry = null;
        }
        if (entry !== null) {
            if (entry.isDir || entry.isLink)
                out.push({
                    "text": qsTr("Open"),
                    "keys": "Enter",
                    "fn": () => files.activate(row)
                });
            if (entry.isDir) {
                out.push({
                    "text": qsTr("Download folder…"),
                    "fn": () => panel.downloadMany([row])
                });
            } else {
                out.push({
                    "text": qsTr("Edit (auto-upload on save)"),
                    "fn": () => files.editEntry(row)
                });
                out.push({
                    "text": qsTr("Download…"),
                    "fn": () => panel.downloadEntry(row)
                });
            }
            out.push({
                "text": qsTr("Rename…"),
                "keys": "F2",
                "fn": () => panel.rename(row)
            });
            out.push({
                "text": qsTr("Permissions…"),
                "fn": () => panel.permissions([row])
            });
            out.push({
                "text": qsTr("Delete…"),
                "keys": "Del",
                "fn": () => panel.deleteRows([row])
            });
            out.push("-");
            out.push({
                "text": qsTr("Copy remote path"),
                "fn": () => panel.copyText(files.copyPath(row))
            });
            out.push("-");
        }
        out.push({
            "text": qsTr("Upload files here…"),
            "fn": () => panel.uploadFiles()
        });
        out.push({
            "text": qsTr("Upload folder here…"),
            "fn": () => panel.uploadFolder()
        });
        out.push({
            "text": qsTr("New folder…"),
            "keys": "Ctrl+N",
            "fn": () => panel.mkdir()
        });
        out.push({
            "text": qsTr("Refresh"),
            "keys": "F5",
            "fn": () => files.refresh()
        });
        return out;
    }

    function menuTexts(row) {
        return menuEntries(row).map(e => e === "-" ? "-" : e.text).join("|");
    }

    // The key of an entry beside its text (KeyedMenuContent), as in HostsPanel.
    function keyed(item, keys) {
        item.shortcutText = keys;
        item.contentItem = keyedContentComponent.createObject(item, {
            "menuItem": item
        });
    }

    function openContextMenu(row, x, y) {
        const entries = menuEntries(row);
        if (entries.length === 0)
            return;
        if (contextMenu)
            contextMenu.destroy();
        contextMenu = menuComponent.createObject(rl);
        for (const e of entries) {
            if (e === "-")
                contextMenu.addItem(separatorComponent.createObject(null));
            else {
                const item = itemComponent.createObject(null, {
                    "text": e.text,
                    "run": e.fn
                });
                if (e.keys)
                    keyed(item, e.keys);
                contextMenu.addItem(item);
            }
        }
        contextMenu.popup(listView, x, y);
    }

    function openHeaderMenu() {
        headerMenu.popup();
    }

    // ---- drop (files from the file manager upload)

    function rowAt(y) {
        return listView.indexAt(10, y + listView.contentY);
    }

    function droppedPaths(urls) {
        return JSON.parse(files.localDropPaths(JSON.stringify(urls.map(u => u.toString()))));
    }

    // Dropping onto a folder row uploads into that folder, anywhere else into the current one.
    function dropUrls(urls, y) {
        const paths = droppedPaths(urls);
        if (!files.hasTarget || files.cwd === "" || paths.length === 0)
            return false;
        panel.uploadPaths(paths, rowAt(y));
        return true;
    }

    function dragRows(row) {
        // The pinned ".." row (and any non-entry row) has a negative index: nothing to drag.
        if (!(row >= 0))
            return [];
        return selection.includes(row) ? selection : [row];
    }

    Connections {
        target: rl.files

        function onModelReset() {
            rl.selection = [];
            rl.currentRow = -1;
            rl.anchorRow = -1;
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 2

        QQC2.Frame {
            Layout.fillWidth: true
            Layout.fillHeight: true
            padding: 1

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                // The header, inside the view's frame as in a QTreeView: click sorts, right-click
                // chooses the optional columns. One header section spans the strip behind the
                // cells, so the edges outside the columns look like the rest of the header.
                Item {
                    Layout.fillWidth: true
                    // The cells' own widths must not push the panel wider than the dock.
                    Layout.minimumWidth: 0
                    implicitHeight: header.implicitHeight
                    visible: rl.files !== null && rl.files.hasTarget
                    clip: true

                    HeaderCell {
                        anchors.fill: parent
                        position: "only"
                    }

                    RowLayout {
                        id: header

                        // Lined up with the rows' columns and scrolled with the list.
                        x: 4 - listView.contentX
                        width: rl.rowWidth - 8
                        height: parent.height
                        spacing: 0

                        Repeater {
                            model: [
                                {
                                    "text": qsTr("Name"),
                                    "column": 0,
                                    "key": ""
                                },
                                {
                                    "text": qsTr("Size"),
                                    "column": 1,
                                    "key": "size"
                                },
                                {
                                    "text": qsTr("Modified"),
                                    "column": 2,
                                    "key": "modified"
                                },
                                {
                                    "text": qsTr("Permissions"),
                                    "column": 3,
                                    "key": "permissions"
                                },
                                {
                                    "text": qsTr("Owner"),
                                    "column": 4,
                                    "key": "owner"
                                }
                            ]

                            HeaderCell {
                                id: headerLabel

                                required property var modelData

                                readonly property int column: modelData.column
                                readonly property string widthKey: modelData.key

                                objectName: "listHeader" + column
                                // A grip sits on its cell's right edge, over the next cell: earlier cells stack higher.
                                z: 10 - column
                                Layout.preferredWidth: column === 0 ? rl.nameWidth : rl.widthFor(widthKey)
                                Layout.minimumWidth: column === 0 ? rl.nameWidth : rl.widthFor(widthKey)
                                visible: column < 3 || (column === 3 ? rl.showPermissions : rl.showOwner)
                                text: modelData.text
                                sortState: rl.files !== null && rl.files.sortColumn === column ? (rl.files.ascending ? 1 : 2) : 0
                                // The last one shown is the "end" section: no separator of its own next to
                                // the frame's border.
                                readonly property bool last: column === rl.lastColumn
                                position: column === 0 ? "beginning" : (last ? "end" : "middle")
                                extendLeft: column === 0 ? 4 : 0
                                // The last one also covers the strip above the vertical scroll bar.
                                extendRight: last ? 4 + rl.scrollBarWidth : 0

                                MouseArea {
                                    anchors.fill: parent
                                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                                    onClicked: mouse => {
                                        if (mouse.button === Qt.RightButton)
                                            rl.openHeaderMenu();
                                        else
                                            rl.files.sortBy(headerLabel.column);
                                    }
                                }

                                // Drag the right edge of a column: it sets that column's (minimum) width,
                                // Name gives or takes the difference, the list scrolls when the minimums
                                // outgrow it. Saved on release; a double click fits the column.
                                ColumnGrip {
                                    objectName: "grip" + headerLabel.column
                                    headerItem: header
                                    // The last one's grip sits inside the frame.
                                    x: headerLabel.last ? parent.width - width : parent.width - width / 2
                                    width: 8
                                    anchors.top: parent.top
                                    anchors.bottom: parent.bottom
                                    minimum: headerLabel.column === 0 ? rl.nameMinimumWidth : 40
                                    value: headerLabel.column === 0 ? rl.nameWidth : rl.widthFor(headerLabel.widthKey)
                                    onResized: width => rl.setWidth(headerLabel.column === 0 ? "name" : headerLabel.widthKey, width)
                                    onCommitted: rl.saveWidths()
                                    onDoubleClicked: {
                                        rl.setWidth(headerLabel.column === 0 ? "name" : headerLabel.widthKey, rl.fitWidth(headerLabel.column));
                                        rl.saveWidths();
                                    }
                                }
                            }
                        }
                    }
                }

                ListView {
                    id: listView

                    Kirigami.WheelHandler {
                        target: listView
                    }

                    objectName: "remoteListView"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    contentWidth: rl.rowWidth
                    // Room after the rows for the floating scroll bars, so the last row and the
                    // last column can be scrolled out from under them.
                    rightMargin: rl.scrollBarWidth
                    flickableDirection: Flickable.AutoFlickIfNeeded
                    clip: true
                    focus: true
                    activeFocusOnTab: true
                    model: rl.files
                    boundsBehavior: Flickable.StopAtBounds
                    // The wheel scrolls this list (a ListView outside a ScrollView ignores the wheel
                    // when interactive is false). A mouse drag on a row is the row's DnD, not a flick.
                    interactive: true
                    Accessible.role: Accessible.List
                    Accessible.name: qsTr("Remote files")
                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {
                        id: vbar
                    }
                    Keys.onPressed: event => rl.listKey(event)

                    // Clicks below the last row: clear the selection; right-click: the empty-space menu.
                    MouseArea {
                        parent: listView
                        z: -1
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        onPressed: mouse => {
                            listView.forceActiveFocus(Qt.MouseFocusReason);
                            if (!(mouse.modifiers & (Qt.ControlModifier | Qt.ShiftModifier)))
                                rl.setCurrent(-1, true);
                            if (mouse.button === Qt.RightButton)
                                rl.openContextMenu(-1, mouse.x, mouse.y);
                        }
                    }

                    delegate: Rectangle {
                        id: item

                        required property int index
                        required property string kind
                        required property string name
                        required property bool isDir
                        required property bool isLink
                        required property string iconName
                        required property string sizeText
                        required property string mtime
                        required property string mode
                        required property string owner
                        required property string tooltip

                        readonly property bool selected: rl.selection.includes(index)
                        readonly property color textColor: selected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor

                        width: rl.rowWidth
                        height: Math.max(22, nameLabel.implicitHeight + 4)
                        color: selected ? Kirigami.Theme.highlightColor : (index % 2 === 1 ? Kirigami.Theme.alternateBackgroundColor : "transparent")
                        border.width: index === rl.currentRow && listView.activeFocus ? 1 : 0
                        border.color: Kirigami.Theme.highlightColor

                        // Dragged out as the drag payload, for a drop on the local pane.
                        Item {
                            id: dragProxy

                            width: 1
                            height: 1
                            Drag.active: area.drag.active
                            Drag.dragType: Drag.Automatic
                            Drag.supportedActions: Qt.CopyAction
                            Drag.mimeData: {
                                const payload = {};
                                payload[rl.remoteMime] = rl.files.dragPayload(JSON.stringify(rl.dragRows(item.index)));
                                return payload;
                            }
                        }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 4
                            anchors.rightMargin: 4
                            spacing: 0

                            Kirigami.Icon {
                                Layout.preferredWidth: 16
                                Layout.preferredHeight: 16
                                source: item.iconName
                            }
                            QQC2.Label {
                                id: nameLabel

                                Layout.preferredWidth: Math.max(0, rl.nameWidth - 20)
                                Layout.minimumWidth: Math.max(0, rl.nameWidth - 20)
                                Layout.leftMargin: 4
                                text: item.name
                                elide: Text.ElideMiddle
                                color: item.textColor
                            }
                            QQC2.Label {
                                Layout.preferredWidth: rl.widthFor("size")
                                Layout.minimumWidth: rl.widthFor("size")
                                text: item.sizeText
                                color: item.textColor
                            }
                            QQC2.Label {
                                Layout.preferredWidth: rl.widthFor("modified")
                                Layout.minimumWidth: rl.widthFor("modified")
                                text: item.mtime
                                elide: Text.ElideRight
                                color: item.textColor
                            }
                            QQC2.Label {
                                Layout.preferredWidth: rl.widthFor("permissions")
                                Layout.minimumWidth: rl.widthFor("permissions")
                                visible: rl.showPermissions
                                text: item.mode
                                color: item.textColor
                            }
                            QQC2.Label {
                                Layout.preferredWidth: rl.widthFor("owner")
                                Layout.minimumWidth: rl.widthFor("owner")
                                visible: rl.showOwner
                                text: item.owner
                                elide: Text.ElideRight
                                color: item.textColor
                            }
                            Item {
                                Layout.fillWidth: true
                            }
                        }

                        MouseArea {
                            id: area

                            anchors.fill: parent
                            acceptedButtons: Qt.LeftButton | Qt.RightButton
                            hoverEnabled: true
                            drag.target: item.kind === "entry" ? dragProxy : null
                            drag.threshold: Application.styleHints.startDragDistance
                            QQC2.ToolTip.visible: containsMouse && item.tooltip !== ""
                            QQC2.ToolTip.text: item.tooltip
                            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                            onPressed: mouse => {
                                listView.forceActiveFocus(Qt.MouseFocusReason);
                                const ctrl = mouse.modifiers & Qt.ControlModifier;
                                const shift = mouse.modifiers & Qt.ShiftModifier;
                                if (shift && rl.anchorRow >= 0) {
                                    rl.currentRow = item.index;
                                    rl.selectRange(rl.anchorRow, item.index);
                                } else if (ctrl) {
                                    rl.currentRow = item.index;
                                    rl.toggleSelected(item.index);
                                } else if (item.selected) {
                                    rl.currentRow = item.index; // kept for a drag of the whole selection or the menu
                                } else {
                                    rl.setCurrent(item.index, true);
                                }
                                if (mouse.button === Qt.RightButton) {
                                    const p = mapToItem(listView, mouse.x, mouse.y);
                                    rl.openContextMenu(item.index, p.x, p.y);
                                }
                            }
                            onClicked: mouse => {
                                if (mouse.button === Qt.LeftButton && !(mouse.modifiers & (Qt.ControlModifier | Qt.ShiftModifier)))
                                    rl.setCurrent(item.index, true);
                            }
                            onDoubleClicked: mouse => {
                                if (mouse.button === Qt.LeftButton)
                                    rl.panel.activate(item.index);
                            }
                        }
                    }
                }

                // The horizontal bar sits below the view, not over the last row (and no longer
                // crosses the vertical bar in the corner).
                QQC2.ScrollBar {
                    id: hbar

                    objectName: "remoteHBar"
                    orientation: Qt.Horizontal
                    Layout.fillWidth: true
                    Layout.preferredHeight: implicitHeight
                    // Compare the same rounded widths the rows use (not contentWidth, which can
                    // be a frame behind); 1 px of slack for fractional view widths.
                    visible: rl.rowWidth > rl.viewWidth + 1
                    // The scrollable width is the content plus the margin kept for the vertical bar
                    // (list.rightMargin): a bar over `contentWidth` alone stops that margin short,
                    // and the last column stays under the vertical bar.
                    readonly property real span: Math.max(1, listView.contentWidth + listView.rightMargin)
                    size: Math.min(1, listView.width / span)
                    position: listView.contentX / span
                    onPositionChanged: listView.contentX = position * span
                }
            }

            DropArea {
                objectName: "remoteDrop"
                anchors.fill: parent
                onEntered: drag => {
                    drag.accepted = drag.hasUrls && rl.files.hasTarget && rl.files.cwd !== "" && rl.droppedPaths(drag.urls).length > 0;
                }
                onDropped: drop => {
                    // The area spans the header too: the row is looked up in the list's coordinates.
                    drop.accepted = rl.dropUrls(drop.urls, mapToItem(listView, 0, drop.y).y);
                }
            }

            // No session → a hint over the list instead of empty column headers.
            QQC2.Label {
                objectName: "remoteEmpty"
                anchors.centerIn: parent
                width: parent.width - 24
                visible: rl.files === null || !rl.files.hasTarget
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                opacity: 0.6
                text: rl.emptyText
            }
        }
    }

    QQC2.Menu {
        id: headerMenu

        QQC2.MenuItem {
            objectName: "colPermissions"
            text: qsTr("Permissions")
            checkable: true
            checked: rl.showPermissions
            onToggled: rl.panel.setColumn("permissions", checked)
        }
        QQC2.MenuItem {
            objectName: "colOwner"
            text: qsTr("Owner")
            checkable: true
            checked: rl.showOwner
            onToggled: rl.panel.setColumn("owner", checked)
        }
        QQC2.MenuSeparator {}
        QQC2.MenuItem {
            objectName: "colReset"
            text: qsTr("Reset column widths")
            enabled: Object.keys(rl.widths).length > 0
            onTriggered: rl.resetWidths()
        }
    }

    Component {
        id: menuComponent

        QQC2.Menu {}
    }

    Component {
        id: keyedContentComponent

        KeyedMenuContent {}
    }

    Component {
        id: itemComponent

        QQC2.MenuItem {
            property var run: null
            // The entry's key, drawn by KeyedMenuContent ("" = none).
            property string shortcutText: ""

            onTriggered: {
                if (run)
                    run();
            }
        }
    }

    Component {
        id: separatorComponent

        QQC2.MenuSeparator {}
    }
}
