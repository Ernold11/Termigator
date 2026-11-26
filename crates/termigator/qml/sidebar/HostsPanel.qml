import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// The Hosts side tab: search, Add ▾, the local-shell button, the host tree, its context menus, drag
// & drop and keys. The rows and every config change come from HostListModel (sidebar_qobj.rs);
// this file saves with controller.persist() and opens tabs / editors through the shell's hooks
// (a hook that is still null disables what needs it).
FocusScope {
    id: panel

    required property var shell
    readonly property alias hosts: hostModel
    readonly property alias list: listView
    readonly property alias search: searchField
    readonly property alias messageBox: box
    readonly property alias saveQuestion: saveQuestionBox
    readonly property alias prompt: promptDialog
    readonly property alias deleteFolderDialog: deleteFolder
    readonly property alias testDialog: connectionTestDialog
    // Selected rows (sorted) and the current row; both reset with the model.
    property var selection: []
    property int currentRow: -1
    property int anchorRow: -1
    // The row whose name is edited in place (F2), -1 for none.
    property int editingRow: -1
    // Bumped when the tree's clipboard changes, so the rows read `isCut` again.
    property int clipVersion: 0
    // Settings → Appearance: one line per host (default) or two (user@host under the name).
    property bool detailed: shell.controller.windowBool("host_list_detailed", false)
    property QQC2.Menu contextMenu: null
    property QQC2.Menu addMenu: null
    property QQC2.Menu shellMenu: null
    readonly property var tools: JSON.parse(hostModel.toolsJson())
    readonly property var iconChoices: JSON.parse(hostModel.iconChoicesJson())

    objectName: "hostsPanel"

    Connections {
        target: panel.shell.controller

        function onConfigChanged() {
            panel.detailed = panel.shell.controller.windowBool("host_list_detailed", false);
        }
    }

    // ---- rows and selection ------------------------------------------------------------

    function info(row) {
        const text = row >= 0 ? hostModel.rowJson(row) : "";
        return text === "" ? null : JSON.parse(text);
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

    function selectAll() {
        selectRange(0, listView.count - 1);
    }

    function toggle(row) {
        const i = info(row);
        if (!i || (i.kind !== "group" && i.kind !== "recent"))
            return;
        const kept = hostModel.setExpanded(row, !i.expanded, [currentRow].concat(selection));
        currentRow = kept[0];
        selection = kept.slice(1).filter(r => r >= 0);
    }

    function reveal(kind, index) {
        setCurrent(hostModel.reveal(kind, index), true);
    }

    // ---- outcomes, boxes and prompts ---------------------------------------------------

    // A mutation's outcome (sidebar_qobj.rs `Outcome`): save, report, select.
    function finish(json, title) {
        const o = JSON.parse(json);
        if (o.changed)
            shell.controller.persist();
        if (o.copy !== null) {
            clipboard.text = o.copy;
            clipboard.selectAll();
            clipboard.copy();
        }
        if (o.notice)
            shell.notify(o.notice);
        if (o.select)
            reveal(o.select[0], o.select[1]);
        else if (o.folder)
            setCurrent(hostModel.groupRow(o.folder), true);
        if (o.warning)
            inform(o.warning, title, "warning");
        else if (o.info)
            inform(o.info, title);
    }

    function inform(text, title, icon) {
        box.onYes = null;
        box.title = title || "Termigator";
        box.buttons = MessageDialog.Ok;
        box.icon = icon || "information";
        box.text = text;
        box.open();
    }

    function ask(text, onYes) {
        box.onYes = onYes;
        box.title = "Termigator";
        box.buttons = MessageDialog.Yes | MessageDialog.No;
        box.icon = "auto";
        box.text = text;
        box.open();
    }

    // ---- flows -----------------------------------------------

    function focusFilter() {
        searchField.forceActiveFocus(Qt.ShortcutFocusReason);
        searchField.selectAll();
    }

    function openEntry(i) {
        if (shell.openConnection)
            shell.openConnection(hostModel.openJson(i.kind, i.index)); // with the folder's environment
    }

    function activate(row) {
        const i = info(row);
        if (!i)
            return;
        if (i.kind === "group" || i.kind === "recent")
            toggle(row);
        else
            openEntry(i);
    }

    // The editor's OK: "" once stored, else the reason, which the editor shows in place.
    function store(json) {
        const o = JSON.parse(json);
        if (o.warning || o.info)
            return o.warning || o.info;
        finish(json);
        return "";
    }

    function addEntry(kind, group) {
        if (shell.openEditor)
            shell.openEditor(kind, "", group || "", data => store(hostModel.storeEntry(kind, "", -1, data)));
    }

    function editEntry(i) {
        const kind = i.kind === "favorite" ? "favorite" : i.protocol;
        if (shell.openEditor)
            shell.openEditor(kind, hostModel.entryJson(i.kind, i.index), "", data => store(hostModel.storeEntry(kind, i.kind, i.index, data)));
    }

    // "Add to Hosts…" on a tab (shell.saveToHosts): the editor with the tab's connection, without
    // its secrets. One that is saved already (same name, or same protocol, host, port and user)
    // asks first: Yes edits that entry (it keeps its own password), No adds a new one.
    function saveFromTab(conn) {
        if (!shell.openEditor)
            return;
        const p = JSON.parse(hostModel.tabSavePlan(JSON.stringify(conn)));
        const addNew = () => shell.openEditor(p.kind, JSON.stringify(p.entry), p.entry.group || "", data => store(hostModel.storeEntry(p.kind, "", -1, data)));
        if (p.existing < 0) {
            addNew();
            return;
        }
        saveQuestionBox.onYes = () => shell.openEditor(p.mergedKind, JSON.stringify(p.merged), "", data => store(hostModel.storeEntry(p.mergedKind, "connection", p.existing, data)));
        saveQuestionBox.onNo = addNew;
        saveQuestionBox.text = qsTr("“%1” is already in Hosts.\n\nYes updates that entry, No adds a new one.").arg(p.existingLabel);
        saveQuestionBox.open();
    }

    function duplicate(i) {
        finish(hostModel.duplicate(i.kind, i.index));
    }

    function moveBy(i, delta) {
        finish(hostModel.moveBy(i.kind, i.index, delta));
    }

    function moveToGroup(i) {
        const choices = ["(top level)"].concat(hostModel.groupNames());
        const at = i.group ? choices.indexOf(i.group) : 0;
        promptDialog.ask(qsTr("Move to folder"), qsTr("Folder (type a new name to create it; use / to nest, e.g. prod/web):"), "", choices, Math.max(at, 0), name => finish(hostModel.moveToGroup(i.kind, i.index, name)));
    }

    // After a delete the row that took the deleted one's place (else the last row) is current.
    function keepRow(row) {
        if (currentRow < 0 && row >= 0 && listView.count > 0)
            setCurrent(Math.min(row, listView.count - 1), true);
        listView.forceActiveFocus();
    }

    function removeEntry(i) {
        const at = currentRow;
        ask(qsTr("Remove “%1” from Hosts?\n\nOnly this entry is removed.").arg(i.label), () => {
            finish(hostModel.removeEntry(i.kind, i.index));
            keepRow(at);
        });
    }

    function addGroup(parentPath) {
        const where = parentPath ? qsTr(" inside “%1”").arg(parentPath) : "";
        promptDialog.ask(qsTr("New folder"), qsTr("Folder name%1\n(use / to nest deeper):").arg(where), "", null, 0, name => {
            finish(hostModel.addGroup(parentPath, name));
            listView.forceActiveFocus();
        });
    }

    // F2 / Rename: the name in place in the row, for entries and folders (not Recent).
    function startRename(row) {
        const i = info(row);
        if (!i || i.kind === "recent")
            return;
        setCurrent(row, true);
        editingRow = row;
    }

    // The field's Enter, or leaving it (`blur`: the focus went elsewhere and stays there):
    // "" once renamed or unchanged, else the reason and the field stays.
    function commitRename(row, text, blur) {
        const i = info(row);
        if (!i || editingRow !== row)
            return "";
        const json = i.kind === "group" ? hostModel.renameFolder(i.path, text) : hostModel.renameEntry(i.kind, i.index, text);
        const o = JSON.parse(json);
        if (o.warning)
            return o.warning;
        editingRow = -1;
        finish(json);
        if (!blur)
            listView.forceActiveFocus();
        return "";
    }

    function cancelRename(blur) {
        editingRow = -1;
        if (!blur)
            listView.forceActiveFocus();
    }

    // Into another folder (not itself or below it); a typed path that does not exist is created.
    function moveFolder(path) {
        const parentPath = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
        const choices = ["(top level)"].concat(hostModel.folderMoveTargets(path));
        const at = parentPath ? choices.indexOf(parentPath) : 0;
        promptDialog.ask(qsTr("Move folder"), qsTr("Move “%1” with its contents into:").arg(path), "", choices, Math.max(at, 0), dest => finish(hostModel.moveFolder(path, dest)));
    }

    // An empty folder asks Yes / No; one with entries or subfolders asks where they go (into
    // the parent unless another choice is made).
    function deleteGroup(path) {
        const at = currentRow;
        const done = json => {
            finish(json);
            keepRow(at);
        };
        const c = JSON.parse(hostModel.folderContents(path));
        if (c.question === null) {
            ask(qsTr("Delete the empty folder “%1”?").arg(path), () => done(hostModel.deleteGroup(path)));
            return;
        }
        deleteFolder.ask(path, c.question, c.parent, hostModel.folderMoveTargets(path), (mode, dest) => done(mode === "delete" ? hostModel.deleteGroup(path) : hostModel.deleteFolderInto(path, dest)));
    }

    // Ctrl+C / Ctrl+X: the selection when the current row is in it, else the current row.
    function copyRows(cut) {
        const rows = selection.includes(currentRow) ? selection : currentRow >= 0 ? [currentRow] : [];
        if (rows.length === 0)
            return;
        finish(hostModel.copyRows(rows, cut));
        ++clipVersion;
    }

    // Ctrl+V: into the folder of `row` (the folder itself, an entry's folder; -1 = top level).
    function pasteAt(row) {
        finish(hostModel.pasteAt(row), qsTr("Paste"));
        ++clipVersion;
        listView.forceActiveFocus();
    }

    // Where New folder / New connection go from the list: the folder of `row`.
    function folderAt(row) {
        const i = info(row);
        if (!i || i.kind === "recent")
            return "";
        return i.kind === "group" ? i.path : i.group;
    }

    function connectMany(indices) {
        for (const i of indices)
            openEntry({ kind: "connection", index: i });
    }

    function bulkApply(indices, changes) {
        finish(hostModel.bulkApply(indices, JSON.stringify(changes)));
    }

    // Bulk edit: the tick-to-change dialog, then the same apply as the icon menu.
    function bulkEdit(indices) {
        shell.openBulkEdit(indices.length, hostModel.groupNames(), changes => bulkApply(indices, changes));
    }

    function bulkMove(indices) {
        const choices = ["(top level)"].concat(hostModel.groupNames());
        promptDialog.ask(qsTr("Move to folder"), qsTr("Folder for %1 entries (type a new name to create it):").arg(indices.length), "", choices, 0, name => bulkApply(indices, {
            "group": name === "(top level)" ? "" : name
        }));
    }

    function bulkDelete(indices) {
        const at = currentRow;
        ask(qsTr("Remove %1 entries from config?").arg(indices.length), () => {
            finish(hostModel.bulkDelete(indices));
            keepRow(at);
        });
    }

    function copyCommand(index) {
        finish(hostModel.copyCommand(index));
    }

    function copyAddress(index) {
        finish(hostModel.copyAddress(index));
    }

    function testConnection(index) {
        connectionTestDialog.test(hostModel.entryJson("connection", index));
    }

    function runNettool(key, index) {
        const o = JSON.parse(hostModel.nettool(index, key));
        if (o.warning)
            inform(o.warning, qsTr("Network tools"), "warning");
        else if (o.cmd && shell.openInner)
            shell.openInner(o.cmd, o.title, "embedded", "");
    }

    function openShell(path) {
        if (shell.newLocalShell)
            shell.newLocalShell(path);
    }

    // The menu-bar actions that act on the list's current row.
    function copyFromSelection() {
        const i = info(currentRow);
        if (!i)
            inform(qsTr("Select a connection in the left list."));
        else if (i.kind !== "connection")
            inform(qsTr("Copy command applies to SSH/RDP connections only (not favorites)."));
        else
            copyCommand(i.index);
    }

    function testFromSelection() {
        const i = info(currentRow);
        if (!i || i.kind !== "connection")
            inform(qsTr("Select an SSH / RDP / VNC connection in the left list."));
        else
            testConnection(i.index);
    }

    function nettoolFromSelection(key) {
        const i = info(currentRow);
        if (!i || i.kind !== "connection" || i.protocol === "serial")
            inform(qsTr("Select a host (SSH / RDP / VNC / Telnet) in the left list."), qsTr("Network tools"));
        else
            runNettool(key, i.index);
    }

    // ---- keys (context hostList) ----------------------------------------------------

    function deleteKey() {
        const multi = hostModel.selectedConnections(selection);
        if (multi.length > 1) {
            bulkDelete(multi);
            return;
        }
        const i = info(currentRow);
        if (!i)
            return;
        if (i.kind === "group")
            deleteGroup(i.path);
        else if (i.kind === "connection" || i.kind === "favorite")
            removeEntry(i);
    }

    // Esc: drop a pending cut, else the selection (the current row stays); false: nothing to do.
    function cancelKey() {
        if (hostModel.cancelCut()) {
            ++clipVersion;
            shell.notify(qsTr("Cut cancelled."));
            return true;
        }
        if (selection.length === 0)
            return false;
        selection = [];
        return true;
    }

    // The folder row above `row`, -1 at the top level.
    function parentRow(row) {
        const i = info(row);
        for (let r = row - 1; i && r >= 0; --r) {
            if (info(r).depth < i.depth)
                return r;
        }
        return -1;
    }

    // QTreeView: Left collapses, else goes to the parent; Right expands, else to the first child.
    function sideKey(right) {
        const i = info(currentRow);
        if (!i)
            return;
        const folder = i.kind === "group" || i.kind === "recent";
        if (folder && i.expanded !== right)
            toggle(currentRow);
        else if (!right && parentRow(currentRow) >= 0)
            setCurrent(parentRow(currentRow), true);
        else if (right && folder) {
            const next = info(currentRow + 1);
            if (next && next.depth > i.depth)
                setCurrent(currentRow + 1, true);
        }
    }

    function moveKey(delta) {
        const i = info(currentRow);
        if (i && (i.kind === "connection" || i.kind === "favorite"))
            moveBy(i, delta);
    }

    // Menu key: at the current row, else at the viewport's top-left corner.
    function menuKey() {
        let row = currentRow;
        if (row < 0)
            row = listView.indexAt(0, listView.contentY);
        const item = row >= 0 ? listView.itemAtIndex(row) : null;
        const p = item ? item.mapToItem(listView, item.width / 2, item.height / 2) : Qt.point(0, 0);
        openContextMenu(row, p.x, p.y);
    }

    function navigate(row, modifiers) {
        if (listView.count === 0)
            return;
        row = Math.max(0, Math.min(listView.count - 1, row));
        if (modifiers & Qt.ShiftModifier) {
            currentRow = row;
            selectRange(anchorRow >= 0 ? anchorRow : row, row);
            listView.positionViewAtIndex(row, ListView.Contain);
        } else {
            setCurrent(row, !(modifiers & Qt.ControlModifier));
        }
    }

    // Only for the list itself: a key the rename field (or anything else inside) leaves alone
    // must not delete, rename or move rows.
    function listKey(event) {
        if (listView.Window.activeFocusItem !== listView) {
            event.accepted = false;
            return;
        }
        const mods = event.modifiers & (Qt.ShiftModifier | Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier);
        const key = event.key;
        const page = Math.max(1, Math.floor(listView.height / 48));
        if ((key === Qt.Key_Return || key === Qt.Key_Enter) && mods === 0)
            activate(currentRow); // hostlist.connect
        else if (key === Qt.Key_Up && mods === Qt.AltModifier)
            moveKey(-1); // hostlist.move_up
        else if (key === Qt.Key_Down && mods === Qt.AltModifier)
            moveKey(1); // hostlist.move_down
        else if (key === Qt.Key_Delete && mods === 0)
            deleteKey(); // hostlist.delete
        else if (key === Qt.Key_F2 && mods === 0)
            startRename(currentRow); // hostlist.rename
        else if (key === Qt.Key_Space && mods === 0) {
            const i = info(currentRow); // hostlist.toggle
            if (i && (i.kind === "group" || i.kind === "recent"))
                toggle(currentRow);
        } else if (key === Qt.Key_C && mods === Qt.ControlModifier)
            copyRows(false); // hostlist.copy
        else if (key === Qt.Key_X && mods === Qt.ControlModifier)
            copyRows(true); // hostlist.cut
        else if (key === Qt.Key_V && mods === Qt.ControlModifier)
            pasteAt(currentRow); // hostlist.paste
        else if (key === Qt.Key_N && mods === Qt.ControlModifier)
            addGroup(folderAt(currentRow)); // hostlist.new_folder
        else if (key === Qt.Key_Escape && mods === 0) {
            if (!cancelKey()) { // hostlist.cancel
                event.accepted = false;
                return;
            }
        }
        else if ((key === Qt.Key_F || key === Qt.Key_L) && mods === Qt.ControlModifier)
            focusFilter(); // hostlist.focus_search
        else if ((key === Qt.Key_F10 && mods === Qt.ShiftModifier) || (key === Qt.Key_Menu && mods === 0))
            menuKey(); // hostlist.context_menu
        else if (key === Qt.Key_A && mods === Qt.ControlModifier)
            selectAll(); // hostlist.select_all
        else if (key === Qt.Key_Up && (mods & ~(Qt.ShiftModifier | Qt.ControlModifier)) === 0)
            navigate(currentRow < 0 ? 0 : currentRow - 1, mods);
        else if (key === Qt.Key_Down && (mods & ~(Qt.ShiftModifier | Qt.ControlModifier)) === 0)
            navigate(currentRow + 1, mods);
        else if (key === Qt.Key_Home)
            navigate(0, mods);
        else if (key === Qt.Key_End)
            navigate(listView.count - 1, mods);
        else if (key === Qt.Key_PageUp)
            navigate(currentRow - page, mods);
        else if (key === Qt.Key_PageDown)
            navigate(currentRow + page, mods);
        else if ((key === Qt.Key_Left || key === Qt.Key_Right) && mods === 0)
            sideKey(key === Qt.Key_Right);
        else {
            event.accepted = false;
            return;
        }
        event.accepted = true;
    }

    // ---- menus -------------------------------------------------------------------------

    function newMenu(title) {
        const m = menuComponent.createObject(panel, {
            "title": title || ""
        });
        if (title)
            m.icon.name = hostModel.menuIcon(title);
        return m;
    }

    function add(menu, text, fn, enabled, iconName, iconColor) {
        const item = itemComponent.createObject(null, {
            "text": text,
            "run": fn,
            "enabled": enabled === undefined ? true : enabled
        });
        item.icon.name = iconName === undefined ? hostModel.menuIcon(text) : iconName;
        if (iconColor)
            item.icon.color = iconColor;
        menu.addItem(item);
        return item;
    }

    // A menu entry with a filled dot in the category's colour. Icon tinting depends on the icon
    // theme (Breeze's media-record is a fixed red picture), so draw the dot ourselves.
    function addDot(menu, text, fn, color) {
        const item = add(menu, text, fn, true, "");
        const content = envItemContentComponent.createObject(item, {
            "label": text,
            "dotColor": color,
            "menuItem": item
        });
        item.contentItem = content;
        return item;
    }

    // The key of an entry beside its text (KeyedMenuContent).
    function keyed(item, keys) {
        item.shortcutText = keys;
        item.contentItem = keyedContentComponent.createObject(item, {
            "menuItem": item
        });
        return item;
    }

    // Cut / Copy / Paste of the row's menu (the same calls as Ctrl+X / Ctrl+C / Ctrl+V).
    function clipItems(menu, row) {
        keyed(add(menu, qsTr("Cut"), () => copyRows(true)), "Ctrl+X");
        keyed(add(menu, qsTr("Copy"), () => copyRows(false)), "Ctrl+C");
        keyed(add(menu, qsTr("Paste"), () => pasteAt(row), hostModel.hasClip()), "Ctrl+V");
    }

    function sep(menu) {
        menu.addItem(separatorComponent.createObject(null));
    }

    function sub(menu, title) {
        const s = newMenu(title);
        menu.addMenu(s);
        return s;
    }

    // Read on each menu build so a category edited in Settings shows up without a restart.
    function envCategories() {
        return JSON.parse(hostModel.environmentsJson());
    }

    function envName(id) {
        const e = envCategories().find(c => c.id.toLowerCase() === String(id).toLowerCase());
        return e ? e.name : id;
    }

    // Environment ▸ for a folder; entries inside without their own inherit it.
    function envMenu(menu, path) {
        const st = JSON.parse(hostModel.groupEnvironment(path));
        const s = sub(menu, qsTr("Label"));
        const inherited = st.inherited !== "" ? envName(st.inherited) : "";
        const inheritText = st.own === "" && inherited !== "" ? qsTr("Inherit from parent (%1)").arg(inherited) : qsTr("Inherit from parent folder (none)");
        const inherit = add(s, inheritText, () => finish(hostModel.setGroupEnvironment(path, "")), true, "");
        inherit.checkable = true;
        inherit.checked = st.own === "";
        for (const en of envCategories()) {
            const item = addDot(s, en.name, () => finish(hostModel.setGroupEnvironment(path, en.id)), en.color);
            item.checkable = true;
            item.checked = st.own.toLowerCase() === en.id.toLowerCase();
        }
    }

    // The same menu for one entry; "" means "take the folder's".
    function entryEnvMenu(menu, i) {
        const st = JSON.parse(hostModel.entryEnvironment(i.kind, i.index));
        const s = sub(menu, qsTr("Label"));
        const inherited = st.inherited !== "" ? envName(st.inherited) : "";
        const inheritText = inherited !== "" ? qsTr("Inherit from folder (%1)").arg(inherited) : qsTr("Inherit from folder (none)");
        const inherit = add(s, inheritText, () => finish(hostModel.setEntryEnvironment(i.kind, i.index, "")), true, "");
        inherit.checkable = true;
        inherit.checked = st.own === "";
        for (const en of envCategories()) {
            const item = addDot(s, en.name, () => finish(hostModel.setEntryEnvironment(i.kind, i.index, en.id)), en.color);
            item.checkable = true;
            item.checked = st.own.toLowerCase() === en.id.toLowerCase();
        }
    }

    function iconMenu(menu, indices) {
        const s = sub(menu, qsTr("Set icon"));
        add(s, qsTr("Default (by protocol / distro)"), () => bulkApply(indices, {
            "icon": ""
        }));
        sep(s);
        for (const c of iconChoices)
            add(s, c[0], () => bulkApply(indices, {
                "icon": c[1]
            }), true, c[2]);
    }

    function fillMulti(menu, indices) {
        const n = indices.length;
        add(menu, qsTr("Connect %1 entries").arg(n), () => connectMany(indices), shell.openConnection !== null);
        sep(menu);
        add(menu, qsTr("Edit %1 entries…").arg(n), () => bulkEdit(indices), shell.openBulkEdit !== null);
        add(menu, qsTr("Move %1 entries to folder…").arg(n), () => bulkMove(indices));
        iconMenu(menu, indices);
        sep(menu);
        keyed(add(menu, qsTr("Cut"), () => copyRows(true)), "Ctrl+X");
        keyed(add(menu, qsTr("Copy"), () => copyRows(false)), "Ctrl+C");
        sep(menu);
        keyed(add(menu, qsTr("Delete %1 entries…").arg(n), () => bulkDelete(indices)), "Del");
    }

    function copyLabel(protocol) {
        switch (protocol) {
        case "rdp":
            return qsTr("Copy RDP command");
        case "vnc":
            return qsTr("Copy VNC command");
        case "telnet":
            return qsTr("Copy Telnet command");
        case "serial":
            return qsTr("Copy serial command");
        }
        return qsTr("Copy SSH command");
    }

    // The list's context menu.
    function buildContextMenu(row) {
        const m = newMenu("");
        const i = info(row);
        const editor = shell.openEditor !== null;
        const toggleText = i && i.expanded ? qsTr("Collapse") : qsTr("Expand");
        if (i && i.kind === "recent") {
            keyed(add(m, toggleText, () => toggle(row)), "Space");
            return m;
        }
        const multi = hostModel.selectedConnections(selection);
        if (i && multi.length > 1 && selection.includes(row)) {
            fillMulti(m, multi);
            return m;
        }
        if (!i) {
            keyed(addKind(m, qsTr("Add SSH connection…"), "ssh", undefined, editor), "Ctrl+Shift+N");
            addKind(m, qsTr("Add RDP connection…"), "rdp", undefined, editor);
            addKind(m, qsTr("Add VNC connection…"), "vnc", undefined, editor);
            addKind(m, qsTr("Add Telnet connection…"), "telnet", undefined, editor);
            addKind(m, qsTr("Add serial connection…"), "serial", undefined, editor);
            addKind(m, qsTr("Add favorite command…"), "favorite", undefined, editor);
            sep(m);
            keyed(add(m, qsTr("New folder…"), () => addGroup("")), "Ctrl+N");
            keyed(add(m, qsTr("Paste"), () => pasteAt(-1), hostModel.hasClip()), "Ctrl+V");
        } else if (i.kind === "group") {
            const g = i.path;
            keyed(add(m, toggleText, () => toggle(row)), "Space");
            sep(m);
            keyed(addKind(m, qsTr("Add SSH connection here…"), "ssh", g, editor), "Ctrl+Shift+N");
            addKind(m, qsTr("Add RDP connection here…"), "rdp", g, editor);
            addKind(m, qsTr("Add VNC connection here…"), "vnc", g, editor);
            addKind(m, qsTr("Add Telnet connection here…"), "telnet", g, editor);
            addKind(m, qsTr("Add serial connection here…"), "serial", g, editor);
            addKind(m, qsTr("Add favorite command here…"), "favorite", g, editor);
            sep(m);
            keyed(add(m, qsTr("New subfolder…"), () => addGroup(g)), "Ctrl+N");
            envMenu(m, g);
            keyed(add(m, qsTr("Rename folder"), () => startRename(row)), "F2");
            add(m, qsTr("Move folder to…"), () => moveFolder(g));
            sep(m);
            clipItems(m, row);
            sep(m);
            keyed(add(m, qsTr("Delete folder…"), () => deleteGroup(g)), "Del");
        } else {
            const conn = i.kind === "connection";
            keyed(add(m, qsTr("Connect / Run"), () => openEntry(i), shell.openConnection !== null), "Enter");
            if (conn) {
                add(m, qsTr("Test connection…"), () => testConnection(i.index));
                if (i.protocol !== "serial") {
                    const net = sub(m, qsTr("Network tools"));
                    for (const t of tools)
                        add(net, t[1], () => runNettool(t[0], i.index), shell.openInner !== null);
                }
                add(m, copyLabel(i.protocol), () => copyCommand(i.index));
                add(m, qsTr("Copy address"), () => copyAddress(i.index));
                if (i.protocol === "ssh") {
                    sep(m);
                    add(m, qsTr("Open files (SFTP)"), () => shell.openFilesTab(hostModel.entryJson("connection", i.index)), shell.openFilesTab !== null);
                    add(m, qsTr("Upload file to host…"), () => shell.scpTransfer(hostModel.entryJson("connection", i.index), true), shell.scpTransfer !== null);
                    add(m, qsTr("Download file from host…"), () => shell.scpTransfer(hostModel.entryJson("connection", i.index), false), shell.scpTransfer !== null);
                }
            }
            sep(m);
            add(m, qsTr("Edit…"), () => editEntry(i), editor);
            keyed(add(m, qsTr("Rename"), () => startRename(row)), "F2");
            add(m, qsTr("Duplicate entry"), () => duplicate(i));
            add(m, qsTr("Move to folder…"), () => moveToGroup(i));
            if (conn) {
                entryEnvMenu(m, i);
                iconMenu(m, [i.index]);
            }
            sep(m);
            clipItems(m, row);
            sep(m);
            keyed(add(m, qsTr("Move up"), () => moveBy(i, -1)), "Alt+Up");
            keyed(add(m, qsTr("Move down"), () => moveBy(i, 1)), "Alt+Down");
            sep(m);
            keyed(add(m, qsTr("Delete…"), () => removeEntry(i)), "Del");
        }
        return m;
    }

    function openContextMenu(row, x, y) {
        if (contextMenu)
            contextMenu.destroy();
        contextMenu = buildContextMenu(row);
        contextMenu.popup(listView, x, y);
    }

    function importAction(id) {
        return shell.shellActions.actions[id] ?? null;
    }

    // Kinds this machine cannot open (Windows: serial, VNC / Telnet without a client): greyed out
    // with a short reason, so no one builds an entry that only fails when connecting.
    function unavailableKinds() {
        return JSON.parse(hostModel.kindSupport());
    }

    function kindNote(kind) {
        return kind === "serial" ? qsTr("not on Windows yet") : kind === "vnc" ? qsTr("no VNC viewer") : kind === "telnet" ? qsTr("no telnet client") : qsTr("not available");
    }

    function addKind(menu, text, kind, group, editor) {
        const why = unavailableKinds()[kind];
        return add(menu, why ? text + " (" + kindNote(kind) + ")" : text, () => addEntry(kind, group), editor && !why);
    }

    function buildAddMenu() {
        const editor = shell.openEditor !== null;
        const m = newMenu("");
        addKind(m, qsTr("SSH connection…"), "ssh", undefined, editor);
        addKind(m, qsTr("RDP connection…"), "rdp", undefined, editor);
        addKind(m, qsTr("VNC connection…"), "vnc", undefined, editor);
        addKind(m, qsTr("Telnet connection…"), "telnet", undefined, editor);
        addKind(m, qsTr("Serial connection…"), "serial", undefined, editor);
        addKind(m, qsTr("Favorite command…"), "favorite", undefined, editor);
        add(m, qsTr("Folder…"), () => addGroup(""));
        sep(m);
        // The importers: the File → Import actions, live once something handles them.
        for (const [text, id] of [[qsTr("Import MobaXterm sessions…"), "file.import.mobaxterm"], [qsTr("Import PuTTY sessions…"), "file.import.putty"], [qsTr("Import SuperPuTTY sessions…"), "file.import.superputty"], [qsTr("Import ~/.ssh/config hosts…"), "file.import.ssh_config"], [qsTr("Import Remmina profiles…"), "file.import.remmina"]]) {
            const act = importAction(id);
            const linuxOnly = id === "file.import.remmina" && Qt.platform.os === "windows";
            add(m, linuxOnly ? text + " " + qsTr("(Linux only)") : text, () => act.trigger(), act !== null && act.enabled && !linuxOnly);
        }
        return m;
    }

    function popupShellMenu() {
        if (shellMenu)
            shellMenu.destroy();
        shellMenu = buildShellMenu();
        shellMenu.popup(shellButton, 0, shellButton.height);
    }

    function buildShellMenu() {
        const m = newMenu("");
        const live = shell.newLocalShell !== null;
        add(m, qsTr("Local shell"), () => openShell(""), live);
        // Linux: /etc/shells when there is a choice; Windows: cmd, PowerShell and WSL always, the
        // ones not installed here greyed out.
        const shells = JSON.parse(hostModel.shells());
        if (shells.length > 1 || shells.some(s => !s.available)) {
            sep(m);
            for (const sh of shells)
                add(m, sh.label, () => openShell(sh.id), live && sh.available);
        }
        return m;
    }

    // ---- drag & drop (SidebarTree) -----------------------------------------------------

    function dropTarget(x, y) {
        const row = listView.indexAt(x, y + listView.contentY);
        const item = row >= 0 ? listView.itemAtIndex(row) : null;
        const fraction = item ? (y + listView.contentY - item.y) / item.height : 0;
        return {
            "row": row,
            "fraction": fraction
        };
    }

    function dragOver(x, y) {
        const t = dropTarget(x, y);
        listView.dropRow = t.row;
        listView.dropZone = t.row >= 0 ? hostModel.dropZone(t.row, t.fraction) : -1;
        // Auto-scroll near the edges, as QAbstractItemView does.
        const edge = 16;
        if (y < edge)
            listView.contentY = Math.max(0, listView.contentY - 8);
        else if (y > listView.height - edge)
            listView.contentY = Math.max(0, Math.min(listView.contentHeight - listView.height, listView.contentY + 8));
    }

    function dropRows(current, selected, row, fraction) {
        finish(hostModel.dropRows(current, selected, row, fraction));
    }

    function endDrag(x, y, drop) {
        const t = dropTarget(x, y);
        const zone = listView.dropZone;
        listView.dropRow = -1;
        listView.dropZone = -1;
        if (drop && (t.row < 0 || zone >= 0))
            dropRows(currentRow, selection, t.row, t.fraction);
    }

    Component.onCompleted: {
        hostModel.refresh();
        shell.listSummary = Qt.binding(() => hostModel.status);
        shell.saveToHosts = conn => panel.saveFromTab(conn);
        const a = shell.shellActions;
        const live = (id, hook) => {
            const act = a.actions[id];
            act.enabled = Qt.binding(() => act.handler !== null && panel.shell[hook] !== null);
        };
        const unavailable = unavailableKinds();
        // From the focused list (Ctrl+Shift+N there) a new entry goes into the selected
        // folder; from anywhere else to the top level.
        const here = () => listView.activeFocus ? folderAt(currentRow) : "";
        for (const kind of ["ssh", "rdp", "vnc", "telnet", "serial", "favorite"]) {
            a.handle("file.new." + kind, () => addEntry(kind, here()));
            live("file.new." + kind, "openEditor");
            if (unavailable[kind]) {
                const act = a.actions["file.new." + kind];
                act.enabled = false;
                act.text = act.text + " (" + kindNote(kind) + ")";
            }
        }
        a.handle("file.new.folder", () => addGroup("")); // Ctrl+N in the list: hostlist.new_folder
        a.handle("edit.search_hosts", () => focusFilter());
        a.handle("edit.copy_conn_command", () => copyFromSelection());
        a.handle("tools.test_connection", () => testFromSelection());
        for (const t of tools) {
            a.handle("tools.net." + t[0], () => nettoolFromSelection(t[0]));
            live("tools.net." + t[0], "openInner");
        }
    }

    // Ctrl+L focuses the search from anywhere in the panel (never a window shortcut).
    Keys.onPressed: event => {
        if (event.key === Qt.Key_L && (event.modifiers & (Qt.ShiftModifier | Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier)) === Qt.ControlModifier) {
            focusFilter();
            event.accepted = true;
        }
    }

    HostListModel {
        id: hostModel

        onModelReset: {
            panel.selection = [];
            panel.currentRow = -1;
            panel.anchorRow = -1;
            panel.editingRow = -1;
        }
    }

    Connections {
        target: panel.shell.controller

        function onConfigChanged() {
            hostModel.refresh();
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 12
        anchors.topMargin: 12
        anchors.rightMargin: 10
        anchors.bottomMargin: 10
        spacing: 9

        // The search with compact Add ▾ / local-shell buttons beside it.
        RowLayout {
            Layout.fillWidth: true
            spacing: 2

            Kirigami.ActionTextField {
                id: searchField

                Layout.fillWidth: true
                placeholderText: qsTr("Search…")
                placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
                Accessible.name: qsTr("Filter list")
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: qsTr("Filter by label, host, user, tag or command (Ctrl+Shift+H from anywhere)")
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                KeyNavigation.tab: listView
                leftActions: Kirigami.Action {
                    icon.name: "search"
                }
                rightActions: Kirigami.Action {
                    icon.name: "edit-clear"
                    visible: searchField.text !== ""
                    onTriggered: searchField.clear()
                }
                onTextChanged: hostModel.setFilter(text)
                // hostlist.clear_search
                Keys.onEscapePressed: event => {
                    searchField.clear();
                    event.accepted = true;
                }
            }

            QQC2.ToolButton {
                id: addButton

                objectName: "sidebarAdd"
                icon.name: "list-add"
                display: QQC2.AbstractButton.IconOnly
                Accessible.name: qsTr("Add")
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: qsTr("Add a connection, favorite or folder; import sessions")
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                onClicked: {
                    if (panel.addMenu)
                        panel.addMenu.destroy();
                    panel.addMenu = panel.buildAddMenu();
                    panel.addMenu.popup(addButton, 0, addButton.height);
                }
            }

            // A split button like the toolbar's New: the button opens the default shell, the
            // arrow lists the shells (/etc/shells; Windows: cmd, PowerShell, WSL).
            QQC2.ToolButton {
                id: shellButton

                objectName: "sidebarShell"
                icon.name: Kirigami.Settings.style === "org.kde.desktop" ? "utilities-terminal" : "utilities-terminal-symbolic" // other styles tint it into a square
                text: qsTr("Shell")
                display: QQC2.AbstractButton.TextBesideIcon
                enabled: panel.shell.newLocalShell !== null
                Accessible.name: qsTr("New local shell")
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: qsTr("New local shell — arrow: choose the shell")
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                onClicked: panel.openShell("")
            }

            QQC2.ToolButton {
                objectName: "sidebarShellMenu"
                icon.name: "arrow-down"
                display: QQC2.AbstractButton.IconOnly
                enabled: shellButton.enabled
                Accessible.name: qsTr("Choose the local shell")
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: qsTr("Choose the local shell")
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                onClicked: panel.popupShellMenu()
            }
        }

        QQC2.Frame {
            Layout.fillWidth: true
            Layout.fillHeight: true
            padding: 2

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                QQC2.Label {
                    Layout.fillWidth: true
                    visible: hostModel.hint !== ""
                    text: hostModel.hint
                    font.italic: true
                    wrapMode: Text.Wrap
                    horizontalAlignment: Text.AlignHCenter
                    padding: 6
                }

                ListView {
                    id: listView
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {
                        id: vbar
                    }
                    rightMargin: vbar.visible ? vbar.width : 0

                    property int dropRow: -1
                    property int dropZone: -1

                    objectName: "hostList"
                    model: hostModel
                    clip: true
                    focus: true
                    activeFocusOnTab: true
                    // Desktop list: wheel and scroll bar scroll it, a mouse drag is DnD.
                    // (Outside a ScrollView a non-interactive ListView ignores the wheel.)
                    interactive: true
                    boundsBehavior: Flickable.StopAtBounds
                    Accessible.role: Accessible.List
                    Accessible.name: qsTr("Hosts and favorites")
                    Keys.onPressed: event => panel.listKey(event)

                    // Flickable's own wheel handling restarts its scroll animation on every event and
                    // drops the unfinished distance, so a fast (free-spinning) wheel crawls.
                    Kirigami.WheelHandler {
                        target: listView
                    }

                    // Clicks below the last row (behind the rows): clear the selection;
                    // right-click: the empty-space menu.
                    MouseArea {
                        parent: listView
                        z: -1
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        preventStealing: true
                        onPressed: mouse => {
                            listView.forceActiveFocus(Qt.MouseFocusReason);
                            if (!(mouse.modifiers & (Qt.ControlModifier | Qt.ShiftModifier)))
                                panel.setCurrent(-1, true);
                            if (mouse.button === Qt.RightButton)
                                panel.openContextMenu(-1, mouse.x, mouse.y);
                        }
                    }

                    delegate: HostRow {
                        id: rowItem

                        compact: !panel.detailed
                        selected: panel.selection.includes(index)
                        current: index === panel.currentRow && listView.activeFocus
                        dropZone: listView.dropRow === index ? listView.dropZone : -1
                        cut: panel.clipVersion >= 0 && panel.hosts.isCut(index)
                        // (A delegate being made has index -1 for a moment.)
                        editing: panel.editingRow >= 0 && index === panel.editingRow
                        onCommitRequested: (text, blur) => {
                            const err = panel.commitRename(index, text, blur);
                            if (err !== "" && blur)
                                panel.cancelRename(true);
                            else
                                rowItem.renameError = err;
                        }
                        onCancelRequested: panel.cancelRename(false)

                        MouseArea {
                            id: rowMouse

                            property point origin
                            property bool dragging: false
                            property bool pendingSingle: false

                            anchors.fill: parent
                            enabled: !rowItem.editing // clicks go to the rename field
                            acceptedButtons: Qt.LeftButton | Qt.RightButton
                            // The list is interactive only for the wheel: it must not take a fast drag.
                            preventStealing: true

                            onPressed: mouse => {
                                listView.forceActiveFocus(Qt.MouseFocusReason);
                                origin = Qt.point(mouse.x, mouse.y);
                                dragging = false;
                                pendingSingle = false;
                                const ctrl = mouse.modifiers & Qt.ControlModifier;
                                const shift = mouse.modifiers & Qt.ShiftModifier;
                                if (mouse.button === Qt.LeftButton && rowItem.arrowHit(mouse.x)) {
                                    panel.toggle(rowItem.index);
                                    return;
                                }
                                if (shift && panel.anchorRow >= 0) {
                                    panel.currentRow = rowItem.index;
                                    panel.selectRange(panel.anchorRow, rowItem.index);
                                } else if (ctrl) {
                                    panel.currentRow = rowItem.index;
                                    panel.toggleSelected(rowItem.index);
                                } else if (rowItem.selected) {
                                    // Kept for a drag of the whole selection or the menu.
                                    panel.currentRow = rowItem.index;
                                    pendingSingle = mouse.button === Qt.LeftButton;
                                } else {
                                    panel.setCurrent(rowItem.index, true);
                                }
                                if (mouse.button === Qt.RightButton) {
                                    const p = mapToItem(listView, mouse.x, mouse.y);
                                    panel.openContextMenu(rowItem.index, p.x, p.y);
                                }
                            }
                            onPositionChanged: mouse => {
                                if (!(mouse.buttons & Qt.LeftButton))
                                    return;
                                const p = mapToItem(listView, mouse.x, mouse.y);
                                if (!dragging) {
                                    const far = Math.abs(mouse.x - origin.x) + Math.abs(mouse.y - origin.y) >= Qt.styleHints.startDragDistance;
                                    // The Recent section is not draggable.
                                    if (!far || !rowItem.draggable)
                                        return;
                                    dragging = true;
                                    pendingSingle = false;
                                }
                                panel.dragOver(p.x, p.y);
                            }
                            onReleased: mouse => {
                                if (dragging) {
                                    const p = mapToItem(listView, mouse.x, mouse.y);
                                    dragging = false;
                                    panel.endDrag(p.x, p.y, true);
                                } else if (pendingSingle) {
                                    panel.setCurrent(rowItem.index, true);
                                }
                                pendingSingle = false;
                            }
                            onCanceled: {
                                if (dragging)
                                    panel.endDrag(0, 0, false);
                                dragging = false;
                            }
                            onDoubleClicked: mouse => {
                                if (mouse.button === Qt.LeftButton && !rowItem.arrowHit(mouse.x))
                                    panel.activate(rowItem.index);
                            }
                        }
                    }
                }
            }
        }
    }

    // QML's way to the clipboard (QGuiApplication.clipboard().setText()).
    TextEdit {
        id: clipboard

        visible: false
    }

    MessageBox {
        id: box

        property var onYes: null

        title: "Termigator"
        onButtonClicked: (button, role) => {
            const fn = onYes;
            onYes = null;
            if (button === MessageDialog.Yes && fn)
                fn();
        }
    }

    // "Add to Hosts…" when the connection is saved already: update (Yes), add (No) or Cancel.
    MessageBox {
        id: saveQuestionBox

        property var onYes: null
        property var onNo: null

        title: qsTr("Add to Hosts")
        buttons: MessageDialog.Yes | MessageDialog.No | MessageDialog.Cancel
        icon: "question"
        onButtonClicked: (button, role) => {
            const fn = button === MessageDialog.Yes ? onYes : button === MessageDialog.No ? onNo : null;
            onYes = null;
            onNo = null;
            if (fn)
                fn();
        }
    }

    PromptDialog {
        id: promptDialog
    }

    DeleteFolderDialog {
        id: deleteFolder
    }

    ConnectionTestDialog {
        id: connectionTestDialog

        hosts: hostModel
    }

    Component {
        id: menuComponent

        QQC2.Menu {}
    }

    Component {
        id: envItemContentComponent

        Item {
            id: envContent

            property string label: ""
            property color dotColor: "transparent"
            property var menuItem: null

            implicitWidth: envRow.implicitWidth
            implicitHeight: envRow.implicitHeight
            RowLayout {
                id: envRow

                anchors.fill: parent
                anchors.leftMargin: Kirigami.Units.smallSpacing
                anchors.rightMargin: Kirigami.Units.smallSpacing
                spacing: Kirigami.Units.largeSpacing

                // The style draws its check indicator outside contentItem; reserve the same space
                // so the dot does not sit inside the checkbox.
                Item {
                    Layout.preferredWidth: envContent.menuItem && envContent.menuItem.checkable ? envContent.menuItem.indicator.width : 0
                }
                Rectangle {
                    implicitWidth: Math.round(Kirigami.Units.gridUnit * 0.8)
                    implicitHeight: implicitWidth
                    radius: Kirigami.Units.smallSpacing * 0.75
                    color: envContent.dotColor
                    border.width: 0
                    Layout.alignment: Qt.AlignVCenter
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: envContent.label
                    elide: Text.ElideRight
                    verticalAlignment: Text.AlignVCenter
                    color: envContent.menuItem
                        ? (envContent.menuItem.highlighted ? envContent.menuItem.palette.highlightedText : envContent.menuItem.palette.windowText)
                        : Kirigami.Theme.textColor
                }
            }
        }
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
