import QtQuick
import QtQuick.Dialogs
import Termigator.Terminal.Probe

// Offscreen check of the Files local pane from the keyboard (tests/files_keys_smoke.rs): next to
// a local shell, in the folder `keys/` beside the smoke-test grab (sub/, a.txt, b.txt). Arrows,
// Home / End, Enter into a folder, Backspace up, F2, Ctrl+N, Delete, Shift+F10 with the keys in
// the menu, and no reaction while the path field has the focus. Quits with 0 (1 after a failed
// expectation). A step returning false is retried.
Item {
    id: check

    required property var shell
    property int step: 0
    property int waited: 0
    property var panel: null
    property var local: null
    property string dir: ""
    property string before: ""

    function fail(reason) {
        console.error("termigator files keys smoke: FAILED at step " + step + ": " + reason);
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

    function names() {
        const out = [];
        for (let r = 0; r < local.model.count; ++r)
            out.push(local.model.get(r, "fileName"));
        return out.join(",");
    }

    function rowOf(name) {
        for (let r = 0; r < local.model.count; ++r)
            if (local.model.get(r, "fileName") === name)
                return r;
        return -1;
    }

    function currentName() {
        return local.currentRow >= 0 ? local.model.get(local.currentRow, "fileName") : "";
    }

    // The list has the keyboard on `name` (the window active again after a box).
    function focusOn(name) {
        local.list.forceActiveFocus();
        local.currentRow = rowOf(name);
        local.selection = [local.model.get(local.currentRow, "filePath")];
        return local.list.activeFocus;
    }

    function snapshot() {
        return names() + "|" + local.currentDir() + "|" + panel.prompt.visible + "|" + panel.askBox.visible;
    }

    readonly property var steps: [
        () => {
            if (!shell.newLocalShell)
                return false;
            const shot = String(shell.controller.smokeTestShot);
            dir = shot.slice(0, Math.max(shot.lastIndexOf("/"), shot.lastIndexOf("\\"))) + "/keys";
            shell.newLocalShell("");
        },
        () => {
            const t = shell.panesArea ? shell.panesArea.activeTab : null;
            if (!t || !t.session)
                return false;
            shell.sideTabBar.currentIndex = 1;
        },
        () => {
            panel = find("filesPanel", shell.contentItem);
            local = panel ? panel.local : null;
            if (!local)
                return false;
            local.goTo(dir);
        },
        () => {
            if (names() !== "sub,a.txt,b.txt")
                return false;
            shell.requestActivate();
            if (!focusOn("sub"))
                return false;
            // Home / End / Down / Up, Shift extends the selection.
            press(Qt.Key_End);
            expect(currentName() === "b.txt" && local.selection.length === 1, "End " + currentName());
            press(Qt.Key_Up, Qt.ShiftModifier);
            expect(currentName() === "a.txt" && local.selection.length === 2, "Shift+Up " + local.selection);
            press(Qt.Key_Home);
            expect(currentName() === "sub" && local.selection.length === 1, "Home " + currentName());
            press(Qt.Key_Escape);
            expect(local.selection.length === 0, "Esc " + local.selection);
            // Enter opens the folder.
            press(Qt.Key_Return);
        },
        () => {
            if (!local.currentDir().endsWith("/keys/sub") || local.model.count !== 1)
                return false;
            expect(local.list.activeFocus, "the list lost the focus in the folder");
            // Backspace goes up.
            press(Qt.Key_Backspace);
        },
        () => {
            if (local.currentDir() !== dir || rowOf("a.txt") < 0)
                return false;
            // F2: the rename prompt with the name.
            focusOn("a.txt");
            press(Qt.Key_F2);
            expect(panel.prompt.visible && panel.prompt.field.text === "a.txt", "F2 prompt " + panel.prompt.field.text);
            panel.prompt.field.text = "c.txt";
            panel.prompt.accept();
        },
        () => {
            if (rowOf("c.txt") < 0 || panel.prompt.visible)
                return false;
            expect(rowOf("a.txt") < 0, "renamed " + names());
            // Ctrl+N: a new folder.
            focusOn("c.txt");
            press(Qt.Key_N, Qt.ControlModifier);
            expect(panel.prompt.visible && panel.prompt.title === "New local folder", "Ctrl+N prompt " + panel.prompt.title);
            panel.prompt.field.text = "newdir";
            panel.prompt.accept();
        },
        () => {
            if (rowOf("newdir") < 0 || panel.prompt.visible)
                return false;
            // Delete: the question names the file; Yes deletes it.
            focusOn("b.txt");
            press(Qt.Key_Delete);
            expect(panel.askBox.visible && panel.askBox.text.indexOf("\"b.txt\"") >= 0, "Delete question " + panel.askBox.text);
            panel.askBox.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
            panel.askBox.close();
        },
        () => {
            shell.requestActivate();
            if (rowOf("b.txt") >= 0 || !shell.active || !focusOn("c.txt"))
                return false;
            // Shift+F10: the row's menu, with the keys beside the entries.
            press(Qt.Key_F10, Qt.ShiftModifier);
            const m = local.contextMenu;
            expect(m !== null && m.visible, "Shift+F10 menu");
            const keys = {};
            for (let i = 0; i < m.count; ++i) {
                const it = m.itemAt(i);
                if (it.shortcutText)
                    keys[it.text] = it.shortcutText;
            }
            expect(JSON.stringify(keys) === JSON.stringify({
                "Open with the default application": "Enter",
                "Rename…": "F2",
                "Delete…": "Del",
                "New folder…": "Ctrl+N",
                "Refresh": "F5"
            }), "menu keys " + JSON.stringify(keys));
            m.close();
            // Typing in the path field: the list's keys do nothing.
            local.pathField.forceActiveFocus();
            before = snapshot();
            for (const k of [[Qt.Key_Delete, 0], [Qt.Key_F2, 0], [Qt.Key_N, Qt.ControlModifier], [Qt.Key_Backspace, 0], [Qt.Key_F5, 0]])
                press(k[0], k[1]);
            expect(snapshot() === before, "keys acted in the path field:\n" + before + "\n" + snapshot());
            console.info("termigator files keys smoke: done");
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
