import QtQuick
import QtQuick.Dialogs
import Termigator.Terminal.Probe

// Offscreen check of the host list (tests/sidebar_smoke.rs): rows, status, search, keys, the
// context menus' texts and order, null hooks, hooks stubbed like the panes and editors would set them, editor
// validation, copy, duplicate, move, drop, the probe dialog; saves a grab and quits with 0
// (1 after a console.error). A step returning false is retried (up to 10 s).
Item {
    id: check

    required property var shell
    property int step: 0
    property int waited: 0
    property var panel: null
    property var opened: []
    property var inner: []
    property var shells: []
    property var editor: null
    property var bulkCall: null

    function fail(reason) {
        console.error("termigator sidebar smoke: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok)
            fail(what);
        return ok;
    }

    function find(name, item) {
        if (item.objectName === name)
            return item;
        for (const child of item.children) {
            const hit = find(name, child);
            if (hit)
                return hit;
        }
        return null;
    }

    function press(key, modifiers, text) {
        Probe.key(shell, key, modifiers || Qt.NoModifier, text || "");
    }

    function type(text) {
        for (const ch of text)
            press(ch.toUpperCase().charCodeAt(0), Qt.NoModifier, ch);
    }

    function rows() {
        const out = [];
        for (let r = 0; r < panel.list.count; ++r)
            out.push(JSON.parse(panel.hosts.rowJson(r)).label);
        return out.join(",");
    }

    function findByName(name, item) {
        if (item.objectName === name)
            return item;
        for (const c of item.children) {
            const f = findByName(name, c);
            if (f)
                return f;
        }
        return null;
    }

    function rowOf(label) {
        for (let r = 0; r < panel.list.count; ++r)
            if (JSON.parse(panel.hosts.rowJson(r)).label === label)
                return r;
        return -1;
    }

    // "text" per item, "-" per separator, "Title>" per submenu, "!" before a disabled entry.
    function menuTexts(menu) {
        const out = [];
        for (let i = 0; i < menu.count; ++i) {
            const it = menu.itemAt(i);
            if (it.subMenu)
                out.push(it.subMenu.title + ">");
            else if (it.text === undefined)
                out.push("-");
            else
                out.push((it.enabled ? "" : "!") + it.text);
        }
        return out.join("|");
    }

    function menuAt(row) {
        const m = panel.buildContextMenu(row);
        const t = menuTexts(m);
        m.destroy();
        return t;
    }

    // The connections in the shared config, in order.
    function conns() {
        const out = [];
        for (let i = 0; ; ++i) {
            const json = panel.hosts.entryJson("connection", i);
            if (json === "")
                return out;
            out.push(JSON.parse(json));
        }
    }

    function groupOf(label) {
        const c = conns().find(e => e.label === label);
        return c && c.group ? c.group : "";
    }

    function entries() {
        return conns().map(c => c.label).join(",");
    }

    readonly property var steps: [
        // The panel in the Hosts tab, the rows of the collapsed tree, the status bar summary.
        () => {
            panel = find("hostsPanel", shell.contentItem);
            if (!expect(panel !== null, "no hosts panel"))
                return;
            expect(rows() === "prod,desk,broken,top", "rows " + rows());
            expect(panel.hosts.status === "5 entries · 2 folder(s)", "status " + panel.hosts.status);
            expect(shell.listSummary === panel.hosts.status, "summary " + shell.listSummary);
            const top = panel.list.itemAtIndex(rowOf("top"));
            expect(top.badge === "ext" && top.subtitle === "htop", "favorite row " + top.badge + " " + top.subtitle);
            const broken = panel.list.itemAtIndex(rowOf("broken"));
            expect(broken.error && broken.iconName === "dialog-warning", "error row");
            expect(broken.tooltip.endsWith("SSH gateway 'gone' is not a saved SSH entry."), broken.tooltip);
            const desk = panel.list.itemAtIndex(rowOf("desk"));
            expect(desk.badge === "rdp" && desk.subtitle === "u@10.0.0.7", "rdp row " + desk.badge + " " + desk.subtitle);
            expect(panel.list.itemAtIndex(0).badge === "2", "folder count chip");
        },
        // Settings → Appearance: the list is one line per host by default, two when asked for.
        () => {
            const desk = panel.list.itemAtIndex(rowOf("desk"));
            expect(!panel.detailed && desk.compact && !desk.showSubtitle && !desk.chipShown, "not compact by default");
            const compactHeight = desk.implicitHeight;
            expect(compactHeight > 0 && compactHeight < 30, "compact row height " + compactHeight);
            panel.detailed = true;
            expect(!desk.compact && desk.showSubtitle && desk.implicitHeight >= 36, "detailed row " + desk.implicitHeight);
            expect(desk.implicitHeight > compactHeight + 8, "detailed not taller: " + desk.implicitHeight + " vs " + compactHeight);
            panel.detailed = false;
            expect(desk.compact && desk.implicitHeight === compactHeight, "compact again " + desk.implicitHeight);
        },
        // A refresh that changes nothing keeps the rows (no model reset: the delegates, the scroll
        // position and the selection stay); the pointer over a row marks it hovered.
        () => {
            let resets = 0;
            const count = () => resets++;
            panel.hosts.modelReset.connect(count);
            panel.hosts.refresh();
            panel.hosts.modelReset.disconnect(count);
            expect(resets === 0, "an unchanged refresh reset the model " + resets + " times");
            const desk = panel.list.itemAtIndex(rowOf("desk"));
            const at = desk.mapToItem(null, desk.width / 2, desk.height / 2);
            Probe.move(shell, at.x - 40, at.y, at.x, at.y, 4);
        },
        () => {
            const desk = panel.list.itemAtIndex(rowOf("desk"));
            if (!desk.hovered)
                return false;
            const other = panel.list.itemAtIndex(rowOf("prod"));
            const at = other.mapToItem(null, other.width / 2, other.height / 2);
            Probe.move(shell, at.x, at.y, at.x, at.y, 1);
        },
        () => {
            const desk = panel.list.itemAtIndex(rowOf("desk"));
            if (desk.hovered)
                return false;
            expect(panel.list.itemAtIndex(rowOf("prod")).hovered, "the row under the pointer is not hovered");
        },
        // Keys: the list has focus; Down, Right expands, Enter toggles a folder too.
        () => {
            panel.list.forceActiveFocus();
            press(Qt.Key_Down);
            press(Qt.Key_Right);
        },
        () => {
            expect(panel.currentRow === 0 && rows() === "prod,db,web,desk,broken,top", "expanded " + rows());
            const db = panel.list.itemAtIndex(1);
            expect(db.kind === "group" && db.depth === 1, "subfolder row");
            const web = panel.list.itemAtIndex(rowOf("web"));
            expect(web.distroColor === "#E95420" && web.subtitle === "admin@10.0.0.5", "distro row " + web.distroColor + " " + web.subtitle);
            // The environment's bar and the tooltip line; desk has none.
            expect(web.environment === "production" && findByName("envMark", web).visible && web.tooltip.indexOf("Label: Production") >= 0, "environment row " + web.environment);
            expect(panel.list.itemAtIndex(rowOf("desk")).environment === "", "desk environment");
            // A folder's environment goes to everything inside without its own.
            const db0 = JSON.parse(panel.hosts.openJson("connection", 1));
            expect(db0.environment === undefined, "db has no environment yet " + JSON.stringify(db0));
            panel.finish(panel.hosts.setGroupEnvironment("prod", "staging"));
            const dbRow = panel.list.itemAtIndex(rowOf("db"));
            expect(dbRow.environment === "staging" && panel.list.itemAtIndex(rowOf("prod")).environment === "staging", "inherited " + dbRow.environment);
            expect(panel.list.itemAtIndex(rowOf("web")).environment === "production", "own environment wins");
            expect(JSON.parse(panel.hosts.openJson("connection", 1)).environment === "staging" && JSON.parse(panel.hosts.entryJson("connection", 1)).environment === undefined, "opened with the folder's, stored without");
            expect(JSON.parse(panel.hosts.groupEnvironment("prod/db")).from === "prod", "inherited from");
            panel.finish(panel.hosts.setGroupEnvironment("prod", ""));
            expect(panel.list.itemAtIndex(rowOf("db")).environment === "", "cleared");
            // An entry's own category wins over the folder's; the row gets its name and colour.
            const deskIndex = conns().findIndex(c => c.label === "desk");
            panel.finish(panel.hosts.setEntryEnvironment("connection", deskIndex, "development"));
            const deskRow = panel.list.itemAtIndex(rowOf("desk"));
            expect(deskRow.environment === "development" && deskRow.environmentName === "Development" && deskRow.environmentColor === "#27ae60", "entry environment " + deskRow.environment + " " + deskRow.environmentName + " " + deskRow.environmentColor);
            expect(deskRow.tooltip.indexOf("Label: Development") >= 0, "entry environment tooltip " + deskRow.tooltip);
            panel.finish(panel.hosts.setEntryEnvironment("connection", deskIndex, ""));
            expect(panel.list.itemAtIndex(rowOf("desk")).environment === "", "entry environment cleared");
            panel.setCurrent(0, true); // the rebuilds above dropped the current row
            panel.list.forceActiveFocus();
            press(Qt.Key_Return);
        },
        () => {
            expect(rows() === "prod,desk,broken,top", "Enter on a folder " + rows());
            expect(panel.currentRow === 0, "current kept " + panel.currentRow);
        },
        // Hooks are null: Enter on an entry and the menus' hook entries do nothing / are disabled.
        () => {
            // The merged areas (panes, editors, Files) set these at startup.
            shell.openConnection = null;
            shell.openInner = null;
            shell.newLocalShell = null;
            shell.openFilesTab = null;
            shell.openEditor = null;
            shell.openBulkEdit = null;
            panel.setCurrent(rowOf("desk"), true);
            press(Qt.Key_Return);
            const m = menuAt(rowOf("desk"));
            expect(m === "!Connect / Run|Test connection…|Network tools>|Copy RDP command|Copy address|-|!Edit…|Rename|Duplicate entry|Move to folder…|Label>|Set icon>|-|Cut|Copy|!Paste|-|Move up|Move down|-|Delete…", "rdp menu " + m);
        },
        // The menus of the list context menu: texts and order.
        () => {
            const empty = menuAt(-1);
            expect(empty === "!Add SSH connection…|!Add RDP connection…|!Add VNC connection…|!Add Telnet connection…|!Add serial connection…|!Add favorite command…|-|New folder…|!Paste", "empty menu " + empty);
            const folder = menuAt(0);
            expect(folder === "Expand|-|!Add SSH connection here…|!Add RDP connection here…|!Add VNC connection here…|!Add Telnet connection here…|!Add serial connection here…|!Add favorite command here…|-|New subfolder…|Label>|Rename folder|Move folder to…|-|Cut|Copy|!Paste|-|Delete folder…", "folder menu " + folder);
            const fav = menuAt(rowOf("top"));
            expect(fav === "!Connect / Run|-|!Edit…|Rename|Duplicate entry|Move to folder…|-|Cut|Copy|!Paste|-|Move up|Move down|-|Delete…", "favorite menu " + fav);
            const m = panel.buildContextMenu(rowOf("desk"));
            const net = m.itemAt(2).subMenu;
            expect(menuTexts(net) === "!Ping|!Trace route|!DNS lookup|!Check port|!Scan common ports", "network tools " + menuTexts(net));
            let env = null;
            for (let i = 0; i < m.count; ++i) {
                if (m.itemAt(i).text === "Label")
                    env = m.itemAt(i).subMenu;
            }
            expect(env !== null, "entry environment menu missing");
            expect(menuTexts(env) === "Inherit from folder (none)|Production|Staging|Test|Development", "entry environment menu " + menuTexts(env));
            const envColors = ["#da4453", "#f67400", "#3daee9", "#27ae60"];
            for (let i = 0; i < envColors.length; ++i) {
                const item = env.itemAt(i + 1);
                const dot = item.contentItem;
                expect(dot !== null && String(dot.dotColor).toLowerCase() === envColors[i], "environment dot " + i + " " + (dot ? dot.dotColor : "?"));
            }
            m.destroy();
            // Ctrl+A selects every row; the menu on a selected row is the multi one (connections only).
            panel.list.forceActiveFocus();
            press(Qt.Key_A, Qt.ControlModifier);
        },
        () => {
            expect(panel.selection.length === panel.list.count, "Ctrl+A " + panel.selection);
            const multi = menuAt(rowOf("desk"));
            expect(multi === "!Connect 2 entries|-|!Edit 2 entries…|Move 2 entries to folder…|Set icon>|-|Cut|Copy|-|Delete 2 entries…", "multi menu " + multi);
            // Delete with several connections selected: the bulk question; answer No.
            press(Qt.Key_Delete);
        },
        () => {
            expect(panel.messageBox.text === "Remove 2 entries from config?", "bulk delete " + panel.messageBox.text);
            panel.messageBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            panel.messageBox.close();
            expect(entries() === "web,db,desk,broken", "deleted on No " + entries());
            shell.requestActivate();
        },
        // Ctrl+F from the list focuses the search (once the box gave the focus back); typing
        // filters; Escape clears.
        () => {
            panel.list.forceActiveFocus();
            panel.setCurrent(rowOf("desk"), true);
            press(Qt.Key_F, Qt.ControlModifier);
        },
        () => {
            expect(panel.search.activeFocus, "Ctrl+F did not focus the search");
            type("db");
        },
        () => {
            expect(panel.search.text === "db" && rows() === "prod,db,db", "search rows " + rows() + " / " + panel.search.text);
            expect(panel.hosts.status === "1 result · search active", "search status " + panel.hosts.status);
            press(Qt.Key_Escape);
        },
        () => {
            expect(panel.search.text === "" && rows() === "prod,desk,broken,top", "Escape " + rows());
            // Now the hooks panes / editors / Files set.
            shell.openConnection = json => check.opened.push(JSON.parse(json).label);
            shell.openInner = (cmd, title, mode, env) => check.inner.push(title + "|" + mode);
            shell.newLocalShell = path => check.shells.push(path);
            shell.openEditor = (kind, json, group, done) => {
                check.editor = {
                    "kind": kind,
                    "json": json,
                    "group": group,
                    "done": done
                };
            };
            // "Edit N entries…" hands the count and the folders to the dialog's hook.
            shell.openBulkEdit = (count, groups, done) => {
                check.bulkCall = {
                    "count": count,
                    "groups": groups.join(","),
                    "done": done
                };
            };
            panel.bulkEdit([2, 3]);
            expect(check.bulkCall !== null && check.bulkCall.count === 2 && check.bulkCall.groups === "prod,prod/db", "bulk edit hook " + JSON.stringify(check.bulkCall));
            check.bulkCall.done({}); // nothing ticked: nothing changes
            panel.list.forceActiveFocus();
            panel.setCurrent(rowOf("desk"), true);
            press(Qt.Key_Return);
        },
        () => {
            expect(opened.join() === "desk", "opened " + opened);
            // A double click on the entry opens it too (Enter is checked above).
            const row = panel.list.itemAtIndex(rowOf("desk"));
            const at = row.mapToItem(null, row.width / 3, row.height / 2);
            Probe.click(shell, at.x, at.y, Qt.LeftButton, 2, 0);
            expect(opened.join() === "desk,desk", "double click opened " + opened);
            const m = menuAt(rowOf("desk"));
            expect(m.startsWith("Connect / Run|Test connection…|Network tools>|Copy RDP command|Copy address|-|Edit…|"), "live menu " + m);
            panel.runNettool("ping", 2);
            expect(inner.join() === "Ping: 10.0.0.7|embedded" || panel.messageBox.text.indexOf("not installed") >= 0, "nettool " + inner + " " + panel.messageBox.text);
            find("sidebarShell", panel).clicked();
            expect(shells.join() === "", "shell button " + JSON.stringify(shells));
            // Alt+Down moves the entry (within its folder), persisted.
            panel.setCurrent(rowOf("desk"), true);
            press(Qt.Key_Down, Qt.AltModifier);
        },
        () => {
            expect(entries() === "web,db,broken,desk", "Alt+Down " + entries());
            expect(rows() === "prod,broken,desk,top" && panel.currentRow === rowOf("desk"), "moved row " + rows() + " " + panel.currentRow);
            press(Qt.Key_Up, Qt.AltModifier);
        },
        () => {
            expect(entries() === "web,db,desk,broken", "Alt+Up " + entries());
            // Copy command: the clipboard text and the status-bar notice.
            panel.setCurrent(rowOf("desk"), true);
            panel.copyFromSelection();
            expect(shell.statusMessage === "Copied the RDP command of “desk” to the clipboard.", "copy " + shell.statusMessage);
            panel.copyAddress(panel.info(rowOf("desk")).index);
            expect(shell.statusMessage === "Copied the address “u@10.0.0.7” to the clipboard.", "address " + shell.statusMessage);
            // Duplicate: inserted after the original and selected.
            panel.duplicate(panel.info(rowOf("desk")));
        },
        () => {
            expect(entries() === "web,db,desk,desk (copy),broken", "duplicate " + entries());
            expect(panel.currentRow === rowOf("desk (copy)"), "duplicate selected " + panel.currentRow);
            // Edit… opens the editor for the entry; its result is validated, stored and saved.
            panel.editEntry(panel.info(panel.currentRow));
        },
        () => {
            expect(editor !== null && editor.kind === "rdp" && JSON.parse(editor.json).label === "desk (copy)", "editor " + JSON.stringify(editor));
            const err = editor.done(JSON.stringify({
                "protocol": "rdp",
                "user": "u"
            }));
            expect(err === "Enter a host." && !panel.messageBox.visible, "edit validation " + err);
        },
        () => {
            editor.done(JSON.stringify({
                "protocol": "rdp",
                "host": "10.0.0.9",
                "user": "u"
            }));
            expect(entries() === "web,db,desk,u@10.0.0.9,broken", "edited " + entries());
            // Add ▾ → SSH connection… into a folder from the folder menu.
            editor = null;
            panel.addEntry("ssh", "prod");
            expect(editor !== null && editor.json === "" && editor.group === "prod", "add editor " + JSON.stringify(editor));
            // The reason goes back to the editor, which stays open; no message box.
            const err = editor.done(JSON.stringify({
                "host": "10.0.0.10",
                "group": "prod"
            }));
            expect(err === "Enter host and user." && !panel.messageBox.visible, "add validation " + err);
        },
        () => {
            editor.done(JSON.stringify({
                "host": "10.0.0.10",
                "user": "ops",
                "group": "prod"
            }));
            expect(entries() === "web,db,desk,u@10.0.0.9,broken,ops@10.0.0.10", "added " + entries());
            // Drag & drop: desk onto the prod folder (the middle of the row = "on").
            const desk = rowOf("desk");
            panel.setCurrent(desk, true);
            expect(panel.hosts.dropZone(rowOf("prod"), 0.5) === 1 && panel.hosts.dropZone(rowOf("prod"), 0.1) === 0, "drop zones");
            panel.dropRows(desk, [desk], rowOf("prod"), 0.5);
        },
        () => {
            expect(conns().find(c => c.label === "desk").group === "prod", "dropped " + JSON.stringify(conns().map(c => c.group)));
            expect(rows().startsWith("prod,") && rowOf("desk") > 0 && panel.currentRow === rowOf("desk"), "revealed " + rows() + " " + panel.currentRow);
            // Move to folder through the prompt (editable combo), a new nested folder.
            panel.moveToGroup(panel.info(rowOf("desk")));
        },
        () => {
            expect(panel.prompt.visible && panel.prompt.title === "Move to folder", "prompt " + panel.prompt.title);
            panel.prompt.combo.editText = "lab/new";
            panel.prompt.accept();
        },
        () => {
            expect(conns().find(c => c.label === "desk").group === "lab/new", "moved " + JSON.stringify(conns().map(c => c.group)));
            expect(panel.info(panel.currentRow).label === "desk", "selected after move");
            // Probe of a refused local port, off the GUI thread.
            panel.testDialog.test(JSON.stringify({
                "label": "loop",
                "host": "127.0.0.1",
                "port": 1,
                "user": "u"
            }));
            expect(panel.testDialog.visible && panel.testDialog.title === "Test connection — loop", "probe dialog " + panel.testDialog.title);
        },
        () => {
            if (panel.testDialog.report === null)
                return false;
            const r = panel.testDialog.report;
            expect(r.headline.startsWith("✘  ") && r.steps.length > 0, "probe report " + JSON.stringify(r));
            panel.testDialog.close();
            // The shell's menu actions handled here: File → New → Folder… asks for a name.
            shell.shellActions.actions["file.new.folder"].trigger();
        },
        () => {
            expect(panel.prompt.visible && panel.prompt.title === "New folder", "new folder prompt " + panel.prompt.title);
            panel.prompt.field.text = "empty";
            panel.prompt.accept();
            expect(rowOf("empty") >= 0, "new folder row " + rows());
            expect(shell.shellActions.actions["file.new.ssh"].enabled, "New SSH action disabled with an editor");
            // Offscreen, a closed message box stays the focus window until the shell asks back.
            shell.requestActivate();
        },
        // Delete on a folder with entries: the dialog asks where they go; into another folder.
        () => {
            panel.list.forceActiveFocus();
            panel.setCurrent(rowOf("prod"), true);
            press(Qt.Key_Delete);
        },
        () => {
            const d = panel.deleteFolderDialog;
            expect(d.visible && d.question === "The “prod” folder holds 3 entries and 1 subfolder.\nWhat should happen to them?", "delete folder " + d.question);
            expect(d.toParent.checked && d.toParent.text === "Move them to the top level", "default choice " + d.toParent.text);
            expect(JSON.stringify(d.target.model) === JSON.stringify(["empty", "lab", "lab/new"]), "targets " + JSON.stringify(d.target.model));
            d.toOther.checked = true;
            d.target.currentIndex = d.target.find("empty");
            d.accept();
            expect(entries() === "web,db,desk,u@10.0.0.9,broken,ops@10.0.0.10", "entries kept " + entries());
            expect(groupOf("web") === "empty" && groupOf("db") === "empty/db" && groupOf("ops@10.0.0.10") === "empty", "moved into empty " + JSON.stringify(conns().map(c => c.group)));
            expect(panel.hosts.groupNames().indexOf("prod") < 0, "prod gone " + panel.hosts.groupNames());
            // Move folder to…: the picker leaves out the folder itself and what is below it.
            panel.moveFolder("empty/db");
        },
        () => {
            expect(panel.prompt.visible && panel.prompt.title === "Move folder", "move prompt " + panel.prompt.title);
            expect(panel.prompt.choices.indexOf("empty/db") < 0 && panel.prompt.choices[0] === "(top level)", "move targets " + panel.prompt.choices);
            panel.prompt.combo.editText = "lab";
            panel.prompt.accept();
            expect(groupOf("db") === "lab/db", "folder moved " + groupOf("db"));
            // Rename folder: the name in place in its row; Enter takes it.
            shell.requestActivate();
            panel.startRename(panel.hosts.groupRow("lab"));
        },
        () => {
            const field = panel.list.itemAtIndex(panel.editingRow).renameField;
            if (!field.activeFocus)
                return false;
            expect(field.text === "lab" && field.selectedText === "lab", "rename field " + field.text + " / " + field.selectedText);
            field.text = "infra";
            press(Qt.Key_Return);
        },
        () => {
            expect(panel.editingRow === -1 && groupOf("db") === "infra/db" && groupOf("desk") === "infra/new", "renamed " + JSON.stringify(conns().map(c => c.group)));
            // The last entry dragged out of a folder: the folder stays, empty.
            const row = panel.hosts.reveal("connection", conns().findIndex(c => c.label === "db"));
            expect(row >= 0, "db shown");
            panel.dropRows(row, [row], -1, 0);
            expect(groupOf("db") === "" && panel.hosts.groupNames().indexOf("infra/db") >= 0, "emptied folder kept " + panel.hosts.groupNames());
            // Delete with its contents.
            panel.deleteGroup("empty");
        },
        () => {
            const d = panel.deleteFolderDialog;
            expect(d.visible && d.question === "The “empty” folder holds 2 entries.\nWhat should happen to them?", "delete empty " + d.question);
            d.deleteAll.checked = true;
            d.accept();
            expect(entries() === "db,desk,u@10.0.0.9,broken", "folder deleted " + entries());
            // Into the parent folder.
            panel.deleteGroup("infra/new");
        },
        () => {
            const d = panel.deleteFolderDialog;
            expect(d.visible && d.toParent.text === "Move them to the parent folder “infra”", "parent choice " + d.toParent.text);
            d.accept();
            expect(groupOf("desk") === "infra" && panel.hosts.groupNames().indexOf("infra/new") < 0, "into the parent " + groupOf("desk"));
            // An empty folder only asks Yes / No.
            panel.deleteGroup("infra/db");
            expect(panel.messageBox.text === "Delete the empty folder “infra/db”?", "empty folder question " + panel.messageBox.text);
            panel.messageBox.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
            panel.messageBox.close();
            expect(panel.deleteFolderDialog.path === "infra/new" && panel.hosts.groupNames().indexOf("infra/db") < 0, "empty folder deleted " + panel.hosts.groupNames());
            shell.requestActivate();
            panel.addGroup("infra");
        },
        () => {
            panel.prompt.field.text = "kept";
            panel.prompt.accept();
            expect(panel.hosts.groupNames().indexOf("infra/kept") >= 0, "subfolder " + panel.hosts.groupNames());
            // "Add to Hosts…" on a tab: the hook, then a new connection without its password.
            expect(typeof shell.saveToHosts === "function", "saveToHosts hook");
            editor = null;
            shell.saveToHosts({
                "label": "ops@192.0.2.20",
                "host": "192.0.2.20",
                "user": "ops",
                "port": null,
                "terminal": "embedded",
                "password": "tab-secret"
            });
            const e = editor ? JSON.parse(editor.json) : null;
            expect(e !== null && editor.kind === "ssh" && e.host === "192.0.2.20" && e.password === undefined && !panel.saveQuestion.visible, "add to hosts " + JSON.stringify(editor));
            editor.done(JSON.stringify({
                "label": "jump",
                "host": "192.0.2.20",
                "user": "ops"
            }));
            expect(entries() === "db,desk,u@10.0.0.9,broken,jump", "added from tab " + entries());
        },
        // The same target again (another name, port 22 spelled out): the question; No adds anew.
        () => {
            editor = null;
            shell.saveToHosts({
                "label": "ops@192.0.2.20",
                "host": "192.0.2.20",
                "user": "ops",
                "port": 22
            });
            expect(editor === null && panel.saveQuestion.text.indexOf("“jump”") >= 0, "duplicate question " + panel.saveQuestion.text);
            panel.saveQuestion.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            panel.saveQuestion.close();
            expect(editor !== null && editor.kind === "ssh" && JSON.parse(editor.json).label === "ops@192.0.2.20", "create new " + JSON.stringify(editor));
            editor.done(JSON.stringify({
                "label": "ops@192.0.2.20",
                "host": "192.0.2.20",
                "user": "ops",
                "port": 22
            }));
            expect(entries() === "db,desk,u@10.0.0.9,broken,jump,ops@192.0.2.20", "created anew " + entries());
        },
        // The same name (any case): Yes edits that entry, with the tab's host; Cancel does nothing.
        () => {
            editor = null;
            shell.saveToHosts({
                "label": "DESK",
                "protocol": "rdp",
                "host": "10.0.0.70",
                "user": "u"
            });
            panel.saveQuestion.buttonClicked(MessageDialog.Cancel, MessageDialog.RejectRole);
            panel.saveQuestion.close();
            expect(editor === null, "cancel " + JSON.stringify(editor));
            shell.saveToHosts({
                "label": "DESK",
                "protocol": "rdp",
                "host": "10.0.0.70",
                "user": "u"
            });
            panel.saveQuestion.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
            panel.saveQuestion.close();
            const m = editor ? JSON.parse(editor.json) : null;
            expect(m !== null && editor.kind === "rdp" && m.label === "desk" && m.host === "10.0.0.70" && m.group === "infra", "update " + JSON.stringify(editor));
            editor.done(JSON.stringify(m));
            const desk = conns().find(c => c.label === "desk");
            expect(entries() === "db,desk,u@10.0.0.9,broken,jump,ops@192.0.2.20" && desk.host === "10.0.0.70" && desk.group === "infra", "updated " + JSON.stringify(desk));
        },
        () => {
            const started = shell.contentArea.grabToImage(result => {
                if (!result.saveToFile(shell.controller.smokeTestShot))
                    check.fail("cannot save the grab");
                console.info("termigator sidebar smoke: done");
                Qt.exit(0);
            });
            if (!started)
                fail("grab not started");
        }
    ]

    Timer {
        interval: 150
        repeat: true
        running: check.step < check.steps.length
        onTriggered: {
            let done;
            try {
                done = check.steps[check.step]();
            } catch (e) {
                check.fail(e);
                return;
            }
            if (done === false && ++check.waited < 70)
                return;
            check.waited = 0;
            ++check.step;
        }
    }
}
