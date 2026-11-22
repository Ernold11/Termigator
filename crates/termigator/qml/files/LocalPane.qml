pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import org.kde.kirigami as Kirigami
import Termigator

// The local half of the two-pane Files view:
// WinSCP's Commander layout. A local folder beside the remote listing: drag files from here onto
// the remote list to upload (plain file URLs, the same drop the file manager makes), drag remote
// rows here to download into this folder, or use the → button.
ColumnLayout {
    id: pane

    // The remote-rows drag format (`application/x-termigator-remote-entries`).
    readonly property string remoteMime: "application/x-termigator-remote-entries"
    property string startDir: ""
    property string path: ""
    // Selected file paths (ExtendedSelection) and the anchor row.
    property var selection: []
    property int anchorRow: -1
    readonly property alias list: localListView
    readonly property alias model: folderModel
    // Column widths work as in RemoteList.qml and share its store (launch.files_column_widths):
    // a width set by hand is a minimum, Name fills the rest, the list scrolls sideways below the
    // sum of the minimums, a double click on a grip fits the column, the header menu resets them.
    property var files: null
    // The Files panel: its prompt, confirmation and message boxes serve the context menu.
    property var host: null
    property var contextMenu: null
    // Alone in the panel (a local shell is in front): there is no remote folder to upload into.
    property bool standalone: false
    // The Files bar's "Show hidden files" for a local shell; the two-pane side shows all.
    property bool showHidden: true
    readonly property var defaultWidths: ({
            "size": 74,
            "modified": 132
        })
    property var widths: ({})
    property int nameMinimumWidth: 80
    readonly property real nameMin: typeof widths.name === "number" ? Math.max(nameMinimumWidth, widths.name) : nameMinimumWidth
    // Only as much as the handle covers (see RemoteList.qml).
    readonly property real scrollBarWidth: localVBar.visible ? Math.min(localVBar.width, 10) : 0
    readonly property real viewWidth: localListView.width - scrollBarWidth
    readonly property real nameWidth: Math.max(nameMin, Math.floor(viewWidth - 8 - fixedWidth()))
    readonly property real rowWidth: Math.max(viewWidth, nameMin + fixedWidth() + 8)

    signal uploadRequested(var paths)
    signal remoteDropped(string payload, string target)
    signal dirChanged(string path)
    // The user went to another folder here (not the shell's folder being followed).
    signal userNavigated
    // The widths were saved or reset: the remote list reloads them.
    signal widthsStored

    function widthFor(key) {
        const w = widths[key];
        return (typeof w === "number" && w >= 40) ? w : defaultWidths[key];
    }

    function fixedWidth() {
        return widthFor("size") + widthFor("modified");
    }

    function setWidth(key, w) {
        const next = Object.assign({}, widths);
        next[key] = Math.max(key === "name" ? nameMinimumWidth : 40, Math.round(w));
        widths = next;
    }

    function saveWidths() {
        if (files === null)
            return;
        for (const k of Object.keys(widths))
            files.setColumnWidth(k, widths[k]);
        widthsStored();
    }

    function resetWidths() {
        if (files !== null)
            files.resetColumnWidths();
        widths = ({});
        widthsStored();
    }

    function loadWidths() {
        widths = files !== null ? JSON.parse(files.columnWidths()) : ({});
    }

    // The width that shows the widest cell of a column (and its header) whole.
    function fitWidth(column) {
        metrics.text = [qsTr("Name"), qsTr("Size"), qsTr("Modified")][column];
        let w = metrics.advanceWidth + 24; // the sort arrow
        if (column === 2) {
            metrics.text = "0000-00-00 00:00";
            w = Math.max(w, metrics.advanceWidth + 12);
        } else {
            for (let r = 0; r < Math.min(folderModel.count, 3000); ++r) {
                metrics.text = column === 0 ? folderModel.get(r, "fileName") : (folderModel.get(r, "fileIsDir") ? "" : helper.humanSize(folderModel.get(r, "fileSize")));
                w = Math.max(w, metrics.advanceWidth + (column === 0 ? 20 : 0) + 12);
            }
        }
        return Math.ceil(w);
    }

    // Through QUrl: a hand-made "file://" + path breaks Windows drive paths (C:\dir).
    function toUrl(p) {
        return helper.fileUrl(p);
    }

    function currentDir() {
        return path !== "" ? path : helper.homeDir();
    }

    function setDir(p) {
        if (p === "" || !helper.isDir(p)) {
            pathField.text = currentDir();
            return;
        }
        // Windows paths keep one spelling (C:\dir), whatever the caller used: the shell reports
        // backslashes, the folder model slashes, and the follow timer compares them.
        const win = /^[A-Za-z]:/.test(p);
        if (win)
            p = p.length === 2 ? p + "\\" : p.replace(/\//g, "\\");
        const clean = p.length > (win ? 3 : 1) ? p.replace(win ? /\\+$/ : /\/+$/, "") : p;
        path = clean === "" ? "/" : clean;
        folderModel.folder = toUrl(path);
        pathField.text = path;
        selection = [];
        anchorRow = -1;
        dirChanged(path);
    }

    function goUp() {
        userNavigated();
        setDir(helper.parentDir(currentDir()));
    }

    // Folder changes made by the user: they pause following the shell's folder.
    function goTo(p) {
        userNavigated();
        setDir(p);
    }

    function selectedPaths() {
        return selection.slice();
    }

    function uploadSelected() {
        const paths = selectedPaths();
        if (paths.length > 0)
            uploadRequested(paths);
    }

    function activate(row) {
        const p = folderModel.get(row, "filePath");
        if (folderModel.get(row, "fileIsDir"))
            goTo(p);
        else
            Qt.openUrlExternally(toUrl(p));
    }

    function select(row, mods) {
        const p = folderModel.get(row, "filePath");
        if ((mods & Qt.ShiftModifier) && anchorRow >= 0) {
            const out = [];
            for (let r = Math.min(anchorRow, row); r <= Math.max(anchorRow, row); ++r)
                out.push(folderModel.get(r, "filePath"));
            selection = out;
            return;
        }
        if (mods & Qt.ControlModifier)
            selection = selection.includes(p) ? selection.filter(x => x !== p) : selection.concat([p]);
        else if (!selection.includes(p))
            selection = [p];
        anchorRow = row;
    }

    // ---- the context menu (the remote list's menu for the local side)

    function nameOf(p) {
        return p.split(/[\\/]/).filter(x => x !== "").pop() || p;
    }

    function fail(message) {
        if (message !== "" && host)
            host.inform(message);
    }

    // Read the folder again. The folder model's own watcher misses changes on Windows, so every
    // change made from this pane ends with this.
    function refresh() {
        const url = folderModel.folder;
        folderModel.folder = "";
        folderModel.folder = url;
    }

    function makeFolder() {
        if (!host)
            return;
        host.askText(qsTr("New local folder"), qsTr("Folder name:"), "", name => {
            fail(helper.localMkdir(currentDir(), name));
            refresh();
        });
    }

    function renameEntry(p) {
        if (!host)
            return;
        host.askText(qsTr("Rename"), qsTr("New name:"), nameOf(p), name => {
            const error = helper.localRename(p, name);
            fail(error);
            if (error === "")
                selection = [];
            refresh();
        });
    }

    function deleteEntries(paths) {
        if (!host || paths.length === 0)
            return;
        const question = paths.length === 1 ? qsTr("Delete \"%1\" from this computer? A folder goes with everything in it.").arg(nameOf(paths[0])) : qsTr("Delete %1 items from this computer? Folders go with everything in them.").arg(paths.length);
        host.confirm(question, () => {
            fail(helper.localDelete(JSON.stringify(paths)));
            selection = [];
            refresh();
        });
    }

    // [{text, fn}] and "-" for separators; `row` -1 = empty space. The same order as the remote
    // list's menu (RemoteList.qml), without what only a server has (permissions, edit).
    function menuEntries(row) {
        const out = [];
        const canUpload = !standalone && files && files.hasTarget;
        if (row >= 0) {
            const p = folderModel.get(row, "filePath");
            const isDir = folderModel.get(row, "fileIsDir");
            if (selection.length > 1 && selection.includes(p)) {
                // Right-click inside a multi-selection acts on all of it.
                const chosen = selection.slice();
                if (canUpload)
                    out.push({
                        "text": qsTr("Upload %1 items").arg(chosen.length),
                        "fn": () => {
                            selection = chosen;
                            uploadSelected();
                        }
                    });
                out.push({
                    "text": qsTr("Delete %1 items…").arg(chosen.length),
                    "fn": () => deleteEntries(chosen)
                });
                out.push("-");
            } else {
                out.push({
                    "text": isDir ? qsTr("Open") : qsTr("Open with the default application"),
                    "fn": () => activate(row)
                });
                if (canUpload)
                    out.push({
                        "text": qsTr("Upload to the remote folder"),
                        "fn": () => {
                            selection = [p];
                            uploadSelected();
                        }
                    });
                out.push({
                    "text": qsTr("Rename…"),
                    "fn": () => renameEntry(p)
                });
                out.push({
                    "text": qsTr("Delete…"),
                    "fn": () => deleteEntries([p])
                });
                out.push("-");
                out.push({
                    "text": qsTr("Copy path"),
                    "fn": () => {
                        if (host)
                            host.copyText(p);
                    }
                });
                out.push({
                    "text": qsTr("Show in the file manager"),
                    "fn": () => Qt.openUrlExternally(toUrl(isDir ? p : helper.parentDir(p)))
                });
                out.push("-");
            }
        }
        out.push({
            "text": qsTr("New folder…"),
            "fn": () => makeFolder()
        });
        out.push({
            "text": qsTr("Refresh"),
            "fn": () => refresh()
        });
        return out;
    }

    function menuTexts(row) {
        return menuEntries(row).map(e => e === "-" ? "-" : e.text).join("|");
    }

    function openContextMenu(row, x, y) {
        const entries = menuEntries(row);
        if (contextMenu)
            contextMenu.destroy();
        contextMenu = menuComponent.createObject(pane);
        for (const e of entries) {
            if (e === "-")
                contextMenu.addItem(separatorComponent.createObject(null));
            else
                contextMenu.addItem(itemComponent.createObject(null, {
                    "text": e.text,
                    "run": e.fn
                }));
        }
        contextMenu.popup(localListView, x, y);
    }

    // Remote rows dropped here: into the folder under the cursor, else this one.
    function dropRemote(payload, y) {
        const row = localListView.indexAt(10, y + localListView.contentY);
        const target = row >= 0 && folderModel.get(row, "fileIsDir") ? folderModel.get(row, "filePath") : currentDir();
        remoteDropped(payload, target);
    }

    function dragUris(p) {
        const paths = selection.includes(p) ? selection : [p];
        return paths.map(toUrl).join("\r\n");
    }

    spacing: 2

    Component.onCompleted: {
        loadWidths();
        setDir(startDir !== "" ? startDir : helper.homeDir());
    }

    TextMetrics {
        id: metrics

        font: Kirigami.Theme.defaultFont
    }

    FilesHelper {
        id: helper
    }

    FolderListModel {
        id: folderModel

        showDirs: true
        showFiles: true
        showHidden: pane.showHidden
        showDotAndDotDot: false
        showDirsFirst: true
        sortField: FolderListModel.Name
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 2

        QQC2.ToolButton {
            objectName: "localUp"
            icon.name: "go-up"
            // The text is what a screen reader says; IconOnly keeps it off screen.
            text: qsTr("Parent folder")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: text
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            onClicked: pane.goUp()
        }
        QQC2.ToolButton {
            objectName: "localHome"
            icon.name: "go-home"
            // The text is what a screen reader says; IconOnly keeps it off screen.
            text: qsTr("Home folder")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: text
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            onClicked: pane.goTo(helper.homeDir())
        }
        QQC2.TextField {
            id: pathField

            objectName: "localPath"
            Layout.fillWidth: true
            placeholderText: qsTr("Local folder…")
            placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
            onAccepted: pane.goTo(helper.expandUser(text.trim()))
        }
        QQC2.ToolButton {
            objectName: "localUpload"
            visible: !pane.standalone
            icon.name: "go-next"
            // The text is what a screen reader says; IconOnly keeps it off screen.
            text: qsTr("Upload the selected files / folders into the remote folder")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: text
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            onClicked: pane.uploadSelected()
        }
    }

    QQC2.Frame {
        Layout.fillWidth: true
        Layout.fillHeight: true
        padding: 1

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            // Name · Size · Modified (the QFileSystemModel's "Kind" column is hidden), inside the
            // view's frame as in a QTreeView; one section behind the cells fills the strip's edges.
            // Lined up with the rows and scrolled with the list, as in RemoteList.qml.
            Item {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                implicitHeight: localHeader.implicitHeight
                clip: true

                HeaderCell {
                    anchors.fill: parent
                    position: "only"
                }

                RowLayout {
                    id: localHeader

                    x: 4 - localListView.contentX
                    width: pane.rowWidth - 8
                    height: parent.height
                    spacing: 0

                    Repeater {
                        model: [[qsTr("Name"), FolderListModel.Name, ""], [qsTr("Size"), FolderListModel.Size, "size"], [qsTr("Modified"), FolderListModel.Time, "modified"]]

                        HeaderCell {
                            id: cell

                            required property var modelData
                            required property int index

                            objectName: "localHeader" + index
                            // A grip sits on its cell's right edge, over the next cell: earlier cells stack higher.
                            z: 10 - index
                            Layout.preferredWidth: index === 0 ? pane.nameWidth : pane.widthFor(modelData[2])
                            Layout.minimumWidth: Layout.preferredWidth
                            text: modelData[0]
                            sortState: folderModel.sortField === modelData[1] ? (folderModel.sortReversed ? 2 : 1) : 0
                            position: index === 0 ? "beginning" : (index === 2 ? "end" : "middle")
                            extendLeft: index === 0 ? 4 : 0
                            extendRight: index === 2 ? 4 + pane.scrollBarWidth : 0

                            MouseArea {
                                anchors.fill: parent
                                acceptedButtons: Qt.LeftButton | Qt.RightButton
                                onClicked: mouse => {
                                    if (mouse.button === Qt.RightButton) {
                                        localHeaderMenu.popup();
                                    } else if (folderModel.sortField === cell.modelData[1]) {
                                        folderModel.sortReversed = !folderModel.sortReversed;
                                    } else {
                                        folderModel.sortField = cell.modelData[1];
                                        folderModel.sortReversed = false;
                                    }
                                }
                            }

                            ColumnGrip {
                                objectName: "localGrip" + cell.index
                                headerItem: localHeader
                                x: cell.index === 2 ? parent.width - width : parent.width - width / 2
                                width: 8
                                anchors.top: parent.top
                                anchors.bottom: parent.bottom
                                minimum: cell.index === 0 ? pane.nameMinimumWidth : 40
                                value: cell.index === 0 ? pane.nameWidth : pane.widthFor(cell.modelData[2])
                                onResized: width => pane.setWidth(cell.index === 0 ? "name" : cell.modelData[2], width)
                                onCommitted: pane.saveWidths()
                                onDoubleClicked: {
                                    pane.setWidth(cell.index === 0 ? "name" : cell.modelData[2], pane.fitWidth(cell.index));
                                    pane.saveWidths();
                                }
                            }
                        }
                    }
                }
            }

            ListView {
                id: localListView

                Kirigami.WheelHandler {
                    target: localListView
                }

                objectName: "localList"
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                contentWidth: pane.rowWidth
                rightMargin: pane.scrollBarWidth
                flickableDirection: Flickable.AutoFlickIfNeeded
                model: folderModel
                boundsBehavior: Flickable.StopAtBounds
                QQC2.ScrollBar.vertical: QQC2.ScrollBar {
                    id: localVBar
                }

                // Below the last row: right-click opens the folder's menu.
                MouseArea {
                    parent: localListView
                    z: -1
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onPressed: mouse => {
                        localListView.forceActiveFocus(Qt.MouseFocusReason);
                        if (!(mouse.modifiers & (Qt.ControlModifier | Qt.ShiftModifier)))
                            pane.selection = [];
                        if (mouse.button === Qt.RightButton)
                            pane.openContextMenu(-1, mouse.x, mouse.y);
                    }
                }

                delegate: Rectangle {
                    id: item

                    required property int index
                    required property string filePath
                    required property string fileName
                    required property bool fileIsDir
                    required property var fileSize
                    required property var fileModified

                    readonly property bool selected: pane.selection.includes(filePath)

                    width: pane.rowWidth
                    height: Math.max(22, nameLabel.implicitHeight + 4)
                    color: selected ? Kirigami.Theme.highlightColor : (index % 2 === 1 ? Kirigami.Theme.alternateBackgroundColor : "transparent")

                    // Rows are dragged out as file URLs, the drop a file manager makes.
                    Item {
                        id: dragProxy

                        width: 1
                        height: 1
                        Drag.active: area.drag.active
                        Drag.dragType: Drag.Automatic
                        Drag.supportedActions: Qt.CopyAction
                        Drag.mimeData: ({
                                "text/uri-list": pane.dragUris(item.filePath)
                            })
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 4
                        anchors.rightMargin: 4
                        spacing: 0

                        Kirigami.Icon {
                            Layout.preferredWidth: 16
                            Layout.preferredHeight: 16
                            source: item.fileIsDir ? "folder" : "text-x-generic"
                        }
                        QQC2.Label {
                            id: nameLabel

                            Layout.preferredWidth: Math.max(0, pane.nameWidth - 20)
                            Layout.minimumWidth: Math.max(0, pane.nameWidth - 20)
                            Layout.leftMargin: 4
                            text: item.fileName
                            elide: Text.ElideMiddle
                            color: item.selected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        }
                        QQC2.Label {
                            Layout.preferredWidth: pane.widthFor("size")
                            text: item.fileIsDir ? "" : helper.humanSize(item.fileSize)
                            color: item.selected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        }
                        QQC2.Label {
                            Layout.preferredWidth: pane.widthFor("modified")
                            text: Qt.formatDateTime(item.fileModified, "yyyy-MM-dd hh:mm")
                            color: item.selected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        }
                    }

                    MouseArea {
                        id: area

                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        drag.target: (area.pressedButtons & Qt.RightButton) ? null : dragProxy
                        drag.threshold: Qt.styleHints.startDragDistance
                        onPressed: mouse => {
                            if (!pane.selection.includes(item.filePath) || (mouse.modifiers & (Qt.ControlModifier | Qt.ShiftModifier)))
                                pane.select(item.index, mouse.modifiers);
                            if (mouse.button === Qt.RightButton) {
                                const p = mapToItem(localListView, mouse.x, mouse.y);
                                pane.openContextMenu(item.index, p.x, p.y);
                            }
                        }
                        onClicked: mouse => {
                            if (mouse.button === Qt.LeftButton && !(mouse.modifiers & (Qt.ControlModifier | Qt.ShiftModifier)))
                                pane.selection = [item.filePath];
                        }
                        onDoubleClicked: mouse => {
                            if (mouse.button === Qt.LeftButton)
                                pane.activate(item.index);
                        }
                    }
                }
            }

            // Below the view, not over the last row (see RemoteList.qml).
            QQC2.ScrollBar {
                id: localHBar

                objectName: "localHBar"
                orientation: Qt.Horizontal
                Layout.fillWidth: true
                Layout.preferredHeight: implicitHeight
                visible: pane.rowWidth > pane.viewWidth + 1
                readonly property real span: Math.max(1, localListView.contentWidth + localListView.rightMargin)
                size: Math.min(1, localListView.width / span)
                position: localListView.contentX / span
                onPositionChanged: localListView.contentX = position * span
            }
        }

        QQC2.Menu {
            id: localHeaderMenu

            QQC2.MenuItem {
                objectName: "localColReset"
                text: qsTr("Reset column widths")
                enabled: Object.keys(pane.widths).length > 0
                onTriggered: pane.resetWidths()
            }
        }

        DropArea {
            anchors.fill: parent
            onEntered: drag => {
                drag.accepted = drag.formats.indexOf(pane.remoteMime) >= 0; // local files are moved with the file manager
            }
            onDropped: drop => {
                if (drop.formats.indexOf(pane.remoteMime) < 0) {
                    drop.accepted = false;
                    return;
                }
                drop.accepted = true;
                // The area spans the header too: the row is looked up in the list's coordinates.
                pane.dropRemote(drop.getDataAsString(pane.remoteMime), mapToItem(localListView, 0, drop.y).y);
            }
        }
    }

    Component {
        id: menuComponent

        QQC2.Menu {}
    }

    Component {
        id: itemComponent

        QQC2.MenuItem {
            property var run: null

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
