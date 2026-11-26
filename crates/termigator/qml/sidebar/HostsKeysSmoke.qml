import QtQuick
import QtQuick.Dialogs
import Termigator.Terminal.Probe

// Offscreen check of the Hosts tree from the keyboard (tests/sidebar_smoke.rs): navigation,
// Enter / Space, F2 in place, Delete, Ctrl+C / Ctrl+X / Ctrl+V, Ctrl+N, Ctrl+Shift+N,
// Shift+F10, Esc, the focus after each change, and no reaction while a text field or a terminal
// has the focus. Quits with 0 (1 after a failed expectation). A step returning false is retried.
Item {
    id: check

    required property var shell
    property int step: 0
    property int waited: 0
    property var panel: null
    property var area: null
    property var tab: null
    property var opened: []
    property var editor: null

    function fail(reason) {
        console.error("termigator hosts keys smoke: FAILED at step " + step + ": " + reason);
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

    function rows() {
        const out = [];
        for (let r = 0; r < panel.list.count; ++r)
            out.push(JSON.parse(panel.hosts.rowJson(r)).label);
        return out.join(",");
    }

    function rowOf(label) {
        for (let r = 0; r < panel.list.count; ++r)
            if (JSON.parse(panel.hosts.rowJson(r)).label === label)
                return r;
        return -1;
    }

    // The entry `label` in folder `group` (the same name can be in several folders).
    function rowIn(label, group) {
        for (let r = 0; r < panel.list.count; ++r) {
            const i = panel.info(r);
            if (i.kind !== "group" && i.label === label && i.group === group)
                return r;
        }
        return -1;
    }

    function current() {
        const i = panel.info(panel.currentRow);
        return i ? i.label : "";
    }

    function conns() {
        const out = [];
        for (let i = 0; ; ++i) {
            const json = panel.hosts.entryJson("connection", i);
            if (json === "")
                return out;
            out.push(JSON.parse(json));
        }
    }

    // label@group of every connection, in config order.
    function entries() {
        return conns().map(c => c.label + "@" + (c.group || "")).join(",");
    }

    function field() {
        const item = panel.editingRow >= 0 ? panel.list.itemAtIndex(panel.editingRow) : null;
        return item ? item.renameField : null;
    }

    // The list has the keyboard focus on `row` (a label: its first row), the window active
    // again after a box.
    function focusOn(row) {
        shell.requestActivate();
        panel.list.forceActiveFocus();
        panel.setCurrent(typeof row === "number" ? row : rowOf(row), true);
    }

    function answer(button) {
        panel.messageBox.buttonClicked(button, button === MessageDialog.Yes ? MessageDialog.YesRole : MessageDialog.NoRole);
        panel.messageBox.close();
    }

    // What the shortcut checks compare: nothing in the tree may change.
    function snapshot() {
        return entries() + "|" + panel.hosts.groupNames().join(",") + "|" + rows() + "|" + panel.editingRow + "|" + panel.hosts.hasClip() + "|" + panel.prompt.visible + "|" + panel.messageBox.visible;
    }

    property string before: ""

    // A step that waits until the window has the keyboard again (offscreen, a closed box
    // stays the focus window until the shell asks back).
    readonly property var back: () => {
        shell.requestActivate();
        if (!shell.active)
            return false;
        panel.list.forceActiveFocus();
        return panel.list.activeFocus ? undefined : false;
    }

    readonly property var steps: [
        () => {
            panel = find("hostsPanel", shell.contentItem);
            if (!expect(panel !== null, "no hosts panel"))
                return;
            shell.openConnection = json => check.opened.push(JSON.parse(json).label);
            shell.openEditor = (kind, json, group, done) => {
                check.editor = {
                    "kind": kind,
                    "group": group
                };
            };
            expect(rows() === "lab,prod,desk,solo", "rows " + rows());
            shell.requestActivate();
            panel.list.forceActiveFocus();
            // Home / End / Up / Down move the current row.
            press(Qt.Key_End);
            expect(current() === "solo", "End " + current());
            press(Qt.Key_Up);
            expect(current() === "desk", "Up " + current());
            press(Qt.Key_Home);
            expect(current() === "lab", "Home " + current());
            press(Qt.Key_Down);
            expect(current() === "prod" && panel.selection.join() === "1", "Down " + current() + " " + panel.selection);
            // Right expands, then goes to the first child; Left collapses, then goes up.
            press(Qt.Key_Right);
            expect(rows() === "lab,prod,db,web,desk,solo" && current() === "prod", "Right expands " + rows());
            press(Qt.Key_Right);
            expect(current() === "db" && panel.info(panel.currentRow).kind === "group", "Right to the child " + current());
            press(Qt.Key_Right);
            expect(rows() === "lab,prod,db,db,web,desk,solo", "Right on db " + rows());
            press(Qt.Key_Down);
            expect(panel.info(panel.currentRow).kind === "connection" && current() === "db", "down into db");
            press(Qt.Key_Left);
            expect(current() === "db" && panel.info(panel.currentRow).kind === "group", "Left to the parent folder");
            press(Qt.Key_Left);
            expect(rows() === "lab,prod,db,web,desk,solo" && current() === "db", "Left collapses " + rows());
            press(Qt.Key_Left);
            expect(current() === "prod", "Left to prod " + current());
            press(Qt.Key_Left);
            expect(rows() === "lab,prod,desk,solo", "Left collapses prod " + rows());
            // Space and Enter toggle a folder.
            press(Qt.Key_Space, Qt.NoModifier, " ");
            expect(rows() === "lab,prod,db,web,desk,solo" && current() === "prod", "Space expands " + rows());
            press(Qt.Key_Space, Qt.NoModifier, " ");
            expect(rows() === "lab,prod,desk,solo", "Space collapses " + rows());
            press(Qt.Key_Return);
            expect(rows() === "lab,prod,db,web,desk,solo", "Enter expands " + rows());
            press(Qt.Key_Return);
            expect(rows() === "lab,prod,desk,solo" && current() === "prod", "Enter collapses " + rows());
            // Enter on a host opens it; Space there does nothing.
            press(Qt.Key_End);
            press(Qt.Key_Space, Qt.NoModifier, " ");
            press(Qt.Key_Return);
            expect(opened.join() === "solo", "Enter on a host " + opened);
            // Esc drops the selection, the current row stays.
            press(Qt.Key_Escape);
            expect(panel.selection.length === 0 && current() === "solo", "Esc " + panel.selection);
            // F2: the name in place, all of it selected.
            press(Qt.Key_F2);
        },
        () => {
            const f = field();
            if (f === null || !f.activeFocus)
                return false;
            expect(panel.editingRow === rowOf("solo") && f.selectedText === "solo", "F2 field " + (f ? f.selectedText : "none"));
            // Keys in the field stay there: Delete edits the text, it does not delete the host.
            press(Qt.Key_Delete);
            expect(f.text === "" && entries().indexOf("solo@") >= 0, "Delete in the field " + f.text);
            // An empty name is refused; the field stays with the reason.
            press(Qt.Key_Return);
            expect(panel.editingRow >= 0 && panel.list.itemAtIndex(panel.editingRow).renameError === "A name cannot be empty.", "empty name");
            // Esc cancels: the name is kept, the list has the focus back.
            press(Qt.Key_Escape);
            expect(panel.editingRow === -1 && rowOf("solo") >= 0 && panel.list.activeFocus && current() === "solo", "Esc cancels " + rows());
            press(Qt.Key_F2);
        },
        () => {
            const f = field();
            if (f === null || !f.activeFocus)
                return false;
            f.text = "alone";
            press(Qt.Key_Return);
            expect(entries() === "web@prod,db@prod/db,desk@,alone@", "renamed " + entries());
            expect(current() === "alone" && panel.list.activeFocus, "focus after rename " + current());
            // A gateway renamed: the RDP entry that goes through it follows.
            panel.setCurrent(panel.hosts.reveal("connection", 0), true);
            press(Qt.Key_F2);
        },
        () => {
            const f = field();
            if (f === null || !f.activeFocus)
                return false;
            f.text = "bastion";
            press(Qt.Key_Return);
            expect(conns()[0].label === "bastion" && conns()[2].ssh_gateway === "bastion", "gateway kept " + JSON.stringify(conns()[2]));
            // F2 on a folder; a name its siblings have is refused.
            focusOn("lab");
            press(Qt.Key_F2);
        },
        () => {
            const f = field();
            if (f === null || !f.activeFocus)
                return false;
            expect(f.selectedText === "lab", "folder field " + f.selectedText);
            f.text = "prod";
            press(Qt.Key_Return);
            const err = panel.list.itemAtIndex(panel.editingRow).renameError;
            expect(err === "A folder “prod” already exists in the top level.", "folder conflict " + err);
            f.text = "lab2";
            press(Qt.Key_Return);
            expect(panel.hosts.groupNames().indexOf("lab2") >= 0 && panel.hosts.groupNames().indexOf("lab") < 0, "folder renamed " + panel.hosts.groupNames());
            expect(current() === "lab2" && panel.list.activeFocus, "focus after the folder rename " + current());
            // Ctrl+C on a host, Ctrl+V on a folder: a copy goes in, the name kept there.
            focusOn("alone");
            press(Qt.Key_C, Qt.ControlModifier);
            expect(shell.statusMessage.startsWith("Copied “alone”"), "copy notice " + shell.statusMessage);
            panel.setCurrent(rowOf("lab2"), true);
            press(Qt.Key_V, Qt.ControlModifier);
            expect(entries() === "bastion@prod,db@prod/db,desk@,alone@,alone@lab2", "pasted " + entries());
            expect(current() === "alone" && panel.info(panel.currentRow).group === "lab2" && panel.list.activeFocus, "pasted copy selected " + current());
            // Pasted again next to the original: a free name.
            focusOn(rowIn("alone", ""));
            press(Qt.Key_V, Qt.ControlModifier);
            expect(entries() === "bastion@prod,db@prod/db,desk@,alone@,alone@lab2,alone (copy)@", "copy beside " + entries());
            // A folder: its whole subtree; never into itself.
            focusOn(panel.hosts.groupRow("prod"));
            press(Qt.Key_C, Qt.ControlModifier);
            press(Qt.Key_V, Qt.ControlModifier);
        },
        () => {
            expect(panel.messageBox.visible && panel.messageBox.text === "Cannot paste the folder “prod” into itself or one of its subfolders.", "cycle " + panel.messageBox.text);
            answer(MessageDialog.Ok);
        },
        back,
        () => {
            focusOn("lab2");
            press(Qt.Key_V, Qt.ControlModifier);
            expect(panel.hosts.groupNames().indexOf("lab2/prod/db") >= 0 && entries().indexOf("bastion@lab2/prod,db@lab2/prod/db") >= 0, "folder copied " + entries());
            expect(current() === "prod" && panel.info(panel.currentRow).path === "lab2/prod", "copied folder selected " + JSON.stringify(panel.info(panel.currentRow)));
            // Ctrl+X marks the row; Esc drops the cut.
            focusOn("alone (copy)");
            press(Qt.Key_X, Qt.ControlModifier);
            const row = rowOf("alone (copy)");
            expect(panel.hosts.isCut(row) && panel.list.itemAtIndex(row).cut, "cut mark");
            press(Qt.Key_Escape);
            expect(!panel.hosts.isCut(row) && !panel.list.itemAtIndex(row).cut && !panel.hosts.hasClip(), "cut cancelled");
            // Cut and paste: moved, same place in the list, every setting kept; pasted once.
            const index = conns().findIndex(c => c.label === "alone (copy)");
            press(Qt.Key_X, Qt.ControlModifier);
            panel.setCurrent(rowOf("lab2"), true);
            press(Qt.Key_V, Qt.ControlModifier);
            const moved = conns()[index];
            expect(moved.label === "alone (copy)" && moved.group === "lab2" && moved.host === "10.0.0.8", "moved " + JSON.stringify(moved));
            expect(!panel.hosts.hasClip() && current() === "alone (copy)", "cut pasted once " + current());
            // A folder cut into its own subfolder is refused.
            focusOn(panel.hosts.groupRow("prod"));
            press(Qt.Key_X, Qt.ControlModifier);
            press(Qt.Key_Right);
            press(Qt.Key_Right);
            expect(panel.info(panel.currentRow).path === "prod/db", "on prod/db " + JSON.stringify(panel.info(panel.currentRow)));
            press(Qt.Key_V, Qt.ControlModifier);
        },
        () => {
            expect(panel.messageBox.visible && panel.messageBox.text.startsWith("Cannot paste the folder “prod” into itself"), "cut cycle " + panel.messageBox.text);
            answer(MessageDialog.Ok);
            expect(panel.hosts.groupNames().indexOf("prod/db") >= 0 && panel.hosts.hasClip(), "nothing moved " + panel.hosts.groupNames());
        },
        back,
        () => {
            focusOn("lab2");
            press(Qt.Key_Escape);
            // Ctrl+N: a new folder in the selected one, then selected.
            press(Qt.Key_N, Qt.ControlModifier);
        },
        () => {
            expect(panel.prompt.visible && panel.prompt.title === "New folder" && panel.prompt.label.indexOf("inside “lab2”") >= 0, "new folder prompt " + panel.prompt.label);
            panel.prompt.field.text = "sub";
            panel.prompt.accept();
            expect(panel.hosts.groupNames().indexOf("lab2/sub") >= 0 && panel.info(panel.currentRow).path === "lab2/sub", "new folder " + panel.hosts.groupNames());
            shell.requestActivate();
        },
        () => {
            // A modal popup that is still closing keeps window shortcuts off.
            if (!panel.list.activeFocus || panel.prompt.visible)
                return false;
            // Ctrl+Shift+N (File → New connection → SSH): the editor for the selected folder.
            press(Qt.Key_N, Qt.ControlModifier | Qt.ShiftModifier);
            expect(editor !== null && editor.kind === "ssh" && editor.group === "lab2/sub", "Ctrl+Shift+N " + JSON.stringify(editor));
            // Shift+F10: the menu of the current row, with the keys beside the entries.
            focusOn("desk");
            press(Qt.Key_F10, Qt.ShiftModifier);
            const m = panel.contextMenu;
            expect(m !== null && m.visible, "Shift+F10 menu");
            const keys = {};
            for (let i = 0; i < m.count; ++i) {
                const it = m.itemAt(i);
                if (it.shortcutText)
                    keys[it.text] = it.shortcutText;
            }
            expect(JSON.stringify(keys) === JSON.stringify({
                "Connect / Run": "Enter",
                "Rename": "F2",
                "Cut": "Ctrl+X",
                "Copy": "Ctrl+C",
                "Paste": "Ctrl+V",
                "Move up": "Alt+Up",
                "Move down": "Alt+Down",
                "Delete…": "Del"
            }), "menu keys " + JSON.stringify(keys));
            m.close();
            // The Menu key opens it too, here on a folder.
            focusOn("lab2");
            press(Qt.Key_Menu);
            expect(panel.contextMenu.visible && panel.contextMenu.itemAt(0).text === "Collapse", "Menu key " + panel.contextMenu.itemAt(0).text);
            panel.contextMenu.close();
        },
        // Delete on a host: its name in the question; the next row is current afterwards.
        () => {
            focusOn("alone (copy)");
            press(Qt.Key_Delete);
        },
        () => {
            expect(panel.messageBox.text === "Remove “alone (copy)” from Hosts?\n\nOnly this entry is removed.", "delete question " + panel.messageBox.text);
            const at = panel.currentRow;
            const count = conns().length;
            answer(MessageDialog.Yes);
            expect(conns().length === count - 1 && entries().indexOf("alone (copy)") < 0, "deleted " + entries());
            expect(panel.currentRow === Math.min(at, panel.list.count - 1) && panel.currentRow >= 0, "current after delete " + panel.currentRow + " " + at);
        },
        back,
        () => {
            // An empty folder: Yes / No; No keeps it.
            focusOn("sub");
            press(Qt.Key_Delete);
        },
        () => {
            expect(panel.messageBox.text === "Delete the empty folder “lab2/sub”?", "empty folder " + panel.messageBox.text);
            answer(MessageDialog.No);
            expect(panel.hosts.groupNames().indexOf("lab2/sub") >= 0, "kept on No");
        },
        back,
        () => {
            focusOn("sub");
            press(Qt.Key_Delete);
        },
        () => {
            answer(MessageDialog.Yes);
            expect(panel.hosts.groupNames().indexOf("lab2/sub") < 0, "empty folder deleted " + panel.hosts.groupNames());
        },
        back,
        () => {
            // A folder with contents: the dialog says what is in it; by default it all moves up.
            focusOn("lab2");
            press(Qt.Key_Delete);
        },
        () => {
            const d = panel.deleteFolderDialog;
            expect(d.visible && d.question === "The “lab2” folder holds 3 entries and 2 subfolders.\nWhat should happen to them?", "delete folder " + d.question);
            expect(d.toParent.checked && !d.deleteAll.checked, "safe default");
            const count = conns().length;
            d.accept();
            expect(conns().length === count && panel.hosts.groupNames().indexOf("lab2") < 0, "contents kept " + entries());
            expect(entries().indexOf("alone@,") >= 0 || entries().endsWith("alone@"), "moved to the top " + entries());
            shell.requestActivate();
        },
        // Typing in a text field: the tree's keys do nothing.
        () => {
            focusOn("desk");
            panel.search.forceActiveFocus();
            before = snapshot();
            for (const k of [[Qt.Key_Delete, 0], [Qt.Key_F2, 0], [Qt.Key_X, Qt.ControlModifier], [Qt.Key_C, Qt.ControlModifier], [Qt.Key_N, Qt.ControlModifier], [Qt.Key_A, Qt.ControlModifier]])
                press(k[0], k[1]);
            expect(snapshot() === before, "keys acted in the search field:\n" + before + "\n" + snapshot());
            expect(panel.search.activeFocus, "the search lost the focus");
            // A terminal: a local tab with `cat`.
            shell.openInner("exec cat", "keys", "embedded", "");
        },
        () => {
            area = find("panesArea", shell.contentItem);
            tab = area ? area.activeTab : null;
            if (!tab || !tab.session)
                return false;
            const view = area.terminalHost.viewFor(tab);
            if (!view || !view.term)
                return false;
            view.term.forceActiveFocus();
            if (!view.term.activeFocus)
                return false;
            before = snapshot();
            for (const k of [[Qt.Key_Delete, 0, ""], [Qt.Key_F2, 0, ""], [Qt.Key_Space, 0, " "], [Qt.Key_X, Qt.ControlModifier, "\x18"], [Qt.Key_V, Qt.ControlModifier, "\x16"], [Qt.Key_N, Qt.ControlModifier, "\x0e"]])
                press(k[0], k[1], k[2]);
            for (const ch of "keysok")
                press(ch.toUpperCase().charCodeAt(0), 0, ch);
            press(Qt.Key_Return, 0, "\r");
        },
        () => {
            if (tab.session.ext.history.indexOf("keysok") < 0)
                return false;
            expect(snapshot() === before, "keys acted on the tree from the terminal:\n" + before + "\n" + snapshot());
            expect(area.terminalHost.viewFor(tab).term.activeFocus, "the terminal lost the focus");
            // Ctrl+D ends cat, so the session closes by itself.
            press(Qt.Key_D, Qt.ControlModifier, "\x04");
        },
        () => {
            if (tab.live)
                return false;
            console.info("termigator hosts keys smoke: done");
            Qt.exit(0);
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
            if (done === false) {
                check.fail("timed out");
                return;
            }
            check.waited = 0;
            ++check.step;
        }
    }
}
