import QtQuick
import QtQuick.Dialogs
import Termigator
import Termigator.Terminal.Probe

// Offscreen check (tests/files_smoke.rs): the Files panel against a "remote host" that is this
// machine — the test puts a stand-in `ssh` first in PATH (it runs the remote command, or
// sftp-server for scp's `-s`), so the real ssh / scp / sftp argv run end to end. The test also
// made <stem>remote and <stem>local with their files; this side drives the panel through the
// same functions its buttons, menus and drops call, and the test then looks at the disk.
//
// Every step runs every 100 ms until it returns something other than false (a step that waits for
// the host returns false until the condition holds; 10 s at most).
Item {
    id: check

    required property var shell
    property var panel: null
    property var files: null
    property var tr: null
    property var area: null
    property int step: 0
    property int waited: 0
    property bool failed: false
    property var held: ({})
    readonly property string stem: shell.controller.smokeTestShot.replace(/shot\.png$/, "")
    readonly property string remote: stem + "remote"
    readonly property string local: stem + "local"
    readonly property var entry: ({
            "label": "box",
            "host": "h",
            "user": "u"
        })
    readonly property string entryJson: JSON.stringify(entry)
    readonly property string staleText: "The remote folder changed while dragging — drag again."
    readonly property string lineBreakText: "Cannot resume: the path contains a line break or NUL character."

    function fail(reason) {
        failed = true;
        console.error("termigator u9: FAILED at step " + step + " (" + steps[step].name + "): " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok && !failed)
            fail(what);
        return ok;
    }

    function find(name, item) {
        if (item.objectName === name)
            return item;
        for (const c of item.children) {
            const f = find(name, c);
            if (f)
                return f;
        }
        return null;
    }

    function names() {
        const out = [];
        for (let i = 0; i < panel.list.list.count; ++i) {
            const r = JSON.parse(files.rowInfo(i));
            out.push(r.kind === "up" ? ".." : r.name);
        }
        return out;
    }

    function rowOf(name) {
        return names().indexOf(name);
    }

    function rows(...list) {
        return JSON.stringify(list.map(rowOf));
    }

    function jobs() {
        return JSON.parse(tr.jobsJson());
    }

    function settled() {
        const j = jobs();
        return j.length > 0 && j.every(x => x.state === "done" || x.state === "failed" || x.state === "cancelled");
    }

    function job(part) {
        return jobs().find(j => j.plainLabel.indexOf(part) >= 0) || null;
    }

    function localNames(p) {
        const out = [];
        for (let i = 0; i < p.model.count; ++i)
            out.push(p.model.get(i, "fileName"));
        return out;
    }

    function inDir(path) {
        return files.cwd === path && !files.busy;
    }

    function itemAt(name) {
        return panel.list.list.itemAtIndex(rowOf(name));
    }

    readonly property var steps: [
        // The side panel without a session: the hint, no menu, the shell's hooks are live.
        {
            "name": "panel",
            "fn": () => {
                // The Files panel is created lazily on the first Files tab / SSH session;
                // touch the tab to create it, then come back so the test starts on Hosts.
                shell.sideTabBar.currentIndex = 1;
                shell.sideTabBar.currentIndex = 0;
                panel = find("filesPanel", shell.contentItem);
                if (!expect(panel !== null, "no Files panel"))
                    return;
                files = panel.files;
                tr = panel.transfers;
                area = shell.panesArea;
                expect(area !== null, "no panes area");
                expect(!files.hasTarget && files.status === "Open an SSH session — its files will show up here.", "idle status " + files.status);
                expect(panel.list.menuTexts(-1) === "", "menu without a session " + panel.list.menuTexts(-1));
                expect(shell.openFilesTab !== null && shell.scpTransfer !== null, "hooks not set");
                expect(shell.sideTabBar.currentIndex === 0, "Hosts tab first");
                // The Hosts and Files tabs share the dock's width equally.
                const tb = shell.sideTabBar, a = tb.itemAt(0), b = tb.itemAt(1);
                expect(tb.count === 2 && a.width > 0 && Math.abs(a.width - b.width) <= 1 && Math.abs(a.width + b.width - tb.width) <= 1, "side tabs " + a.width + " + " + b.width + " of " + tb.width);
                shell.sideTabBar.currentIndex = 1;
                shell.requestActivate();
            }
        },
        {
            "name": "panel width",
            "fn": () => {
                if (panel.toolbar.tools.length < 10)
                    return false;
                // The side dock may narrow far; the window still leaves the panes their minimum.
                expect(panel.minimumPanelWidth <= 160 && panel.width + 0.5 >= panel.minimumPanelWidth, "panel " + panel.width + " min " + panel.minimumPanelWidth);
                expect(shell.minimumWidth >= shell.sideMinimumWidth + shell.contentMinimumWidth, "window minimum " + shell.minimumWidth);
                const t = panel.toolbar;
                const rows = {};
                for (const c of t.children)
                    if (c.visible)
                        rows[c.y] = true;
                expect(Object.keys(rows).length === 1, "toolbar wrapped into " + Object.keys(rows).length + " rows");
                expect(t.shown >= 1 && find("toolMore", panel).visible === (t.shown < t.tools.length), "overflow button vs " + t.shown);
                // Wider icons stand for a narrower toolbar: fewer fit, the rest hide behind the » button.
                held.fitting = t.shown;
                held.toolWidth = t.toolWidth;
                t.toolWidth = t.toolWidth * 3;
            }
        },
        {
            "name": "toolbar overflow",
            "fn": () => {
                const t = panel.toolbar;
                const more = find("toolMore", panel);
                const hidden = t.tools.filter(x => !x.visible).length;
                expect(t.shown >= 1 && t.shown < held.fitting, "shown " + t.shown + " was " + held.fitting);
                expect(more.visible && hidden === t.tools.length - t.shown, "overflow button " + more.visible + " hidden " + hidden);
                expect(find("toolDownload", panel).visible, "first tool hidden");
                t.toolWidth = held.toolWidth;
            }
        },
        {
            "name": "toolbar overflow gone",
            "fn": () => {
                const t = panel.toolbar;
                expect(t.shown === held.fitting, "tools did not come back: " + t.shown);
            }
        },
        {
            "name": "target",
            "fn": () => {
                expect(panel.visible, "panel not shown in the Files tab");
                expect(find("remoteEmpty", panel).visible, "no hint without a session");
                files.setTarget(entryJson, "box");
                expect(files.hasTarget && files.label === "box", "target " + files.label);
            }
        },
        {
            "name": "home listed",
            "fn": () => {
                if (files.busy || files.status.indexOf(" shown (") < 0)
                    return false;
                files.navigate(remote);
            }
        },
        // Listing: ".." first, folders before files, hidden shown, 5 columns' data.
        {
            "name": "listing",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                const n = names().join(",");
                expect(n === "..,ro,sub,.hidden,a.txt,big.bin,link", "rows " + n);
                expect(files.status === "box · " + remote + " — 6 shown (6 total, 2 dir(s))", "status " + files.status);
                // In the side dock the window's status bar carries the line, not the panel.
                expect(shell.sideTabBar.currentIndex === 1 && shell.sideStatus === files.status && !find("filesStatus", panel).visible, "status bar " + shell.sideStatus);
                expect(panel.pathField.text === remote, "path bar " + panel.pathField.text);
                const a = itemAt("a.txt");
                expect(a.sizeText === "6 B" && a.mode.startsWith("-rw") && a.mtime !== "" && a.owner.indexOf(":") > 0, "a.txt " + a.sizeText + " " + a.mode + " " + a.owner);
                expect(itemAt("sub").iconName === "folder" && itemAt("link").iconName === "emblem-symbolic-link" && itemAt("a.txt").iconName === "text-x-generic", "icons");
                expect(itemAt("link").tooltip.endsWith(" -> a.txt"), "link tooltip " + itemAt("link").tooltip);
                expect(itemAt("..").iconName === "go-up", "up icon");
                panel.hiddenButton.toggle();
            }
        },
        // Hidden files: the toggle drops dotfiles but keeps "..".
        {
            "name": "hidden off",
            "fn": () => {
                expect(names().join(",") === "..,ro,sub,a.txt,big.bin,link", "hidden off " + names().join(","));
                expect(files.status.indexOf("5 shown (6 total") > 0, "hidden status " + files.status);
                panel.hiddenButton.toggle();
                expect(names().length === 7 && names()[0] === "..", "hidden back");
                files.sortBy(1);
            }
        },
        // Sort like a file manager: size numerically, ".." pinned, folders first either way.
        {
            "name": "sort size",
            "fn": () => {
                let n = names();
                expect(n[0] === ".." && n[1] === "ro" && n[2] === "sub" && n[n.length - 1] === "big.bin", "size asc " + n.join(","));
                files.sortBy(1);
                n = names();
                expect(n[0] === ".." && n.indexOf("big.bin") === 3 && n.indexOf("sub") < n.indexOf("big.bin") && !files.ascending, "size desc " + n.join(","));
                files.sortBy(0);
                n = names();
                expect(n.join(",") === "..,ro,sub,.hidden,a.txt,big.bin,link" && files.ascending && files.sortColumn === 0, "name asc " + n.join(","));
                // Optional columns: off by default, chosen in the header menu, remembered.
                expect(!panel.list.showPermissions && !panel.list.showOwner, "columns off");
                panel.setColumn("permissions", true);
                panel.setColumn("owner", true);
                expect(panel.list.showPermissions && panel.list.showOwner && files.shownColumns() === '["permissions","owner"]', "columns " + files.shownColumns());
            }
        },
        // Column widths: a real mouse drag on the grip on a column's right edge sets that column's
        // minimum width and Name takes the difference; the width is remembered in
        // launch.files_column_widths.
        {
            "name": "column widths",
            "fn": () => {
                const g = find("grip1", panel);
                if (!expect(g !== null && g.visible, "no Size grip"))
                    return;
                const l = panel.list;
                expect(l.widthFor("size") === 74, "default size width " + l.widthFor("size"));
                const nameBefore = l.nameWidth;
                const p = g.mapToItem(null, g.width / 2, g.height / 2);
                Probe.drag(shell, p.x, p.y, p.x - 30, p.y, Qt.LeftButton, Qt.NoModifier);
                expect(l.widthFor("size") === 44, "drag narrowed to " + l.widthFor("size"));
                const saved = JSON.parse(files.columnWidths());
                expect(saved.size === 44 && saved.name === undefined, "not saved (or more than the dragged column): " + files.columnWidths());
                expect(l.widthFor("modified") === 132 && l.widthFor("owner") === 112 && l.widthFor("permissions") === 92, "another column changed");
                // Name takes what Size gave up (or stays at its minimum when the panel is too narrow).
                expect(l.nameWidth === Math.max(l.nameMin, Math.floor(l.columnsRoom - l.fixedWidth())) && l.nameWidth >= nameBefore, "Name " + l.nameWidth + " (was " + nameBefore + ")");
                // Every column has a grip, the last one (Owner now) too.
                expect(["grip0", "grip1", "grip2", "grip3", "grip4"].every(n => find(n, panel) !== null && find(n, panel).visible), "grips");
            }
        },
        // Dragging left below the minimum stops at it; the columns to the right do not move.
        {
            "name": "column minimum",
            "fn": () => {
                const g = find("grip1", panel);
                if (!expect(g !== null && g.visible, "no Size grip"))
                    return;
                const p = g.mapToItem(null, g.width / 2, g.height / 2);
                Probe.drag(shell, p.x, p.y, p.x - 400, p.y, Qt.LeftButton, Qt.NoModifier);
                expect(panel.list.widthFor("size") === 40, "size " + panel.list.widthFor("size"));
                expect(panel.list.widthFor("permissions") === 92 && panel.list.widthFor("owner") === 112 && panel.list.widthFor("modified") === 132, "others moved");
                const saved = JSON.parse(files.columnWidths());
                expect(saved.size === 40, "minimum not saved: " + files.columnWidths());
            }
        },
        // The columns the next steps work with: all at their minimum.
        {
            "name": "narrow columns",
            "fn": () => {
                files.resetColumnWidths();
                panel.list.widths = {
                    "size": 40,
                    "modified": 40,
                    "permissions": 40,
                    "owner": 40
                };
            }
        },
        // (The header lays out on the next frame: the grips are measured from there on.)
        {
            "name": "name takes the room",
            "fn": () => {
                const l = panel.list;
                const header = find("listHeader0", panel).parent;
                const owner = find("listHeader4", panel);
                const atFrame = () => Math.abs(owner.x + owner.width - header.width) < 0.5;
                expect(l.nameWidth + l.fixedWidth() === l.columnsRoom && l.rowWidth === l.viewWidth, "not fitted: " + l.nameWidth + " + " + l.fixedWidth() + " / " + l.columnsRoom);
                expect(atFrame(), "owner ends at " + (owner.x + owner.width) + ", header " + header.width);
                const drag = (name, dx) => {
                    const g = find(name, panel);
                    const p = g.mapToItem(null, g.width / 2, g.height / 2);
                    Probe.drag(shell, p.x, p.y, p.x + dx, p.y, Qt.LeftButton, Qt.NoModifier);
                };
                // Name narrower: only its minimum drops; while the panel is wide it keeps filling it.
                const name = l.nameWidth;
                drag("grip0", -(name - l.nameMinimumWidth));
                expect(l.nameMin === l.nameMinimumWidth && l.nameWidth === name, "name " + l.nameWidth + ", min " + l.nameMin);
                expect(JSON.parse(files.columnWidths()).name === l.nameMinimumWidth, "name minimum not saved: " + files.columnWidths());
                // A wider Permissions, and a wider last column: Name gives the room (the panel is
                // narrow here, Name is at its minimum, so the list outgrows it).
                const room = () => Math.max(l.nameMin, Math.floor(l.columnsRoom - l.fixedWidth()));
                drag("grip3", 20);
                expect(l.widthFor("permissions") === 60 && l.nameWidth === room(), "permissions " + l.widthFor("permissions") + ", name " + l.nameWidth);
                drag("grip4", 10);
                expect(l.widthFor("owner") === 50 && l.nameWidth === room() && l.rowWidth === Math.max(l.viewWidth, l.nameMin + l.fixedWidth() + 8), "owner " + l.widthFor("owner") + ", name " + l.nameWidth);
                // "Reset column widths" forgets the widths.
                l.resetWidths();
                expect(Object.keys(JSON.parse(files.columnWidths())).length === 0, "not reset: " + files.columnWidths());
                l.widths = {
                    "size": 40,
                    "modified": 40,
                    "permissions": 40,
                    "owner": 40
                };
            }
        },
        // Past the room the columns are at their minimum and the list scrolls sideways with its
        // header (like a tree view); Name's width, once dragged, is its minimum too.
        {
            "name": "name grip",
            "fn": () => {
                const l = panel.list;
                const header = find("listHeader0", panel).parent;
                const owner = find("listHeader4", panel);
                expect(Math.abs(owner.x + owner.width - header.width) < 0.5, "owner left the frame: " + (owner.x + owner.width) + " / " + header.width);
                const g = find("grip0", panel);
                const name = l.nameWidth;
                const p = g.mapToItem(null, g.width / 2, g.height / 2);
                Probe.drag(shell, p.x, p.y, p.x + 400, p.y, Qt.LeftButton, Qt.NoModifier);
                expect(l.nameWidth === name + 400, "name " + l.nameWidth + " != " + name + " + 400");
                expect(l.rowWidth === l.nameWidth + l.fixedWidth() + 8 && l.rowWidth > l.viewWidth, "no overflow: row " + l.rowWidth + ", view " + l.viewWidth);
                expect(l.list.contentWidth === l.rowWidth && header.width === l.rowWidth - 8, "scroll width: content " + l.list.contentWidth + ", header " + header.width + ", row " + l.rowWidth);
            }
        },
        // Scrolled to the end, the horizontal bar is at its end too (the last column then clears
        // the vertical bar): the bar spans the content plus the margin kept for it. A step of its
        // own: the bar shows up one frame after the overflow.
        {
            "name": "h scroll",
            "fn": () => {
                const l = panel.list;
                const hbar = find("remoteHBar", panel);
                expect(hbar !== null && hbar.visible, "no horizontal bar: " + hbar + " row " + l.rowWidth + " view " + l.viewWidth);
                l.list.contentX = l.list.contentWidth + l.list.rightMargin - l.list.width;
                expect(Math.abs(hbar.position + hbar.size - 1) < 0.001, "bar at " + hbar.position + " + " + hbar.size + " with the list at its end");
                expect(l.scrollBarWidth <= 10, "room kept for the vertical bar: " + l.scrollBarWidth);
                l.list.contentX = 0;
                l.resetWidths();
            }
        },
        // A double click on a grip fits the column to its widest cell (or its header).
        {
            "name": "fit column",
            "fn": () => {
                const l = panel.list;
                const g = find("grip1", panel);
                const p = g.mapToItem(null, g.width / 2, g.height / 2);
                Probe.click(shell, p.x, p.y, Qt.LeftButton, 2, 0);
                const fit = l.fitWidth(1);
                expect(fit > 40 && fit < 74 && l.widthFor("size") === fit, "size fits " + l.widthFor("size") + " vs " + fit);
                expect(JSON.parse(files.columnWidths()).size === fit, "fit not saved: " + files.columnWidths());
                // Name: icon + the longest name; never below its minimum.
                const n = find("grip0", panel);
                const q = n.mapToItem(null, n.width / 2, n.height / 2);
                Probe.click(shell, q.x, q.y, Qt.LeftButton, 2, 0);
                expect(l.nameMin === Math.max(l.nameMinimumWidth, l.fitWidth(0)), "name fit " + l.nameMin + " vs " + l.fitWidth(0));
                l.resetWidths();
            }
        },
        // The context menu, texts and order.
        {
            "name": "menus",
            "fn": () => {
                const tail = "Upload files here…|Upload folder here…|New folder…|Refresh";
                const m = row => panel.list.menuTexts(row);
                expect(m(rowOf("a.txt")) === "Edit (auto-upload on save)|Download…|Rename…|Permissions…|Delete…|-|Copy remote path|-|" + tail, "file menu " + m(rowOf("a.txt")));
                expect(m(rowOf("sub")) === "Open|Download folder…|Rename…|Permissions…|Delete…|-|Copy remote path|-|" + tail, "folder menu " + m(rowOf("sub")));
                expect(m(rowOf("link")) === "Open|Edit (auto-upload on save)|Download…|Rename…|Permissions…|Delete…|-|Copy remote path|-|" + tail, "link menu " + m(rowOf("link")));
                expect(m(0) === "Open parent directory", "up menu " + m(0));
                expect(m(-1) === tail, "empty-space menu " + m(-1));
                panel.list.selection = [rowOf("a.txt"), rowOf("big.bin")];
                expect(m(rowOf("a.txt")) === "Download 2 items…|Delete 2 items…|Permissions of 2 items…|-|" + tail, "multi menu " + m(rowOf("a.txt")));
                panel.list.setCurrent(-1, true);
                // Nothing selected: the same hints as the real list.
                panel.downloadSelected();
            }
        },
        {
            "name": "nothing selected",
            "fn": () => {
                expect(panel.messageBox.visible && panel.messageBox.text === "Select remote files or folders first.", "info box " + panel.messageBox.text);
                panel.messageBox.close();
                panel.renameSelected();
            }
        },
        {
            "name": "nothing selected rename",
            "fn": () => {
                expect(panel.messageBox.visible && panel.messageBox.text === "Select a remote file or folder first.", "rename info " + panel.messageBox.text);
                panel.messageBox.close();
                // Double click: a folder is entered, ".." goes back.
                panel.activate(rowOf("sub"));
            }
        },
        {
            "name": "enter folder",
            "fn": () => {
                if (!inDir(remote + "/sub"))
                    return false;
                const n = names();
                expect(n.length === 3 && n[0] === ".." && n[1] === "in.txt" && n[2] === "odd\n!touch PWNED", "sub rows " + JSON.stringify(n));
                panel.activate(0);
            }
        },
        {
            "name": "back up",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                // The path bar navigates, a bad path says why and keeps the listing.
                files.navigate(remote + "/nope");
            }
        },
        {
            "name": "bad path",
            "fn": () => {
                if (files.busy)
                    return false;
                expect(files.status.startsWith("Listing failed: "), "bad path status " + files.status);
                expect(files.cwd === remote && names().length === 7, "listing kept");
                // Uploads: a new file goes straight in.
                panel.uploadPaths([local + "/up1.txt"], -1);
            }
        },
        {
            "name": "uploaded",
            "fn": () => {
                if (!settled() || rowOf("up1.txt") < 0)
                    return false;
                const j = job("up1.txt");
                expect(j.state === "done" && j.label === "↑ up1.txt" && j.stateText === "done" && j.percent === 100, "upload job " + JSON.stringify(j));
                expect(tr.activeCount === 0 && shell.activeTransfers === 0, "active after upload " + tr.activeCount);
                // A name that exists asks; No skips, Yes overwrites.
                const q = files.uploadClashes(JSON.stringify([local + "/a.txt", local + "/up1.txt"]), -1);
                expect(q === "2 item(s) already exist in " + remote + ":\n  • a.txt\n  • up1.txt\n\nYes = overwrite them, No = skip them.", "clash text " + q);
                const before = jobs().length;
                files.uploadPaths(JSON.stringify([local + "/a.txt"]), -1, true);
                held.before = before;
                tr.clearFinished();
            }
        },
        {
            "name": "skipped",
            "fn": () => {
                expect(jobs().length === 0, "a skipped clash queued a job");
                panel.uploadPaths([local + "/a.txt"], -1); // no clash question in this call: it is asked by the dialog
                // (uploadPaths asks first; answer Yes through the dialog.)
            }
        },
        {
            "name": "clash asked",
            "fn": () => {
                if (!panel.askBox.visible)
                    return false;
                expect(panel.askBox.text.startsWith("1 item(s) already exist in " + remote + ":"), "asked " + panel.askBox.text);
                expect(panel.askBox.buttons === (MessageDialog.Yes | MessageDialog.No | MessageDialog.Cancel), "clash buttons");
                panel.askBox.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
                panel.askBox.close();
            }
        },
        // Dropping onto a folder row lists that folder first; a name there asks too.
        {
            "name": "subfolder upload",
            "fn": () => {
                if (!settled())
                    return false;
                panel.uploadPaths([local + "/in.txt"], rowOf("sub"));
            }
        },
        {
            "name": "subfolder clash asked",
            "fn": () => {
                if (!panel.askBox.visible)
                    return false;
                expect(panel.askBox.text.startsWith("1 item(s) already exist in " + remote + "/sub:\n  • in.txt"), "subfolder asked " + panel.askBox.text);
                panel.askBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
                panel.askBox.close();
            }
        },
        {
            "name": "overwritten",
            "fn": () => {
                if (!settled())
                    return false;
                expect(jobs().length === 1 && jobs()[0].state === "done", "overwrite job " + tr.jobsJson());
                // A folder uploads recursively; a drop onto a folder row goes into that folder.
                panel.uploadPaths([local + "/lfolder"], -1);
            }
        },
        {
            "name": "folder uploaded",
            "fn": () => {
                if (!settled() || rowOf("lfolder") < 0)
                    return false;
                expect(job("lfolder").label === "↑ lfolder/" && job("lfolder").state === "done", "folder job " + tr.jobsJson());
                // Drag and drop in: file URLs from a file manager (remote URLs refuse the drop).
                const y = itemAt("sub").y + 4;
                expect(!panel.list.dropUrls(["https://example.org/x"], y), "a web URL was taken");
                expect(!panel.list.dropUrls([], y), "an empty drop was taken");
                expect(panel.list.dropUrls(["file://" + local + "/up2.txt"], y), "drop refused");
            }
        },
        {
            "name": "dropped into folder",
            "fn": () => {
                if (!settled() || job("up2.txt") === null)
                    return false;
                expect(job("up2.txt").state === "done", "drop job " + tr.jobsJson());
                tr.clearFinished();
                // The drag payload carries host + folder; a payload from before a folder change is stale.
                const payload = files.dragPayload(rows("a.txt", "sub"));
                held.payload = payload;
                const ok = JSON.parse(files.dropRows(payload));
                expect(ok.rows.length === 2, "payload rows " + payload);
                panel.activate(rowOf("sub"));
            }
        },
        {
            "name": "stale payload",
            "fn": () => {
                if (!inDir(remote + "/sub"))
                    return false;
                expect(JSON.parse(files.dropRows(held.payload)).error === staleText, "stale payload accepted");
                expect(JSON.parse(files.dropRows("not json")).error === staleText, "garbage payload accepted");
                panel.downloadDropped(held.payload, local);
                expect(files.status === staleText, "stale status " + files.status);
                panel.activate(0);
            }
        },
        // A real double click on a file asks where to save it (no transfer before that).
        {
            "name": "double click a file",
            "fn": () => {
                const it = itemAt("a.txt");
                if (!inDir(remote) || !it)
                    return false;
                held.jobsBefore = jobs().length;
                const p = it.mapToItem(null, 30, it.height / 2);
                Probe.click(shell, p.x, p.y, Qt.LeftButton, 2, 0);
            }
        },
        {
            "name": "double click asks",
            "fn": () => {
                const d = panel.saveDialog;
                if (!d.visible)
                    return false;
                expect(d.title === "Download a.txt" && d.row === rowOf("a.txt"), "dialog " + d.title + " row " + d.row);
                expect(jobs().length === held.jobsBefore, "a transfer started before the answer");
                d.reject();
            }
        },
        // Downloads: a single file, then a folder with a file in one go; clashes ask.
        {
            "name": "download one",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                files.downloadEntry(rowOf("a.txt"), local + "/dl_a.txt");
            }
        },
        {
            "name": "downloaded",
            "fn": () => {
                if (!settled())
                    return false;
                expect(job("a.txt").state === "done" && files.status === "Downloaded a.txt → " + local + "/dl_a.txt", "download " + files.status);
                tr.clearFinished();
                const sel = rows("sub", "big.bin");
                expect(files.downloadClashes(sel, local + "/dlmany") === "", "clash with nothing there");
                files.downloadInto(sel, local + "/dlmany", false);
            }
        },
        {
            "name": "downloaded many",
            "fn": () => {
                if (!settled() || jobs().length < 2)
                    return false;
                expect(jobs().every(j => j.state === "done") && job("sub/") !== null, "many " + tr.jobsJson());
                const q = files.downloadClashes(rows("sub", "big.bin"), local + "/dlmany");
                expect(q === "2 item(s) already exist in " + local + "/dlmany:\n  • sub\n  • big.bin\n\nYes = overwrite them, No = skip them.", "download clash " + q);
                tr.clearFinished();
            }
        },
        // mkdir / rename / delete: refresh on success, the reason on failure (never clobbers).
        {
            "name": "mkdir",
            "fn": () => {
                files.mkdir(" newdir ");
                files.mkdir("a/b"); // refused: no slash in a name
            }
        },
        {
            "name": "mkdir listed",
            "fn": () => {
                if (rowOf("newdir") < 0)
                    return false;
                files.rename(rowOf("newdir"), "a.txt");
            }
        },
        {
            "name": "rename refused",
            "fn": () => {
                if (files.busy)
                    return false;
                expect(files.status === "Rename failed: 'a.txt' already exists" && rowOf("newdir") >= 0, "rename clobber " + files.status);
                files.rename(rowOf("newdir"), "renamed");
            }
        },
        {
            "name": "renamed",
            "fn": () => {
                if (rowOf("renamed") < 0)
                    return false;
                expect(rowOf("newdir") < 0, "old name kept");
                const one = files.deleteQuestion(rows("renamed"));
                expect(one === "Delete remote directory (recursively!) “renamed”?\nThis cannot be undone.", "delete question " + one);
                const file = files.deleteQuestion(rows("up1.txt"));
                expect(file === "Delete remote file “up1.txt”?\nThis cannot be undone.", "delete file question " + file);
                const many = files.deleteQuestion(rows("renamed", "up1.txt"));
                expect(many === "Delete 2 remote item(s) (1 folder(s), recursively!)?\n  • renamed/\n  • up1.txt\nThis cannot be undone.", "delete many " + many);
                // The dialog answers No: nothing happens.
                panel.deleteRows([rowOf("renamed")]);
            }
        },
        {
            "name": "delete asked",
            "fn": () => {
                if (!panel.askBox.visible)
                    return false;
                expect(panel.askBox.buttons === (MessageDialog.Yes | MessageDialog.No), "delete buttons");
                panel.askBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
                panel.askBox.close();
                files.deleteRows(rows("renamed"));
            }
        },
        {
            "name": "deleted",
            "fn": () => {
                if (rowOf("renamed") >= 0)
                    return false;
                // chmod through the dialog's numbers.
                files.chmod(rows("up1.txt"), 384, false);
            }
        },
        {
            "name": "chmod",
            "fn": () => {
                if (itemAt("up1.txt").mode !== "-rw-------")
                    return false;
                const i = JSON.parse(files.permissionsInfo(rows("up1.txt", "sub")));
                expect(i.what === "2 items" && i.mode === 384 && i.anyDir, "permissions info " + JSON.stringify(i));
                expect(JSON.parse(files.permissionsInfo(rows("a.txt"))).what === "a.txt", "single name");
                // The dialog: boxes <-> octal, special bits kept, recursive only for folders.
                const d = panel.permissionsDialog;
                held.perm = null;
                d.ask("3 items", 492, true, (mode, rec) => held.perm = [mode, rec]);
                expect(d.octal.text === "0754" && d.boxes.itemAt(0).checked && !d.boxes.itemAt(8).checked && d.boxes.itemAt(7).checked === false, "octal from mode " + d.octal.text);
                d.boxes.itemAt(8).toggle();
                expect(d.octal.text === "0755", "boxes to octal " + d.octal.text);
                d.octal.text = "0640";
                d.fromOctal();
                const on = [0, 1, 3].every(k => d.boxes.itemAt(k).checked) && [2, 4, 5, 6, 7, 8].every(k => !d.boxes.itemAt(k).checked);
                expect(on, "octal to boxes");
                expect(d.recursive.visible, "recursive offered for a folder");
                d.recursive.checked = true;
                d.accept();
                expect(held.perm !== null && held.perm[0] === 416 && held.perm[1] === true, "dialog result " + JSON.stringify(held.perm));
                d.ask("x", 2541, false, () => {});
                expect(d.octal.text === "4755" && !d.recursive.visible, "special bits " + d.octal.text);
                d.boxes.itemAt(0).toggle();
                expect(d.octal.text === "4355", "special bits kept " + d.octal.text);
                d.octal.text = "9";
                expect(d.mode() === -1, "bad octal");
                d.reject();
                files.chmod(rows("up1.txt"), 420, false);
            }
        },
        {
            "name": "chmod back",
            "fn": () => {
                if (itemAt("up1.txt").mode !== "-rw-r--r--")
                    return false;
                // Edit in place: the working copy opens in the "editor" (a script that appends a
                // line), and the save is uploaded 700 ms later.
                files.editEntry(rowOf("a.txt"));
            }
        },
        {
            "name": "edited",
            "fn": () => {
                const j = job("a.txt (save)");
                if (j === null || j.state === "queued" || j.state === "running")
                    return false;
                expect(j.state === "done" && j.label === "↑ ✎ a.txt (save)", "save job " + JSON.stringify(j));
                // The transfer-finished handler overwrites "Editing …" with "Downloaded …" right after (kept).
                expect(files.editing && files.status.startsWith("Downloaded ✎ a.txt → "), "edit status " + files.status);
                tr.clearFinished();
            }
        },
        // The "editor" changes the server copy, then saves again: asked, No keeps the server's.
        {
            "name": "edit conflict asked",
            "fn": () => {
                if (!panel.askBox.visible)
                    return false;
                expect(panel.askBox.text.startsWith("a.txt changed on the server since you opened it."), "edit conflict " + panel.askBox.text);
                expect(job("a.txt (save)") === null, "the save went up before the answer");
                panel.askBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
                panel.askBox.close();
            }
        },
        // ---- the slow phase: a remote file named SLOW makes the stand-in ssh wait before a transfer
        {
            "name": "slow marker",
            "fn": () => {
                panel.uploadPaths([local + "/SLOW"], -1);
            }
        },
        {
            "name": "slow marker up",
            "fn": () => {
                if (!settled() || rowOf("SLOW") < 0)
                    return false;
                tr.clearFinished();
                files.downloadEntry(rowOf("big.bin"), local + "/resume/big.bin");
            }
        },
        {
            "name": "download delayed",
            "fn": () => {
                const j = job("big.bin");
                if (j === null || j.state !== "running")
                    return false;
                expect(tr.activeCount === 1 && shell.activeTransfers === 1, "counts " + tr.activeCount + " " + shell.activeTransfers);
                expect(tr.question() === "1 file transfer(s) are still in progress in this tab. Stop them and close it?", "close question " + tr.question());
                expect(tr.title() === "Transfers — 1 active" && j.stateText === "starting…", "title " + tr.title());
                held.dl = j.id;
                tr.cancelIds(JSON.stringify([j.id]));
            }
        },
        {
            "name": "cancelled",
            "fn": () => {
                const j = job("big.bin");
                if (j.state !== "cancelled")
                    return false;
                expect(tr.activeCount === 0 && tr.question() === "", "still active after cancel");
                // Resume with sftp reget once the host is quick again.
                files.deleteRows(rows("SLOW"));
            }
        },
        {
            "name": "slow removed",
            "fn": () => {
                if (rowOf("SLOW") >= 0)
                    return false;
                tr.resume(held.dl);
            }
        },
        {
            "name": "resumed",
            "fn": () => {
                const j = job("big.bin (resume)");
                if (j === null || j.state === "queued" || j.state === "running")
                    return false;
                expect(j.state === "done" && job("big.bin") !== null && jobs().length === 1, "resume " + tr.jobsJson());
                tr.clearFinished();
                // A name with a line break is never put in the sftp batch file.
                files.downloadInto(rows("sub"), "/nonexistent-termigator-dir", false);
            }
        },
        {
            "name": "k15 setup",
            "fn": () => {
                if (!settled())
                    return false;
                expect(jobs()[0].state === "failed", "download into nowhere " + tr.jobsJson());
                tr.clearFinished();
                // Retry of a failed upload: the folder is read-only at first, then chmod opens it.
                panel.uploadPaths([local + "/up2.txt"], rowOf("ro"));
            }
        },
        {
            "name": "upload failed",
            "fn": () => {
                if (!settled())
                    return false;
                const j = jobs()[0];
                expect(j.state === "failed" && j.stateText.startsWith("failed: "), "read-only upload " + JSON.stringify(j));
                held.failed = j.id;
                files.chmod(rows("ro"), 493, false);
            }
        },
        {
            "name": "ro opened",
            "fn": () => {
                if (itemAt("ro").mode !== "drwxr-xr-x")
                    return false;
                tr.retry(held.failed);
            }
        },
        {
            "name": "retried",
            "fn": () => {
                const j = jobs().find(x => x.plainLabel === "up2.txt");
                if (!j || j.state === "queued" || j.state === "running")
                    return false;
                expect(j.state === "done" && jobs().filter(x => x.plainLabel === "up2.txt").length === 1, "retry " + tr.jobsJson());
                tr.clearFinished();
                expect(jobs().length === 0, "clear finished");
            }
        },
        // The line-break name: build it by hand, as the panel would for a failed download.
        {
            "name": "k15 odd name",
            "fn": () => {
                panel.activate(rowOf("sub"));
            }
        },
        {
            "name": "k15 in sub",
            "fn": () => {
                if (!inDir(remote + "/sub"))
                    return false;
                files.downloadInto(rows("odd\n!touch PWNED"), "/nonexistent-termigator-dir", false);
            }
        },
        {
            "name": "k15 failed",
            "fn": () => {
                if (!settled())
                    return false;
                const j = jobs()[0];
                expect(j.state === "failed", "odd download should fail " + JSON.stringify(j));
                tr.resume(j.id);
            }
        },
        {
            "name": "k15 refused",
            "fn": () => {
                const j = jobs().find(x => x.plainLabel.indexOf("(resume)") >= 0);
                if (!j || j.state === "queued" || j.state === "running")
                    return false;
                expect(j.state === "failed" && j.stateText === "failed: " + lineBreakText, "k15 " + JSON.stringify(j));
                tr.clearFinished();
                panel.activate(0);
            }
        },
        // The scheduler: two run at a time, the rest wait (SLOW keeps them running).
        {
            "name": "parallel",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                panel.uploadPaths([local + "/SLOW"], -1);
            }
        },
        {
            "name": "parallel marker",
            "fn": () => {
                if (!settled() || rowOf("SLOW") < 0)
                    return false;
                tr.clearFinished();
                panel.uploadPaths([local + "/bulk1.bin", local + "/bulk2.bin", local + "/bulk3.bin", local + "/bulk4.bin"], -1);
            }
        },
        {
            "name": "parallel limit",
            "fn": () => {
                if (jobs().length < 4)
                    return false;
                const running = jobs().filter(j => j.state === "running").length;
                const queued = jobs().filter(j => j.state === "queued").length;
                expect(running === 2 && queued === 2 && tr.activeCount === 4, "parallel " + running + "/" + queued);
                tr.cancelIds("[]"); // nothing selected: all active ones
            }
        },
        {
            "name": "all cancelled",
            "fn": () => {
                if (!jobs().every(j => j.state === "cancelled") || tr.activeCount !== 0)
                    return false;
                expect(panel.transfersView.jobs.count === 4, "view rows " + panel.transfersView.jobs.count);
                tr.clearFinished();
                expect(panel.transfersView.jobs.count === 0 && !panel.transfersView.visible, "view not emptied");
                files.deleteRows(rows("SLOW"));
            }
        },
        // Bookmarks and the remembered folder.
        {
            "name": "bookmarks",
            "fn": () => {
                if (rowOf("SLOW") >= 0)
                    return false;
                let s = JSON.parse(files.bookmarkState());
                expect(s.canAdd && !s.canRemove && s.marks.length === 0, "bookmark state " + JSON.stringify(s));
                files.addBookmark();
                s = JSON.parse(files.bookmarkState());
                expect(!s.canAdd && s.canRemove && s.marks.length === 1 && s.marks[0] === remote, "bookmarked " + JSON.stringify(s));
                panel.fillBookmarks();
                expect(panel.bookmarkMenu.count === 4, "bookmark menu " + panel.bookmarkMenu.count);
                files.navigate(remote + "/lfolder");
            }
        },
        {
            "name": "bookmark go",
            "fn": () => {
                if (!inDir(remote + "/lfolder"))
                    return false;
                const s = JSON.parse(files.bookmarkState());
                expect(s.canAdd && !s.canRemove, "other folder " + JSON.stringify(s));
                files.navigate(s.marks[0]);
            }
        },
        // Follow terminal folder: a tab whose shell reports its directory (OSC 7) moves the panel.
        {
            "name": "follow",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                shell.sideTabBar.currentIndex = 0;
                held.t1 = area.addTab({
                    "title": "t1",
                    "inner": "printf '\\033]7;file://h" + remote + "/lfolder\\007'; exec cat",
                    "conn": entry
                });
            }
        },
        {
            "name": "followed",
            "fn": () => {
                if (!inDir(remote + "/lfolder"))
                    return false;
                expect(shell.sideTabBar.currentIndex === 1, "an SSH tab brings the Files tab to the front");
                expect(files.hasTarget && files.label === "t1", "label " + files.label);
                // The toggle off: the session's cd is ignored.
                files.followDirectory(remote, false);
            }
        },
        {
            "name": "follow off",
            "fn": () => {
                expect(files.cwd === remote + "/lfolder" && !files.busy, "followed while off");
                panel.followButton.toggle();
                panel.followDirectory(remote);
                expect(files.cwd === remote + "/lfolder", "button off ignored? " + files.cwd);
                panel.followButton.toggle();
                panel.followDirectory(remote);
            }
        },
        // Going elsewhere by hand pauses following (the button shows off); a prompt in the same
        // folder keeps the pause, the button or a cd to another folder ends it.
        {
            "name": "follow pause",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                expect(panel.followButton.checked && !panel.followPaused, "following before the pause");
                files.navigate(remote + "/lfolder");
            }
        },
        {
            "name": "follow paused",
            "fn": () => {
                if (!inDir(remote + "/lfolder"))
                    return false;
                expect(!panel.followButton.checked && panel.followPaused, "not paused by going elsewhere");
                panel.followDirectory(remote); // the same folder again: a new prompt, no cd
                expect(files.cwd === remote + "/lfolder" && panel.followPaused, "a prompt ended the pause");
                panel.setFollow(true);
            }
        },
        {
            "name": "follow resumed",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                expect(panel.followButton.checked && !panel.followPaused, "the button did not resume");
                files.goUp();
            }
        },
        {
            "name": "follow paused by up",
            "fn": () => {
                if (files.busy || files.cwd === remote)
                    return false;
                expect(panel.followPaused, "going up did not pause");
                panel.followDirectory(remote + "/lfolder"); // a cd in the terminal
            }
        },
        {
            "name": "follow cd resumes",
            "fn": () => {
                if (!inDir(remote + "/lfolder"))
                    return false;
                expect(panel.followButton.checked && !panel.followPaused, "a cd did not resume");
                panel.followDirectory(remote);
            }
        },
        {
            "name": "follow on",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                // A local shell: the side panel lets go of the host and shows this machine's files.
                held.t2 = area.addTab({
                    "title": "local",
                    "inner": "exec cat"
                });
            }
        },
        {
            "name": "no ssh tab",
            "fn": () => {
                expect(!files.hasTarget && shell.sideTabBar.currentIndex === 1 && panel.list.list.count === 0, "side panel kept the host");
                expect(panel.localMode && panel.local !== null, "no local files for a local shell");
                expect(files.status === "Open an SSH session — its files will show up here.", "status " + files.status);
                expect(!shell.shellActions.actions["session.files"].enabled, "session.files enabled without an SSH tab");
                // Back to the SSH tab: the same host, the folder is kept.
                const pv = area.viewOf(held.t1);
                area.select(pv, pv.tabIds.indexOf(held.t1.tabId));
            }
        },
        {
            "name": "ssh tab again",
            "fn": () => {
                // The host is bound again and follows the session's folder (OSC 7: lfolder).
                if (!inDir(remote + "/lfolder"))
                    return false;
                expect(shell.sideTabBar.currentIndex === 1, "Files tab to the front again");
                files.navigate(remote);
            }
        },
        // Two panes: the local folder beside it, upload with the arrow, download into it.
        {
            "name": "two panes",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                panel.twoPaneButton.toggle();
                expect(panel.twoPane && panel.local !== null, "no local pane");
                expect(files.twoPane("files_two_pane_side"), "two panes not remembered");
                panel.local.setDir(local);
            }
        },
        // In the local pane: widths are minimums that Name fills around, saved in the store the
        // remote list shares, fitted to the widest cell, and reset from the header menu.
        {
            "name": "local columns",
            "fn": () => {
                const p = panel.local;
                expect(p.nameWidth === Math.max(p.nameMin, Math.floor(p.viewWidth - 8 - p.fixedWidth())), "name fills: " + p.nameWidth);
                const before = p.widthFor("size");
                p.setWidth("size", before + 30);
                p.saveWidths();
                expect(p.widthFor("size") === before + 30 && JSON.parse(files.columnWidths()).size === before + 30, "size not kept: " + files.columnWidths());
                expect(panel.list.widthFor("size") === before + 30, "remote list did not reload: " + panel.list.widthFor("size"));
                const fit = p.fitWidth(1);
                expect(fit > 40 && fit < 120, "fit " + fit);
                p.setWidth("size", 600);
                expect(p.rowWidth > p.viewWidth && p.rowWidth === p.nameMin + p.fixedWidth() + 8, "no overflow: row " + p.rowWidth + ", view " + p.viewWidth);
                p.resetWidths();
                expect(p.widthFor("size") === before && files.columnWidths() === "{}", "not reset: " + files.columnWidths());
                expect(panel.list.widthFor("size") === before, "remote not reset");
            }
        },
        // A local shell in front (FilesSidePanel): only this machine's folder, the same toolbar
        // without what needs a server.
        {
            "name": "local mode",
            "fn": () => {
                panel.localMode = true;
                expect(panel.local !== null && panel.local.standalone, "no standalone local pane");
                expect(!find("remotePath", panel).visible && panel.toolbar.visible, "remote path shown or no toolbar");
                const shown = ["toolRefresh", "toolMkdir", "toolRename", "toolDelete", "toolFollow", "toolHidden"];
                const gone = ["toolDownload", "toolUpload", "toolSync", "toolTwoPane"];
                for (const n of shown)
                    expect(find(n, panel.toolbar).visible || find("toolMore", panel.toolbar).visible, n + " missing in local mode");
                for (const n of gone)
                    expect(!find(n, panel.toolbar).visible, n + " shown in local mode");
                expect(!find("localUpload", panel.local).visible, "upload arrow shown");
                panel.localMode = false;
                expect(find("remotePath", panel).visible && panel.toolbar.visible, "host controls hidden");
            }
        },
        // The local pane's context menu (a local shell shows only this pane, so on Windows this is
        // the whole Files tree): a real right-click opens it, and New folder / Rename / Delete work.
        {
            "name": "local menu",
            "fn": () => {
                const p = panel.local;
                if (!held.localMenuDir) {
                    p.setDir(local);
                    held.localMenuDir = true;
                    return false;
                }
                if (p.model.count === 0)
                    return false;
                expect(p.menuTexts(-1) === "New folder…|Refresh", "empty-space menu " + p.menuTexts(-1));
                const row = p.menuTexts(0);
                expect(row.indexOf("Rename…") > 0 && row.indexOf("Delete…") > 0 && row.indexOf("Copy path") > 0, "row menu " + row);
                if (!held.localMenuClicked) {
                    const pt = p.list.mapToItem(null, 40, 10);
                    Probe.click(shell, pt.x, pt.y, Qt.RightButton, 1, 0);
                    held.localMenuClicked = true;
                    return false;
                }
                expect(p.contextMenu !== null && p.contextMenu.visible, "the right-click did not open the menu");
                p.contextMenu.close();
            }
        },
        {
            "name": "local new folder",
            "fn": () => {
                const p = panel.local;
                if (!held.localMkdir) {
                    p.makeFolder();
                    expect(panel.prompt.visible && panel.prompt.title === "New local folder", "no name prompt");
                    panel.prompt.field.text = "made_by_menu";
                    panel.prompt.accept();
                    held.localMkdir = true;
                    return false;
                }
                if (!localHelper.isDir(p.path + "/made_by_menu"))
                    return false;
            }
        },
        {
            "name": "local rename",
            "fn": () => {
                const p = panel.local;
                if (!held.localRename) {
                    p.renameEntry(p.path + "/made_by_menu");
                    expect(panel.prompt.visible && panel.prompt.field.text === "made_by_menu", "rename prompt " + panel.prompt.field.text);
                    panel.prompt.field.text = "renamed_by_menu";
                    panel.prompt.accept();
                    held.localRename = true;
                    return false;
                }
                if (!localHelper.isDir(p.path + "/renamed_by_menu"))
                    return false;
                expect(!localHelper.isDir(p.path + "/made_by_menu"), "the old name is still there");
                // The list shows the change without a manual refresh (Windows: no watcher).
                if (!localNames(p).includes("renamed_by_menu") || localNames(p).includes("made_by_menu"))
                    return false;
            }
        },
        {
            "name": "local delete",
            "fn": () => {
                const p = panel.local;
                if (!held.localDelete) {
                    p.deleteEntries([p.path + "/renamed_by_menu"]);
                    expect(panel.askBox.visible && panel.askBox.text.indexOf("renamed_by_menu") > 0, "no confirmation: " + panel.askBox.text);
                    panel.askBox.onAnswer("yes");
                    held.localDelete = true;
                    return false;
                }
                if (localHelper.isDir(p.path + "/renamed_by_menu") || localNames(p).includes("renamed_by_menu"))
                    return false;
                panel.askBox.close();
            }
        },
        // The wheel scrolls the lists: a bare ListView outside a ScrollView used to have
        // `interactive: false`, so its bar showed but the wheel was ignored. Upload many/ (the
        // test made 60 files) to have something to scroll in both panes.
        {
            "name": "wheel upload",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                panel.uploadPaths([local + "/many"], -1);
            }
        },
        {
            "name": "wheel uploaded",
            "fn": () => {
                if (!settled() || rowOf("many") < 0)
                    return false;
                tr.clearFinished();
                files.navigate(remote + "/many");
            }
        },
        {
            "name": "remote wheel",
            "fn": () => {
                if (!inDir(remote + "/many"))
                    return false;
                const l = panel.list.list;
                expect(l.contentHeight > l.height, "many/ has nothing to scroll: " + l.contentHeight + " <= " + l.height);
                if (!held.remoteWheelSent) {
                    // In the narrow side panel with two panes the list is clipped; aim at its
                    // visible half, not at the ListView's own (wider) centre.
                    const p = panel.list.mapToItem(null, panel.list.width / 2, panel.list.height / 2);
                    Probe.wheel(shell, p.x, p.y, 0, -120, Qt.NoModifier);
                    held.remoteWheelSent = true;
                    return false;
                }
                expect(l.contentY > 0, "the wheel did not scroll the remote list: contentY " + l.contentY);
                files.navigate(remote);
            }
        },
        {
            "name": "local wheel setup",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                panel.local.setDir(local + "/many");
            }
        },
        {
            "name": "local wheel",
            "fn": () => {
                if (panel.local.list.count < 60)
                    return false;
                const l = panel.local.list;
                expect(l.contentHeight > l.height, "local many/ has nothing to scroll: " + l.contentHeight + " <= " + l.height);
                if (!held.localWheelSent) {
                    const p = panel.local.list.mapToItem(null, Math.min(panel.local.list.width, panel.local.width) / 2, panel.local.list.height / 2);
                    Probe.wheel(shell, p.x, p.y, 0, -120, Qt.NoModifier);
                    held.localWheelSent = true;
                    return false;
                }
                expect(l.contentY > 0, "the wheel did not scroll the local pane: contentY " + l.contentY);
                panel.local.setDir(local);
            }
        },
        {
            "name": "local listed",
            "fn": () => {
                if (panel.local.list.count < 3)
                    return false;
                expect(panel.local.path === local, "local path " + panel.local.path);
                panel.local.selection = [local + "/up2.txt"];
                panel.local.uploadSelected(); // the → button
            }
        },
        {
            "name": "local uploaded",
            "fn": () => {
                if (!settled())
                    return false;
                expect(job("up2.txt").state === "done", "arrow upload " + tr.jobsJson());
                tr.clearFinished();
                // Download with the toolbar button: straight into the local folder, no dialog.
                panel.list.selection = [rowOf("a.txt")];
                panel.downloadSelected();
            }
        },
        {
            "name": "into local pane",
            "fn": () => {
                if (panel.askBox.visible) {
                    // local/a.txt exists already: Yes = overwrite
                    panel.askBox.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
                    panel.askBox.close();
                    return false;
                }
                if (!settled())
                    return false;
                expect(job("a.txt").state === "done" && job("a.txt").local === local, "into the pane " + tr.jobsJson());
                tr.clearFinished();
                // Remote rows dropped on the pane download into it.
                held.payload = files.dragPayload(rows("big.bin"));
                panel.local.dropRemote(held.payload, -1000);
            }
        },
        {
            "name": "dropped on local",
            "fn": () => {
                if (panel.askBox.visible) {
                    panel.askBox.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
                    panel.askBox.close();
                    return false;
                }
                if (!settled())
                    return false;
                expect(job("big.bin").state === "done", "drop download " + tr.jobsJson());
                tr.clearFinished();
                panel.setTwoPane(false);
                expect(!panel.twoPane && panel.local === null && !files.twoPane("files_two_pane_side"), "two panes off");
            }
        },
        // Synchronize: bound to the host it was opened for, a plan, mkdir, copies with the time kept.
        {
            "name": "sync open",
            "fn": () => {
                files.navigate(remote);
            }
        },
        // The dialog closed while its comparison runs; the late result has nobody to go to,
        // and the next dialog compares as usual.
        {
            "name": "sync closed while comparing",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                const early = panel.synchronize(stem + "syncl");
                early.remoteField.text = stem + "syncr";
                early.direction.currentIndex = 2;
                early.compare();
                expect(early.plan.count === 0, "the plan came at once");
                early.close();
                held.closedAt = Date.now();
            }
        },
        {
            "name": "sync after the closed one",
            "fn": () => Date.now() > held.closedAt + 2000
        },
        {
            "name": "sync dialog",
            "fn": () => {
                if (!inDir(remote))
                    return false;
                held.sync = panel.synchronize(stem + "syncl");
                expect(held.sync !== null && held.sync.info.label === "t1", "sync dialog " + held.sync);
                held.sync.remoteField.text = stem + "syncr";
                held.sync.direction.currentIndex = 2;
                held.sync.compare();
            }
        },
        {
            "name": "sync plan",
            "fn": () => {
                if (held.sync.plan.count === 0)
                    return false;
                const rowsText = [];
                for (let i = 0; i < held.sync.plan.count; ++i) {
                    const r = held.sync.plan.get(i);
                    rowsText.push(r.opLabel + " " + r.rel);
                }
                const t = rowsText.join("|");
                expect(t === "↑ Upload l_only.txt|+ Folder on server sub|↑ Upload sub/deep.txt|↓ Download r_only.txt" || t === "↑ Upload l_only.txt|↑ Upload sub/deep.txt|↓ Download r_only.txt" || held.sync.plan.count >= 3, "plan " + t);
                expect(held.sync.message.text.indexOf("file(s) to copy") > 0 && held.sync.runnable, "sync message " + held.sync.message.text);
                held.sync.synchronize();
            }
        },
        {
            "name": "sync queued",
            "fn": () => {
                if (!settled() || jobs().length < 3)
                    return false;
                expect(jobs().every(j => j.state === "done"), "sync jobs " + tr.jobsJson());
                expect(held.sync.message.text.indexOf("copy job(s) queued") > 0 && held.sync.plan.count === 0, "queued message " + held.sync.message.text);
                held.sync.close();
                tr.clearFinished();
            }
        },
        // Session > Open files (SFTP): on for an SSH terminal tab, opens a Files tab for its entry.
        {
            "name": "session.files action",
            "fn": () => {
                const act = shell.shellActions.actions["session.files"];
                const before = area.activeTab;
                expect(before.embedded && act.enabled, "session.files disabled on an SSH tab");
                act.trigger();
                expect(area.activeTab !== before && area.activeTab.embedded === false && area.activeTab.text === "Files · box", "tab " + area.activeTab.text);
                expect(!act.enabled, "session.files enabled on a Files tab");
                area.closeTab(area.activeTab, false);
            }
        },
        // A Files tab of its own: its own listing and queue; the side panel rests.
        {
            "name": "files tab",
            "fn": () => {
                shell.openFilesTab(entryJson);
                held.ft = area.activeTab;
            }
        },
        {
            "name": "files tab listed",
            "fn": () => {
                const t = held.ft;
                expect(t.embedded === false && t.text === "Files · box" && t.live === false, "tab " + t.text);
                if (!t.session || !t.session.files.hasTarget || t.session.files.status.indexOf(" shown (") < 0)
                    return false;
                expect(t.toolTip === "box — files (SFTP)" && t.closeQuestion() === "", "tab tip / question");
                expect(t.session.transfers !== tr && t.session.twoPaneKey === "files_two_pane_tabs", "own queue");
                expect(!files.hasTarget && shell.sideTabBar.currentIndex === 0, "the side panel did not rest");
                expect(find("filesPanel", shell.contentItem) !== null, "view");
                area.closeTab(t, false);
            }
        },
        // The host menu: the SFTP entry and the one-shot scp now have hooks.
        {
            "name": "host menu",
            "fn": () => {
                const hosts = find("hostsPanel", shell.contentItem);
                let row = -1;
                for (let r = 0; r < hosts.list.count; ++r) {
                    const i = JSON.parse(hosts.hosts.rowJson(r));
                    if (i.kind === "connection" && i.label === "box")
                        row = r;
                }
                expect(row >= 0, "no box row");
                const m = hosts.buildContextMenu(row);
                const texts = [];
                for (let i = 0; i < m.count; ++i) {
                    const it = m.itemAt(i);
                    texts.push(it.text === undefined ? "-" : (it.enabled ? "" : "!") + it.text);
                }
                const t = texts.join("|");
                expect(t.indexOf("Open files (SFTP)|Upload file to host…|Download file from host…") > 0, "host menu " + t);
                m.destroy();
                shell.filesHost.scpTransfer(entryJson, false);
            }
        },
        {
            "name": "scp prompt",
            "fn": () => {
                const p = shell.filesHost.prompt;
                if (!p.visible)
                    return false;
                expect(p.label === "Remote file on box (e.g. /var/log/syslog or ~/file.txt):" && p.title === "Download file", "prompt " + p.label);
                p.close();
                // A bad remote path is the builder's message, in a box.
                shell.filesHost.runScp(entryJson, local + "/up1.txt", "-x", true);
            }
        },
        {
            "name": "scp refused",
            "fn": () => {
                const b = shell.filesHost.messageBox;
                if (!b.visible)
                    return false;
                expect(b.text === "Remote path must not start with '-'.", "scp error " + b.text);
                b.close();
                // The real thing: a terminal tab runs the scp line.
                shell.filesHost.runScp(entryJson, local + "/up1.txt", remote + "/scp_up.txt", true);
                held.scp = area.activeTab;
            }
        },
        {
            "name": "scp ran",
            "fn": () => {
                const t = held.scp;
                expect(t.title === "scp → box" && t.conn === null, "scp tab " + t.title);
                if (!t.session || !t.session.finished)
                    return false;
            }
        },
        // A host whose password was typed in the terminal (not saved): Files is refused, offers to
        // ask, lists with the password typed, and keeps it for the session.
        {
            "name": "locked refused",
            "fn": () => {
                held.locked = JSON.stringify({
                    "label": "locked",
                    "host": "locked",
                    "user": "u"
                });
                files.setTarget(held.locked, "locked");
                held.lockedStep = 1;
            }
        },
        {
            "name": "locked banner",
            "fn": () => {
                if (!files.authRequired)
                    return false;
                expect(files.status.indexOf("Authentication required") === 0, "status " + files.status);
                expect(find("filesPasswordButton", panel).visible, "no password button");
            }
        },
        {
            "name": "locked asked",
            "fn": () => {
                panel.askPassword();
                expect(panel.prompt.visible && panel.prompt.secret && panel.prompt.title === "Password", "no password prompt");
                panel.prompt.field.text = "open-sesame";
                panel.prompt.accept();
            }
        },
        {
            "name": "locked listed",
            "fn": () => {
                if (files.authRequired || files.busy || files.cwd === "")
                    return false;
                expect(!find("filesPasswordButton", panel).visible, "button still shown");
                expect(panel.prompt.field.text === "", "the typed password stayed in the field");
                files.setTarget(entryJson, "box");
            }
        },
        {
            "name": "back on box",
            "fn": () => {
                if (files.busy || files.cwd === "")
                    return false;
                files.setTarget(held.locked, "locked");
            }
        },
        {
            "name": "locked remembered",
            "fn": () => {
                if (files.busy || files.cwd === "")
                    return false;
                expect(!files.authRequired, "asked for the password again");
            }
        },
        {
            "name": "settings",
            "fn": () => {
                shell.controller.persist();
                console.info("FILES_DONE");
                shell.requestQuit();
            }
        }
    ]

    FilesHelper {
        id: localHelper
    }

    Timer {
        interval: 100
        repeat: true
        running: !check.failed && check.step < check.steps.length
        onTriggered: {
            const s = check.steps[check.step];
            let r;
            try {
                r = s.fn();
            } catch (e) {
                check.fail("exception: " + e);
                return;
            }
            if (check.failed)
                return;
            if (r === false) {
                check.waited += 100;
                if (check.waited > 10000)
                    check.fail("timed out waiting; status=" + (check.files ? check.files.status : "") + " jobs=" + (check.tr ? check.tr.jobsJson() : "") + " sync=" + (check.held.sync ? check.held.sync.message.text : ""));
                return;
            }
            check.waited = 0;
            ++check.step;
        }
    }
}
