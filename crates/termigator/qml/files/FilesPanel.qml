pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// The Files panel: a
// remote file browser — toolbar, path bar, the listing (RemoteList.qml), an optional local pane
// beside it (LocalPane.qml, WinSCP's Commander layout), the transfer queue (TransfersView.qml)
// and a status line. The side panel (FilesSidePanel.qml) points it at the active SSH tab; a
// Files tab (FilesHost.qml) is bound to one entry. The state is `session.files` (FilesCore:
// listing and commands) and `session.transfers` (TransfersCore: the queue), which outlive this
// view; what is here is the dialogs and the flows between them (clash questions, selections).
FocusScope {
    id: filesRoot

    required property var session
    required property var files
    required property var transfers
    readonly property var shell: session.host.shell
    property bool twoPane: false
    // In the side dock the window's status bar shows the status line instead (ShellWindow).
    property bool statusInFooter: false
    // A local shell is in front: only this machine's folder, no host (FilesSidePanel.qml).
    property bool localMode: false
    // "Follow terminal folder": the last folder the terminal reported, and whether following is
    // paused because the user went elsewhere in Files (the button shows off meanwhile). A click
    // on the button, or a cd in the terminal to another folder, takes it back up.
    property string terminalDir: ""
    property bool followPaused: false
    // Local mode: go to the shell's folder now (FilesSidePanel reads it).
    signal localFollowRequested
    readonly property string followPausedTip: qsTr("Following paused: you went to another folder here.\nClick to go back to the terminal's folder (a cd in the terminal does it too).")
    readonly property alias list: remote
    readonly property alias followButton: followTool
    readonly property alias hiddenButton: hiddenButton
    readonly property alias twoPaneButton: twoPaneTool
    readonly property alias pathField: pathField
    readonly property alias transfersView: transfersView
    readonly property alias localLoader: localPaneLoader
    readonly property alias saveDialog: saveFileDialog
    readonly property alias messageBox: box
    readonly property alias askBox: ask3
    readonly property alias prompt: promptDialog
    readonly property alias permissionsDialog: permDialog
    readonly property alias bookmarkMenu: bookmarkMenu
    readonly property var local: localPaneLoader.item
    // The status line: the remote side's, or for a local shell this machine's folder (the remote
    // side then only has its "open an SSH session" hint).
    readonly property string statusText: localMode && local ? qsTr("This computer · %1 — %2 shown").arg(local.path).arg(local.model.count) : files.status
    readonly property alias toolbar: toolbarFlow

    // The side dock and the window minimum width bind to it (ShellWindow.qml): the toolbar no
    // longer sets it, the icons that do not fit hide behind its overflow button.
    readonly property real minimumPanelWidth: 140

    objectName: "filesPanel"

    // ---- small helpers

    // Through QUrl: a hand-made "file://" + path breaks Windows drive paths (C:\dir).
    function urlToPath(url) {
        return helper.localPath(url.toString());
    }

    function pathToUrl(p) {
        return helper.fileUrl(p);
    }

    function copyText(text) {
        clipboard.text = text;
        clipboard.selectAll();
        clipboard.copy();
    }

    function inform(text) {
        box.text = text;
        box.open();
    }

    // QMessageBox.question Yes / No: `yes()` runs after Yes.
    function confirm(text, yes) {
        ask3.mode = "yesno";
        ask3.onAnswer = answer => {
            if (answer === "yes")
                yes();
        };
        ask3.text = text;
        ask3.open();
    }

    // The name-clash question Yes = overwrite them, No = skip them,
    // Cancel = do nothing. `go(skipClashes)`.
    function resolveClashes(question, go) {
        if (question === "") {
            go(false);
            return;
        }
        ask3.mode = "yesnocancel";
        ask3.onAnswer = answer => {
            if (answer === "yes")
                go(false);
            else if (answer === "no")
                go(true);
        };
        ask3.text = question;
        ask3.open();
    }

    function rowsJson(rows) {
        return JSON.stringify(rows);
    }

    function twoPaneDir() {
        return twoPane && local ? local.currentDir() : "";
    }

    // ---- columns, two panes

    function applyColumns() {
        const shown = JSON.parse(files.shownColumns());
        remote.showPermissions = shown.includes("permissions");
        remote.showOwner = shown.includes("owner");
    }

    function setColumn(key, on) {
        files.setColumn(key, on); // kept in launch.files_columns, saved with it
        applyColumns();
    }

    function setTwoPane(on) {
        twoPane = on;
        if (twoPaneTool.checked !== on)
            twoPaneTool.checked = on;
        files.setTwoPane(session.twoPaneKey, on);
    }

    function followDirectory(path) {
        noteTerminalDir(path);
        files.followDirectory(path, followTool.checked);
    }

    // The terminal reported `path` (each prompt reports it, changed or not): a different folder
    // ends a pause.
    function noteTerminalDir(path) {
        const changed = path !== terminalDir;
        terminalDir = path;
        if (followPaused && changed) {
            followPaused = false;
            followTool.checked = true;
        }
    }

    // The user went to another folder in Files: stop following until asked again.
    function pauseFollow() {
        if (!followTool.checked)
            return;
        followPaused = true;
        followTool.checked = false;
    }

    // The Follow button (or the local pane's copy of it) clicked: on goes to the terminal's
    // folder at once, off stops following until clicked again.
    function setFollow(on) {
        if (followTool.checked !== on)
            followTool.checked = on;
        if (on)
            resumeFollow();
        else
            followPaused = false;
    }

    function resumeFollow() {
        followPaused = false;
        if (localMode)
            localFollowRequested();
        else if (terminalDir !== "")
            files.followDirectory(terminalDir, true);
    }

    // ---- flows

    function activate(row) {
        if (files.activate(row) === "download")
            downloadEntry(row);
    }

    function downloadSelected() {
        const rows = remote.entryRows();
        if (rows.length === 0) {
            inform(qsTr("Select remote files or folders first."));
            return;
        }
        if (twoPaneDir() !== "")
            downloadInto(rows, twoPaneDir()); // Straight into the local pane
        else if (rows.length === 1 && !remote.info(rows[0]).isDir)
            downloadEntry(rows[0]);
        else
            downloadMany(rows);
    }

    function downloadEntry(row) {
        if (!files.hasTarget)
            return;
        const name = remote.info(row).name;
        saveFileDialog.row = row;
        saveFileDialog.title = qsTr("Download %1").arg(name);
        saveFileDialog.currentFile = pathToUrl(helper.homeDir() + "/" + name);
        saveFileDialog.open();
    }

    // Several items and/or folders → pick a local folder; folders go recursively.
    function downloadMany(rows) {
        if (!files.hasTarget || rows.length === 0)
            return;
        folderDialog.rows = rows;
        folderDialog.title = qsTr("Download %1 item(s) to folder").arg(rows.length);
        folderDialog.currentFolder = pathToUrl(helper.homeDir());
        folderDialog.mode = "download";
        folderDialog.open();
    }

    function downloadInto(rows, target) {
        const json = rowsJson(rows);
        resolveClashes(files.downloadClashes(json, target), skip => files.downloadInto(json, target, skip));
    }

    function uploadFiles() {
        if (!files.hasTarget || files.cwd === "")
            return;
        openDialog.open();
    }

    function uploadFolder() {
        if (!files.hasTarget || files.cwd === "")
            return;
        folderDialog.title = qsTr("Upload folder to remote host");
        folderDialog.currentFolder = pathToUrl(helper.homeDir());
        folderDialog.mode = "upload";
        folderDialog.open();
    }

    // Local files / folders into the folder of `row` (-1: the current one).
    // An upload into another folder looked at it first; same question as for this one.
    Connections {
        target: filesRoot.files
        function onUploadClashesFound(question) {
            filesRoot.resolveClashes(question, skip => filesRoot.files.uploadChecked(skip));
        }
        // The server copy of an edited file changed since it was opened.
        function onEditConflict(question, local) {
            filesRoot.confirm(question, () => filesRoot.files.editOverwrite(local));
        }
    }

    function uploadPaths(paths, row) {
        if (paths.length === 0)
            return;
        const json = JSON.stringify(paths);
        resolveClashes(files.uploadClashes(json, row), skip => files.uploadPaths(json, row, skip));
    }

    // The name prompt of the local pane's menu (New folder, Rename).
    function askText(title, label, text, done) {
        promptDialog.ask(title, label, text, null, 0, done);
    }

    function mkdir() {
        if (localMode && local) {
            local.makeFolder();
            return;
        }
        if (!files.hasTarget || files.busy || files.cwd === "")
            return;
        promptDialog.ask(qsTr("New remote folder"), qsTr("Folder name:"), "", null, 0, text => files.mkdir(text));
    }

    function rename(row) {
        if (!files.hasTarget || files.busy)
            return;
        const name = remote.info(row).name;
        promptDialog.ask(qsTr("Rename"), qsTr("New name:"), name, null, 0, text => files.rename(row, text));
    }

    function renameSelected() {
        if (localMode && local) {
            if (local.selection.length === 1)
                local.renameEntry(local.selection[0]);
            else
                inform(qsTr("Select one local file or folder first."));
            return;
        }
        const row = remote.currentRow;
        if (row < 0 || remote.info(row).kind !== "entry") {
            inform(qsTr("Select a remote file or folder first."));
            return;
        }
        rename(row);
    }

    function deleteRows(rows) {
        if (!files.hasTarget || files.busy || rows.length === 0)
            return;
        const json = rowsJson(rows);
        confirm(files.deleteQuestion(json), () => files.deleteRows(json));
    }

    function deleteSelected() {
        if (localMode && local) {
            if (local.selection.length > 0)
                local.deleteEntries(local.selection.slice());
            else
                inform(qsTr("Select a local file or folder first."));
            return;
        }
        const rows = remote.entryRows();
        if (rows.length === 0) {
            inform(qsTr("Select a remote file or folder first."));
            return;
        }
        deleteRows(rows);
    }

    function permissions(rows) {
        if (!files.hasTarget || files.busy || rows.length === 0)
            return;
        const json = rowsJson(rows);
        const i = JSON.parse(files.permissionsInfo(json));
        permDialog.ask(i.what, i.mode, i.anyDir, (mode, recursive) => files.chmod(json, mode, recursive));
    }

    // Remote rows dropped on the local pane — only if they still belong to the host and folder
    // shown (never act on another host's listing).
    function downloadDropped(payload, target) {
        const r = JSON.parse(files.dropRows(payload));
        if (r.error) {
            files.status = r.error;
            return;
        }
        downloadInto(r.rows, target);
    }

    // Compare / sync a local folder with the current remote one.
    function synchronize(localDir) {
        if (!files.hasTarget || files.cwd === "")
            return null;
        const dlg = syncComponent.createObject(filesRoot, {
            "localDir": localDir || twoPaneDir(),
            "binding": files.bindingJson()
        });
        dlg.open();
        return dlg;
    }

    function fillBookmarks() {
        while (bookmarkMenu.count > 0)
            bookmarkMenu.takeItem(0).destroy();
        const s = JSON.parse(files.bookmarkState());
        const add = (text, fn, enabled) => {
            bookmarkMenu.addItem(menuItem.createObject(null, {
                "text": text,
                "run": fn,
                "enabled": enabled
            }));
        };
        add(qsTr("Bookmark this folder"), () => files.addBookmark(), s.canAdd);
        add(qsTr("Remove bookmark"), () => files.removeBookmark(), s.canRemove);
        if (s.marks.length > 0)
            bookmarkMenu.addItem(separator.createObject(null));
        for (const m of s.marks)
            add(m, () => files.navigate(m), true);
    }

    Component.onCompleted: {
        applyColumns();
        if (!files || !session)
            return;
        const on = files.twoPane(session.twoPaneKey);
        if (on)
            setTwoPane(true);
        files.setActive(visible);
    }

    // Initial properties arrive after `visible` may already have changed (lazy creation).
    onVisibleChanged: if (files) files.setActive(visible)

    FilesHelper {
        id: helper
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 6

        // Layout: an icon toolbar on top, the path bar below it, then the listing.
        // A local shell gets the same bar, without what needs a server (remoteOnly).
        Row {
            id: toolbarFlow

            objectName: "filesToolbar"
            Layout.fillWidth: true
            spacing: 2

            // The icons that do not fit hide behind the » button (its menu runs the same action).
            // One icon's width is measured once and kept: a hidden Files tab reports no width.
            property real toolWidth: 0
            property var tools: []
            readonly property var activeTools: tools.filter(t => !(t.remoteOnly && filesRoot.localMode))
            readonly property real firstWidth: children.length > 0 ? children[0].implicitWidth : 0
            readonly property int shown: {
                const n = activeTools.length;
                if (toolWidth <= 0 || width >= n * toolWidth + (n - 1) * spacing)
                    return n;
                return Math.max(0, Math.floor((width - moreButton.implicitWidth) / (toolWidth + spacing)));
            }

            onFirstWidthChanged: toolWidth = Math.max(toolWidth, firstWidth)
            function relayout() {
                tools.forEach(t => {
                    const i = activeTools.indexOf(t);
                    t.visible = i >= 0 && i < shown;
                });
            }

            onShownChanged: relayout()
            onActiveToolsChanged: relayout()
            Component.onCompleted: {
                tools = Array.from(children).filter(c => c !== moreButton);
                relayout();
            }

            component Tool: QQC2.ToolButton {
                property string tip
                // Hidden for a local shell's files.
                property bool remoteOnly: false

                display: QQC2.AbstractButton.IconOnly
                Accessible.name: tip
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: tip
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            }

            Tool {
                objectName: "toolDownload"
                remoteOnly: true
                icon.name: "download"
                tip: qsTr("Download the selected files / folders")
                onClicked: filesRoot.downloadSelected()
            }
            Tool {
                id: uploadButton

                objectName: "toolUpload"
                remoteOnly: true
                icon.name: "cloud-upload"
                tip: qsTr("Upload files into the current directory (press and hold: upload a folder)")
                onClicked: filesRoot.uploadFiles()
                // Click = upload files; press-and-hold opens the menu.
                onPressAndHold: uploadMenu.popup(uploadButton, 0, uploadButton.height)
            }
            Tool {
                objectName: "toolRefresh"
                icon.name: "view-refresh"
                tip: qsTr("Refresh current folder")
                onClicked: filesRoot.localMode ? filesRoot.local.refresh() : filesRoot.files.refresh()
            }
            Tool {
                objectName: "toolMkdir"
                icon.name: "folder-new"
                tip: filesRoot.localMode ? qsTr("Create a new folder") : qsTr("Create a new remote folder")
                onClicked: filesRoot.mkdir()
            }
            Tool {
                objectName: "toolRename"
                icon.name: "edit-rename"
                tip: qsTr("Rename selected")
                onClicked: filesRoot.renameSelected()
            }
            Tool {
                objectName: "toolDelete"
                icon.name: "edit-delete"
                tip: qsTr("Delete selected")
                onClicked: filesRoot.deleteSelected()
            }
            Tool {
                objectName: "toolSync"
                remoteOnly: true
                icon.name: "document-swap"
                tip: qsTr("Synchronize a local folder with this one (preview first, never deletes)")
                onClicked: filesRoot.synchronize("")
            }
            Tool {
                id: followTool

                objectName: "toolFollow"
                icon.name: "folder-sync"
                checkable: true
                checked: true
                onToggled: filesRoot.setFollow(checked)
                tip: filesRoot.followPaused ? filesRoot.followPausedTip : qsTr("Follow terminal folder — go where the session cd's to (bash, zsh, fish, sh: set up at login,\nsee Preferences → Terminal; by hand: terminal right-click → Enable folder tracking in this shell)")
            }
            Tool {
                id: hiddenButton

                objectName: "toolHidden"
                icon.name: "view-hidden"
                checkable: true
                checked: true
                tip: qsTr("Show hidden files (dotfiles)")
                onCheckedChanged: filesRoot.files.setHidden(checked)
            }
            Tool {
                id: twoPaneTool

                objectName: "toolTwoPane"
                remoteOnly: true
                icon.name: "view-split-left-right"
                checkable: true
                tip: qsTr("Show a local folder beside the remote one — drag between them")
                onCheckedChanged: filesRoot.setTwoPane(checked)
            }
            Tool {
                id: moreButton

                objectName: "toolMore"
                visible: toolbarFlow.shown < toolbarFlow.activeTools.length
                icon.name: "overflow-menu"
                tip: qsTr("More")
                onClicked: moreMenu.popup(moreButton, 0, moreButton.height)
            }

            QQC2.Menu {
                id: moreMenu

                Instantiator {
                    model: toolbarFlow.tools

                    QQC2.MenuItem {
                        required property var modelData

                        visible: !modelData.visible && toolbarFlow.activeTools.includes(modelData)
                        icon.name: modelData.icon.name
                        text: (modelData.checkable ? (modelData.checked ? "\u2611 " : "\u2610 ") : "") + modelData.tip.split("\n")[0]
                        onTriggered: modelData.checkable ? modelData.toggle() : modelData.clicked()
                    }
                    onObjectAdded: (i, o) => moreMenu.insertItem(i, o)
                    onObjectRemoved: (i, o) => moreMenu.removeItem(o)
                }
            }
        }

        // The remote half (path bar + list) and an optional local pane beside it.
        QQC2.SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            Loader {
                id: localPaneLoader

                QQC2.SplitView.preferredWidth: parent.width / 2
                QQC2.SplitView.minimumWidth: 120
                active: filesRoot.twoPane || filesRoot.localMode
                visible: active
                QQC2.SplitView.fillWidth: filesRoot.localMode
                sourceComponent: LocalPane {
                    startDir: filesRoot.files.localDir()
                    files: filesRoot.files
                    standalone: filesRoot.localMode
                    showHidden: !filesRoot.localMode || hiddenButton.checked
                    host: filesRoot
                    onWidthsStored: remote.loadWidths()
                    onUploadRequested: paths => filesRoot.uploadPaths(paths, -1)
                    onRemoteDropped: (payload, target) => filesRoot.downloadDropped(payload, target)
                    onDirChanged: path => filesRoot.files.setLocalDir(path)
                    onUserNavigated: {
                        if (filesRoot.localMode)
                            filesRoot.pauseFollow();
                    }
                }
            }

            ColumnLayout {
                visible: !filesRoot.localMode
                QQC2.SplitView.fillWidth: true
                QQC2.SplitView.minimumWidth: 40
                spacing: 6

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Tool {
                        objectName: "navUp"
                        icon.name: "go-up"
                        tip: qsTr("Parent directory")
                        onClicked: filesRoot.files.goUp()
                    }
                    Tool {
                        objectName: "navHome"
                        icon.name: "go-home"
                        tip: qsTr("Home directory")
                        onClicked: filesRoot.files.navigate(".")
                    }
                    QQC2.TextField {
                        id: pathField

                        objectName: "remotePath"
                        Layout.fillWidth: true
                        placeholderText: qsTr("Remote path…")
                        placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
                        text: filesRoot.files.cwd
                        onAccepted: filesRoot.files.navigate(text.trim() || ".")
                    }
                    Tool {
                        id: bookmarkButton

                        objectName: "navBookmarks"
                        icon.name: "bookmarks"
                        tip: qsTr("Bookmarked folders on this host")
                        onClicked: {
                            filesRoot.fillBookmarks();
                            bookmarkMenu.popup(bookmarkButton, 0, bookmarkButton.height);
                        }
                    }
                }

                RemoteList {
                    id: remote

                    onWidthsStored: if (filesRoot.local)
                        filesRoot.local.loadWidths()
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    panel: filesRoot
                    files: filesRoot.files
                }
            }
        }

        // Transfer queue — progress, speed, cancel; browsing stays available.
        TransfersView {
            id: transfersView

            Layout.fillWidth: true
            transfers: filesRoot.transfers
        }

        TextEdit {
            id: statusLabel

            objectName: "filesStatus"
            visible: !filesRoot.statusInFooter
            Layout.fillWidth: true
            readOnly: true
            selectByMouse: true
            wrapMode: Text.Wrap
            color: Kirigami.Theme.textColor
            selectedTextColor: Kirigami.Theme.highlightedTextColor
            selectionColor: Kirigami.Theme.highlightColor
            font: Kirigami.Theme.defaultFont
            text: filesRoot.statusText
        }

        // The password was typed in the terminal and is not saved: ask for it here (kept in memory
        // for this session). Meanwhile the panel retries, since the terminal's own login usually
        // makes a connection Files can share within moments (not on Windows: no shared connection).
        RowLayout {
            Layout.fillWidth: true
            visible: filesRoot.files.authRequired

            QQC2.Button {
                objectName: "filesPasswordButton"
                text: qsTr("Enter password…")
                icon.name: "dialog-password"
                onClicked: filesRoot.askPassword()
            }
        }
    }

    property int authRetries: 0

    function askPassword() {
        promptDialog.ask(qsTr("Password"), qsTr("Password for %1:").arg(files.label), "", null, 0, pw => files.submitPassword(pw), true);
    }

    Timer {
        interval: 2000
        repeat: true
        running: filesRoot.files.authRequired && filesRoot.authRetries < 30 && Qt.platform.os !== "windows"
        onTriggered: {
            filesRoot.authRetries++;
            filesRoot.files.refresh();
        }
    }

    Connections {
        target: filesRoot.files

        function onNavigatedByUser() {
            filesRoot.pauseFollow();
        }

        function onAuthRequiredChanged() {
            if (!filesRoot.files.authRequired)
                filesRoot.authRetries = 0;
        }
    }

    // QML's way to the clipboard (QGuiApplication.clipboard().setText()).
    TextEdit {
        id: clipboard

        visible: false
    }

    QQC2.Menu {
        id: uploadMenu

        QQC2.MenuItem {
            text: qsTr("Upload files…")
            onTriggered: filesRoot.uploadFiles()
        }
        QQC2.MenuItem {
            text: qsTr("Upload folder…")
            onTriggered: filesRoot.uploadFolder()
        }
    }

    QQC2.Menu {
        id: bookmarkMenu
    }

    MessageBox {
        id: box

        title: "Termigator"
        buttons: MessageDialog.Ok
    }

    // Yes / No (/ Cancel); `onAnswer("yes" | "no")`.
    MessageBox {
        id: ask3

        property string mode: "yesno"
        property var onAnswer: null

        title: "Termigator"
        buttons: mode === "yesnocancel" ? (MessageDialog.Yes | MessageDialog.No | MessageDialog.Cancel) : (MessageDialog.Yes | MessageDialog.No)
        onButtonClicked: (button, role) => {
            const fn = onAnswer;
            onAnswer = null;
            if (fn)
                fn(button === MessageDialog.Yes ? "yes" : (button === MessageDialog.No ? "no" : "cancel"));
        }
    }

    PromptDialog {
        id: promptDialog
    }

    PermissionsDialog {
        id: permDialog
    }

    FileDialog {
        id: openDialog

        title: qsTr("Upload files to remote host")
        fileMode: FileDialog.OpenFiles
        onAccepted: filesRoot.uploadPaths(selectedFiles.map(u => filesRoot.urlToPath(u)), -1)
    }

    FileDialog {
        id: saveFileDialog

        property int row: -1

        fileMode: FileDialog.SaveFile
        onAccepted: filesRoot.files.downloadEntry(row, filesRoot.urlToPath(selectedFile))
    }

    FolderDialog {
        id: folderDialog

        property var rows: []
        property string mode: "download"

        onAccepted: {
            const dir = filesRoot.urlToPath(selectedFolder).replace(/\/+$/, "");
            if (mode === "upload")
                filesRoot.uploadPaths([dir], -1);
            else
                filesRoot.downloadInto(rows, dir);
        }
    }

    Component {
        id: syncComponent

        SyncDialog {
            filesPanel: filesRoot
            onClosed: destroy()
        }
    }

    Component {
        id: menuItem

        QQC2.MenuItem {
            property var run: null

            onTriggered: {
                if (run)
                    run();
            }
        }
    }

    Component {
        id: separator

        QQC2.MenuSeparator {}
    }
}
