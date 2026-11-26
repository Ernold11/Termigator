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
    readonly property alias hosts: hosts
    readonly property alias list: list
    readonly property alias search: search
    readonly property alias messageBox: box
    readonly property alias saveQuestion: saveQuestion
    readonly property alias prompt: prompt
    readonly property alias testDialog: testDialog
    // Selected rows (sorted) and the current row; both reset with the model.
    property var selection: []
    property int currentRow: -1
    property int anchorRow: -1
    // Settings → Appearance: one line per host (default) or two (user@host under the name).
    property bool detailed: shell.controller.windowBool("host_list_detailed", false)
    property QQC2.Menu contextMenu: null
    property QQC2.Menu addMenu: null
    property QQC2.Menu shellMenu: null
    readonly property var tools: JSON.parse(hosts.toolsJson())
    readonly property var iconChoices: JSON.parse(hosts.iconChoicesJson())

    objectName: "hostsPanel"

    Connections {
        target: panel.shell.controller

        function onConfigChanged() {
            panel.detailed = panel.shell.controller.windowBool("host_list_detailed", false);
        }
    }

    // ---- rows and selection ------------------------------------------------------------

    function info(row) {
        const text = row >= 0 ? hosts.rowJson(row) : "";
        return text === "" ? null : JSON.parse(text);
    }

    function setCurrent(row, select) {
        currentRow = row;
        if (select) {
            selection = row >= 0 ? [row] : [];
            anchorRow = row;
        }
        if (row >= 0)
            list.positionViewAtIndex(row, ListView.Contain);
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
        selectRange(0, list.count - 1);
    }

    function toggle(row) {
        const i = info(row);
        if (!i || (i.kind !== "group" && i.kind !== "recent"))
            return;
        const kept = hosts.setExpanded(row, !i.expanded, [currentRow].concat(selection));
        currentRow = kept[0];
        selection = kept.slice(1).filter(r => r >= 0);
    }

    function reveal(kind, index) {
        setCurrent(hosts.reveal(kind, index), true);
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
        search.forceActiveFocus(Qt.ShortcutFocusReason);
        search.selectAll();
    }

    function openEntry(i) {
        if (shell.openConnection)
            shell.openConnection(hosts.openJson(i.kind, i.index)); // with the folder's environment
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
            shell.openEditor(kind, "", group || "", data => store(hosts.storeEntry(kind, "", -1, data)));
    }

    function editEntry(i) {
        const kind = i.kind === "favorite" ? "favorite" : i.protocol;
        if (shell.openEditor)
            shell.openEditor(kind, hosts.entryJson(i.kind, i.index), "", data => store(hosts.storeEntry(kind, i.kind, i.index, data)));
    }

    // "Add to Hosts…" on a tab (shell.saveToHosts): the editor with the tab's connection, without
    // its secrets. One that is saved already (same name, or same protocol, host, port and user)
    // asks first: Yes edits that entry (it keeps its own password), No adds a new one.
    function saveFromTab(conn) {
        if (!shell.openEditor)
            return;
        const p = JSON.parse(hosts.tabSavePlan(JSON.stringify(conn)));
        const addNew = () => shell.openEditor(p.kind, JSON.stringify(p.entry), p.entry.group || "", data => store(hosts.storeEntry(p.kind, "", -1, data)));
        if (p.existing < 0) {
            addNew();
            return;
        }
        saveQuestion.onYes = () => shell.openEditor(p.mergedKind, JSON.stringify(p.merged), "", data => store(hosts.storeEntry(p.mergedKind, "connection", p.existing, data)));
        saveQuestion.onNo = addNew;
        saveQuestion.text = qsTr("“%1” is already in Hosts.\n\nYes updates that entry, No adds a new one.").arg(p.existingLabel);
        saveQuestion.open();
    }

    function duplicate(i) {
        finish(hosts.duplicate(i.kind, i.index));
    }

    function moveBy(i, delta) {
        finish(hosts.moveBy(i.kind, i.index, delta));
    }

    function moveToGroup(i) {
        const choices = ["(top level)"].concat(hosts.groupNames());
        const at = i.group ? choices.indexOf(i.group) : 0;
        prompt.ask(qsTr("Move to folder"), qsTr("Folder (type a new name to create it; use / to nest, e.g. prod/web):"), "", choices, Math.max(at, 0), name => finish(hosts.moveToGroup(i.kind, i.index, name)));
    }

    function removeEntry(i) {
        ask(qsTr("Remove the selected entry from config?"), () => finish(hosts.removeEntry(i.kind, i.index)));
    }

    function addGroup(parentPath) {
        const where = parentPath ? qsTr(" inside “%1”").arg(parentPath) : "";
        prompt.ask(qsTr("New folder"), qsTr("Folder name%1\n(use / to nest deeper):").arg(where), "", null, 0, name => finish(hosts.addGroup(parentPath, name)));
    }

    function renameGroup(path) {
        prompt.ask(qsTr("Rename / move folder"), qsTr("New folder path (use / to nest, e.g. prod/web):"), path, null, 0, name => finish(hosts.renameGroup(path, name)));
    }

    function ungroup(path) {
        ask(hosts.dissolveQuestion(path), () => finish(hosts.ungroup(path)));
    }

    function deleteGroup(path) {
        ask(hosts.deleteGroupQuestion(path), () => finish(hosts.deleteGroup(path)));
    }

    function connectMany(indices) {
        for (const i of indices)
            openEntry({ kind: "connection", index: i });
    }

    function bulkApply(indices, changes) {
        finish(hosts.bulkApply(indices, JSON.stringify(changes)));
    }

    // Bulk edit: the tick-to-change dialog, then the same apply as the icon menu.
    function bulkEdit(indices) {
        shell.openBulkEdit(indices.length, hosts.groupNames(), changes => bulkApply(indices, changes));
    }

    function bulkMove(indices) {
        const choices = ["(top level)"].concat(hosts.groupNames());
        prompt.ask(qsTr("Move to folder"), qsTr("Folder for %1 entries (type a new name to create it):").arg(indices.length), "", choices, 0, name => bulkApply(indices, {
            "group": name === "(top level)" ? "" : name
        }));
    }

    function bulkDelete(indices) {
        ask(qsTr("Remove %1 entries from config?").arg(indices.length), () => finish(hosts.bulkDelete(indices)));
    }

    function copyCommand(index) {
        finish(hosts.copyCommand(index));
    }

    function copyAddress(index) {
        finish(hosts.copyAddress(index));
    }

    function testConnection(index) {
        testDialog.test(hosts.entryJson("connection", index));
    }

    function runNettool(key, index) {
        const o = JSON.parse(hosts.nettool(index, key));
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
        const multi = hosts.selectedConnections(selection);
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

    function renameKey() {
        const i = info(currentRow);
        if (!i)
            return;
        if (i.kind === "group")
            renameGroup(i.path);
        else if (i.kind === "connection" || i.kind === "favorite")
            editEntry(i);
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
            row = list.indexAt(0, list.contentY);
        const item = row >= 0 ? list.itemAtIndex(row) : null;
        const p = item ? item.mapToItem(list, item.width / 2, item.height / 2) : Qt.point(0, 0);
        openContextMenu(row, p.x, p.y);
    }

    function navigate(row, modifiers) {
        if (list.count === 0)
            return;
        row = Math.max(0, Math.min(list.count - 1, row));
        if (modifiers & Qt.ShiftModifier) {
            currentRow = row;
            selectRange(anchorRow >= 0 ? anchorRow : row, row);
            list.positionViewAtIndex(row, ListView.Contain);
        } else {
            setCurrent(row, !(modifiers & Qt.ControlModifier));
        }
    }

    function listKey(event) {
        const mods = event.modifiers & (Qt.ShiftModifier | Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier);
        const key = event.key;
        const page = Math.max(1, Math.floor(list.height / 48));
        if ((key === Qt.Key_Return || key === Qt.Key_Enter) && mods === 0)
            activate(currentRow); // hostlist.connect
        else if (key === Qt.Key_Up && mods === Qt.AltModifier)
            moveKey(-1); // hostlist.move_up
        else if (key === Qt.Key_Down && mods === Qt.AltModifier)
            moveKey(1); // hostlist.move_down
        else if (key === Qt.Key_Delete && mods === 0)
            deleteKey(); // hostlist.delete
        else if (key === Qt.Key_F2 && mods === 0)
            renameKey(); // hostlist.rename
        else if ((key === Qt.Key_F || key === Qt.Key_L) && mods === Qt.ControlModifier)
            focusFilter(); // hostlist.focus_search
        else if ((key === Qt.Key_F10 && mods === Qt.ShiftModifier) || (key === Qt.Key_Menu && mods === 0))
            menuKey(); // hostlist.context_menu
        else if (key === Qt.Key_A && mods === Qt.ControlModifier)
            selectAll();
        else if (key === Qt.Key_Up && (mods & ~(Qt.ShiftModifier | Qt.ControlModifier)) === 0)
            navigate(currentRow < 0 ? 0 : currentRow - 1, mods);
        else if (key === Qt.Key_Down && (mods & ~(Qt.ShiftModifier | Qt.ControlModifier)) === 0)
            navigate(currentRow + 1, mods);
        else if (key === Qt.Key_Home)
            navigate(0, mods);
        else if (key === Qt.Key_End)
            navigate(list.count - 1, mods);
        else if (key === Qt.Key_PageUp)
            navigate(currentRow - page, mods);
        else if (key === Qt.Key_PageDown)
            navigate(currentRow + page, mods);
        else if ((key === Qt.Key_Left || key === Qt.Key_Right) && mods === 0) {
            // QTreeView: Left collapses, Right expands.
            const i = info(currentRow);
            if (i && (i.kind === "group" || i.kind === "recent") && i.expanded === (key === Qt.Key_Left))
                toggle(currentRow);
        } else {
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
            m.icon.name = hosts.menuIcon(title);
        return m;
    }

    function add(menu, text, fn, enabled, iconName, iconColor) {
        const item = itemComponent.createObject(null, {
            "text": text,
            "run": fn,
            "enabled": enabled === undefined ? true : enabled
        });
        item.icon.name = iconName === undefined ? hosts.menuIcon(text) : iconName;
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
        return JSON.parse(hosts.environmentsJson());
    }

    function envName(id) {
        const e = envCategories().find(c => c.id.toLowerCase() === String(id).toLowerCase());
        return e ? e.name : id;
    }

    // Environment ▸ for a folder; entries inside without their own inherit it.
    function envMenu(menu, path) {
        const st = JSON.parse(hosts.groupEnvironment(path));
        const s = sub(menu, qsTr("Label"));
        const inherited = st.inherited !== "" ? envName(st.inherited) : "";
        const inheritText = st.own === "" && inherited !== "" ? qsTr("Inherit from parent (%1)").arg(inherited) : qsTr("Inherit from parent folder (none)");
        const inherit = add(s, inheritText, () => finish(hosts.setGroupEnvironment(path, "")), true, "");
        inherit.checkable = true;
        inherit.checked = st.own === "";
        for (const en of envCategories()) {
            const item = addDot(s, en.name, () => finish(hosts.setGroupEnvironment(path, en.id)), en.color);
            item.checkable = true;
            item.checked = st.own.toLowerCase() === en.id.toLowerCase();
        }
    }

    // The same menu for one entry; "" means "take the folder's".
    function entryEnvMenu(menu, i) {
        const st = JSON.parse(hosts.entryEnvironment(i.kind, i.index));
        const s = sub(menu, qsTr("Label"));
        const inherited = st.inherited !== "" ? envName(st.inherited) : "";
        const inheritText = inherited !== "" ? qsTr("Inherit from folder (%1)").arg(inherited) : qsTr("Inherit from folder (none)");
        const inherit = add(s, inheritText, () => finish(hosts.setEntryEnvironment(i.kind, i.index, "")), true, "");
        inherit.checkable = true;
        inherit.checked = st.own === "";
        for (const en of envCategories()) {
            const item = addDot(s, en.name, () => finish(hosts.setEntryEnvironment(i.kind, i.index, en.id)), en.color);
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
        add(menu, qsTr("Delete %1 entries…").arg(n), () => bulkDelete(indices));
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
            add(m, toggleText, () => toggle(row));
            return m;
        }
        const multi = hosts.selectedConnections(selection);
        if (i && multi.length > 1 && selection.includes(row)) {
            fillMulti(m, multi);
            return m;
        }
        if (!i) {
            addKind(m, qsTr("Add SSH connection…"), "ssh", undefined, editor);
            addKind(m, qsTr("Add RDP connection…"), "rdp", undefined, editor);
            addKind(m, qsTr("Add VNC connection…"), "vnc", undefined, editor);
            addKind(m, qsTr("Add Telnet connection…"), "telnet", undefined, editor);
            addKind(m, qsTr("Add serial connection…"), "serial", undefined, editor);
            addKind(m, qsTr("Add favorite command…"), "favorite", undefined, editor);
            sep(m);
            add(m, qsTr("New folder…"), () => addGroup(""));
        } else if (i.kind === "group") {
            const g = i.path;
            add(m, toggleText, () => toggle(row));
            sep(m);
            addKind(m, qsTr("Add SSH connection here…"), "ssh", g, editor);
            addKind(m, qsTr("Add RDP connection here…"), "rdp", g, editor);
            addKind(m, qsTr("Add VNC connection here…"), "vnc", g, editor);
            addKind(m, qsTr("Add Telnet connection here…"), "telnet", g, editor);
            addKind(m, qsTr("Add serial connection here…"), "serial", g, editor);
            addKind(m, qsTr("Add favorite command here…"), "favorite", g, editor);
            sep(m);
            add(m, qsTr("New subfolder…"), () => addGroup(g));
            envMenu(m, g);
            add(m, qsTr("Rename / move folder…"), () => renameGroup(g));
            add(m, qsTr("Dissolve folder (keep entries)"), () => ungroup(g));
            add(m, qsTr("Delete folder and contents…"), () => deleteGroup(g));
        } else {
            const conn = i.kind === "connection";
            add(m, qsTr("Connect / Run"), () => openEntry(i), shell.openConnection !== null);
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
                    add(m, qsTr("Open files (SFTP)"), () => shell.openFilesTab(hosts.entryJson("connection", i.index)), shell.openFilesTab !== null);
                    add(m, qsTr("Upload file to host…"), () => shell.scpTransfer(hosts.entryJson("connection", i.index), true), shell.scpTransfer !== null);
                    add(m, qsTr("Download file from host…"), () => shell.scpTransfer(hosts.entryJson("connection", i.index), false), shell.scpTransfer !== null);
                }
            }
            sep(m);
            add(m, qsTr("Edit…"), () => editEntry(i), editor);
            add(m, qsTr("Duplicate entry"), () => duplicate(i));
            add(m, qsTr("Move to group…"), () => moveToGroup(i));
            if (conn) {
                entryEnvMenu(m, i);
                iconMenu(m, [i.index]);
            }
            sep(m);
            add(m, qsTr("Move up"), () => moveBy(i, -1));
            add(m, qsTr("Move down"), () => moveBy(i, 1));
            sep(m);
            add(m, qsTr("Delete…"), () => removeEntry(i));
        }
        return m;
    }

    function openContextMenu(row, x, y) {
        if (contextMenu)
            contextMenu.destroy();
        contextMenu = buildContextMenu(row);
        contextMenu.popup(list, x, y);
    }

    function importAction(id) {
        return shell.shellActions.actions[id] ?? null;
    }

    // Kinds this machine cannot open (Windows: serial, VNC / Telnet without a client): greyed out
    // with a short reason, so no one builds an entry that only fails when connecting.
    function unavailableKinds() {
        return JSON.parse(hosts.kindSupport());
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
        const shells = JSON.parse(hosts.shells());
        if (shells.length > 1 || shells.some(s => !s.available)) {
            sep(m);
            for (const sh of shells)
                add(m, sh.label, () => openShell(sh.id), live && sh.available);
        }
        return m;
    }

    // ---- drag & drop (SidebarTree) -----------------------------------------------------

    function dropTarget(x, y) {
        const row = list.indexAt(x, y + list.contentY);
        const item = row >= 0 ? list.itemAtIndex(row) : null;
        const fraction = item ? (y + list.contentY - item.y) / item.height : 0;
        return {
            "row": row,
            "fraction": fraction
        };
    }

    function dragOver(x, y) {
        const t = dropTarget(x, y);
        list.dropRow = t.row;
        list.dropZone = t.row >= 0 ? hosts.dropZone(t.row, t.fraction) : -1;
        // Auto-scroll near the edges, as QAbstractItemView does.
        const edge = 16;
        if (y < edge)
            list.contentY = Math.max(0, list.contentY - 8);
        else if (y > list.height - edge)
            list.contentY = Math.max(0, Math.min(list.contentHeight - list.height, list.contentY + 8));
    }

    function dropRows(current, selected, row, fraction) {
        finish(hosts.dropRows(current, selected, row, fraction));
    }

    function endDrag(x, y, drop) {
        const t = dropTarget(x, y);
        const zone = list.dropZone;
        list.dropRow = -1;
        list.dropZone = -1;
        if (drop && (t.row < 0 || zone >= 0))
            dropRows(currentRow, selection, t.row, t.fraction);
    }

    Component.onCompleted: {
        hosts.refresh();
        shell.listSummary = Qt.binding(() => hosts.status);
        shell.saveToHosts = conn => panel.saveFromTab(conn);
        const a = shell.shellActions;
        const live = (id, hook) => {
            const act = a.actions[id];
            act.enabled = Qt.binding(() => act.handler !== null && panel.shell[hook] !== null);
        };
        const unavailable = unavailableKinds();
        for (const kind of ["ssh", "rdp", "vnc", "telnet", "serial", "favorite"]) {
            a.handle("file.new." + kind, () => addEntry(kind));
            live("file.new." + kind, "openEditor");
            if (unavailable[kind]) {
                const act = a.actions["file.new." + kind];
                act.enabled = false;
                act.text = act.text + " (" + kindNote(kind) + ")";
            }
        }
        a.handle("file.new.folder", () => addGroup(""));
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
        id: hosts

        onModelReset: {
            panel.selection = [];
            panel.currentRow = -1;
            panel.anchorRow = -1;
        }
    }

    Connections {
        target: panel.shell.controller

        function onConfigChanged() {
            hosts.refresh();
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
                id: search

                Layout.fillWidth: true
                placeholderText: qsTr("Search…")
                placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
                Accessible.name: qsTr("Filter list")
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: qsTr("Filter by label, host, user, tag or command (Ctrl+Shift+H from anywhere)")
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                KeyNavigation.tab: list
                leftActions: Kirigami.Action {
                    icon.name: "search"
                }
                rightActions: Kirigami.Action {
                    icon.name: "edit-clear"
                    visible: search.text !== ""
                    onTriggered: search.clear()
                }
                onTextChanged: hosts.setFilter(text)
                // hostlist.clear_search
                Keys.onEscapePressed: event => {
                    search.clear();
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
                    visible: hosts.hint !== ""
                    text: hosts.hint
                    font.italic: true
                    wrapMode: Text.Wrap
                    horizontalAlignment: Text.AlignHCenter
                    padding: 6
                }

                ListView {
                    id: list
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {
                        id: vbar
                    }
                    rightMargin: vbar.visible ? vbar.width : 0

                    property int dropRow: -1
                    property int dropZone: -1

                    objectName: "hostList"
                    model: hosts
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
                        target: list
                    }

                    // Clicks below the last row (behind the rows): clear the selection;
                    // right-click: the empty-space menu.
                    MouseArea {
                        parent: list
                        z: -1
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        preventStealing: true
                        onPressed: mouse => {
                            list.forceActiveFocus(Qt.MouseFocusReason);
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
                        current: index === panel.currentRow && list.activeFocus
                        dropZone: list.dropRow === index ? list.dropZone : -1

                        MouseArea {
                            id: rowMouse

                            property point origin
                            property bool dragging: false
                            property bool pendingSingle: false

                            anchors.fill: parent
                            acceptedButtons: Qt.LeftButton | Qt.RightButton
                            // The list is interactive only for the wheel: it must not take a fast drag.
                            preventStealing: true

                            onPressed: mouse => {
                                list.forceActiveFocus(Qt.MouseFocusReason);
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
                                    const p = mapToItem(list, mouse.x, mouse.y);
                                    panel.openContextMenu(rowItem.index, p.x, p.y);
                                }
                            }
                            onPositionChanged: mouse => {
                                if (!(mouse.buttons & Qt.LeftButton))
                                    return;
                                const p = mapToItem(list, mouse.x, mouse.y);
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
                                    const p = mapToItem(list, mouse.x, mouse.y);
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
        id: saveQuestion

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
        id: prompt
    }

    ConnectionTestDialog {
        id: testDialog

        hosts: hosts
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
