import QtQuick
import QtQuick.Dialogs
import QtQuick.Window

// Offscreen check (tests/tools_smoke.rs, --smoke-check=tools/ToolsSmoke.qml): the Tunnels
// window against a stub `ssh` (autostart, start, stop, failure text, add / edit / delete, the close
// question), the Snippets menu and manager, a snippet typed into a real session, the bulk edit
// dialog, the disconnect notice and the probe windows. Ends through the close path, which stops the
// tunnels. A step returning false is tried again (15 s at most); a console.error ends with 1.
Item {
    id: check

    required property var shell
    property int step: 0
    property int waited: 0
    property var tools: null
    property var area: null
    property var win: null
    property var dlg: null
    property var tab: null
    property var firstNotice: null
    property var got: null
    property var waitFor: null
    property int reopened: 0

    function fail(reason) {
        console.error("termigator tools smoke: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok)
            fail(what);
        return ok;
    }

    function same(actual, wanted, what) {
        expect(JSON.stringify(actual) === JSON.stringify(wanted), what + ": got " + JSON.stringify(actual) + ", wanted " + JSON.stringify(wanted));
    }

    function all(item, pred, out) {
        out = out || [];
        if (!item)
            return out;
        if (pred(item))
            out.push(item);
        for (const c of item.children)
            all(c, pred, out);
        return out;
    }

    function field(window, placeholder) {
        return all(window.contentItem, i => "placeholderText" in i && String(i.placeholderText).startsWith(placeholder))[0];
    }

    function button(window, text) {
        return all(window.contentItem, i => "icon" in i && "clicked" in i && "text" in i && i.text.replace("&", "") === text)[0];
    }

    // Saves a grab of `window` next to the shell's (to look at it); a step returns this to wait for it.
    function shot(window, name) {
        const path = shell.controller.smokeTestShot.replace(/[^/]*$/, name + ".png");
        let state = 0;
        return () => {
            if (state === 0 && window.contentItem.width >= 1) {
                state = 1;
                window.contentItem.grabToImage(r => {
                    if (!r.saveToFile(path))
                        check.fail("cannot save " + path);
                    state = 2;
                });
            }
            return state === 2;
        };
    }

    function row(name) {
        return JSON.parse(tools.backend.tunnelsJson()).find(r => r.name === name);
    }

    function status(name) {
        const r = row(name);
        return r ? r.status : "";
    }

    function has(t, text) {
        return t.session !== null && t.session.ext.history.indexOf(text) >= 0;
    }

    // Menu texts: "text" per item, "-" per separator.
    function menuTexts(menu) {
        const out = [];
        for (let i = 0; i < menu.count; ++i) {
            const it = menu.itemAt(i);
            out.push(it.text === undefined ? "-" : it.text);
        }
        return out.join("|");
    }

    readonly property var steps: [
        // The hooks, and the tunnel with autostart that came up on its own.
        () => {
            tools = shell.tools;
            area = shell.panes;
            if (!expect(tools !== null && area !== null, "no tools host / panes"))
                return;
            expect(shell.openBulkEdit !== null, "no bulk hook");
            expect(area.disconnectNotice !== null && area.testConnection !== null, "pane hooks not set");
            shell.requestActivate();
        },
        () => {
            if (status("auto") !== "running")
                return false;
            expect(status("bad") === "stopped", "bad: " + status("bad"));
            expect(shell.runningTunnels === 1, "running " + shell.runningTunnels);
            shell.controller.setWindowBool("confirm_close", true);
            expect(shell.controller.closeQuestion(0, 0, shell.runningTunnels) === "1 tunnel(s) will be stopped.\nClose the window and end them?", "close question");
            shell.controller.setWindowBool("confirm_close", false);
        },
        // Tools → Tunnels…: one non-modal window.
        () => {
            shell.shellActions.actions["tools.tunnels"].trigger();
            win = tools.tunnelsWindow;
            if (!expect(win !== null && win.visible && win.title === "Tunnels", "no tunnels window"))
                return;
            expect(win.modality === Qt.NonModal, "tunnels window is modal");
            shell.shellActions.actions["tools.tunnels"].trigger();
            expect(tools.tunnelsWindow === win, "a second Tunnels window");
            same(win.rows.map(r => r.name + ":" + r.statusText + ":" + r.summary), ["auto:Running:L 18081:127.0.0.1:9", "bad:Stopped:L 18083:fail:1", "keyed:Stopped:D 127.0.0.1:18086"], "rows");
            expect(win.selected === null, "a row is selected at start");
        },
        // A forward the server refuses: the status is Failed with ssh's last line, shown in full.
        () => {
            win.select("bad");
            expect(win.selected !== null && !win.live, "bad not selected");
            win.startSelected();
        },
        () => {
            if (status("bad") !== "failed")
                return false;
            same(row("bad").detail, "Could not request local forwarding.", "failure text");
            const shown = all(win.contentItem, i => "type" in i && "showCloseButton" in i && i.visible).map(i => i.text);
            same(shown, ["bad: Could not request local forwarding."], "inline error");
            win.select("auto");
            win.stopSelected();
        },
        () => {
            if (status("auto") !== "stopped")
                return false;
            expect(shell.runningTunnels === 0, "running after stop " + shell.runningTunnels);
            win.startSelected();
        },
        () => {
            if (status("auto") !== "running")
                return false;
            // The Autostart check box saves at once.
            const cell = win.list.itemAtIndex(1);
            const box = all(cell, i => "checked" in i && "tristate" in i)[0];
            expect(box !== undefined && !box.checked, "autostart box");
            box.toggle();
            box.toggled(); // toggle() flips the state; the signal is what a click adds
            same(JSON.parse(tools.backend.autostartJson()), ["auto", "bad"], "autostart saved");
            win.select("bad");
            return shot(win, "tunnels");
        },
        // Add: the form is checked before it closes.
        () => {
            win.add();
            dlg = win.lastEdit;
            if (!expect(dlg !== null && dlg.visible && dlg.title === "Add tunnel", "no add dialog"))
                return;
            dlg.accept();
            expect(dlg.visible && dlg.messageBox.visible && dlg.messageBox.text === "Give the tunnel a name not used by another tunnel.", "empty name: " + dlg.messageBox.text);
            dlg.messageBox.close();
            field(dlg, "e.g.").text = "auto";
            dlg.accept();
            expect(dlg.visible && dlg.messageBox.text === "Give the tunnel a name not used by another tunnel.", "taken name");
            dlg.messageBox.close();
            field(dlg, "e.g.").text = "web";
            dlg.accept();
            expect(dlg.visible && dlg.messageBox.text === "A tunnel needs at least one forward.", "no forwards: " + dlg.messageBox.text);
            dlg.messageBox.close();
            field(dlg, "One per line").text = "Q nonsense";
            dlg.accept();
            expect(dlg.visible && dlg.messageBox.text !== "", "bad forward accepted");
            dlg.messageBox.close();
            field(dlg, "One per line").text = "L 18084:intranet:80";
            return shot(dlg, "tunnel-edit");
        },
        () => {
            dlg.accept();
        },
        () => {
            const web = row("web");
            if (!expect(web !== undefined, "web not saved"))
                return;
            same([web.via, web.summary, web.status, web.autostart], ["bastion", "L 18084:intranet:80", "stopped", false], "new tunnel");
            win.select("web");
            win.startSelected();
        },
        () => {
            if (status("web") !== "running")
                return false;
            // Edit a running tunnel: it restarts with the new definition.
            win.edit();
            dlg = win.lastEdit;
            expect(dlg.title === "Edit tunnel" && field(dlg, "e.g.").text === "web", "edit dialog");
            field(dlg, "One per line").text = "L 18085:intranet:80";
            dlg.accept();
        },
        () => {
            const web = row("web");
            if (web.status !== "running" || web.summary !== "L 18085:intranet:80")
                return false;
            // Delete asks, and stops what runs.
            win.select("web");
            win.askDelete();
            expect(win.deleteBox.visible && win.deleteBox.text === "Delete the tunnel “web”? It is running and will be stopped.", "delete question: " + win.deleteBox.text);
            win.deleteBox.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
            win.deleteBox.close();
        },
        () => {
            expect(row("web") === undefined, "web not deleted");
            same(win.rows.map(r => r.name), ["auto", "bad", "keyed"], "rows after delete");
            // A tunnel with its own key: shown in the editor, with the passphrase hint, and kept.
            win.select("keyed");
            win.edit();
            dlg = win.lastEdit;
            const key = row("keyed").identityFile;
            expect(key !== "" && dlg.identityField.text === key, "tunnel key in the editor: " + dlg.identityField.text);
            expect(dlg.keyLocked, "no passphrase hint for " + key);
            same(dlg.backend.keyFileProblem(key), "", "key problem");
            expect(button(dlg, "Browse…") !== undefined && button(dlg, "From SSH keys…") !== undefined, "key buttons");
            const keyField = field(dlg, "optional — empty: the key of the SSH entry");
            expect(keyField !== undefined && keyField.text === key, "key field: " + (keyField ? keyField.text : "none"));
            dlg.accept();
        },
        () => {
            same(row("keyed").identityFile.length > 0, true, "key kept after edit");
            win.select("keyed");
            win.startSelected();
        },
        () => {
            if (status("keyed") !== "running")
                return false;
            win.stopSelected();
        },
        () => {
            if (status("keyed") !== "stopped")
                return false;
            win.close();
            expect(!win.visible, "tunnels window stays open");
        },
        // Edit → Snippets: the entries from the config, then Manage snippets….
        () => {
            const menu = shell.shellActions.menus["Edit/Snippets"];
            same(menuTexts(menu), "uptime|logs|-|Manage snippets…", "snippets menu");
            tools.runSnippet({
                "name": "uptime",
                "command": "uptime",
                "enter": true
            });
            expect(area.messageBox.visible && area.messageBox.text === "Open an embedded terminal to send a snippet to.", "no terminal: " + area.messageBox.text);
            area.messageBox.close();
            tab = area.addTab({
                "title": "web box",
                "inner": "printf 'READY_%s\\n' ONE; exec cat",
                "conn": {
                    "label": "web",
                    "host": "10.0.0.5",
                    "user": "admin"
                }
            });
        },
        () => {
            if (!has(tab, "READY_ONE"))
                return false;
            // {?Question} is asked, {host} is filled from the session; no Enter, so it stays on the line.
            tools.runSnippet({
                "name": "logs",
                "command": "journalctl -u {?Service} -f on {host}",
                "enter": false
            });
            expect(tools.prompt.visible && tools.prompt.title === "logs" && tools.prompt.label === "Service:", "question prompt");
            tools.prompt.field.text = "nginx";
            tools.prompt.accept();
        },
        () => {
            if (!has(tab, "journalctl -u nginx -f on 10.0.0.5"))
                return false;
            tools.runSnippet({
                "name": "uptime",
                "command": "uptime",
                "enter": true
            });
        },
        () => {
            if (!has(tab, "uptime"))
                return false;
            // A snippet of several lines is shown before it is typed; No sends nothing.
            tools.runSnippet({
                "name": "two",
                "command": "echo MULTI_A\necho MULTI_B",
                "enter": true
            });
            expect(area.questionBox.visible && area.questionBox.text.startsWith("Send the snippet “two” to 1 session(s)?") && area.questionBox.text.indexOf("web box (admin@10.0.0.5)") > 0, "snippet asked: " + area.questionBox.text);
            area.questionBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            area.questionBox.close();
            expect(!has(tab, "MULTI_A"), "snippet sent after No");
            // Manage snippets…: delete one, add one, OK saves and refills the menu.
            tools.manageSnippets();
            dlg = tools.lastSnippets;
            expect(dlg.visible && dlg.title === "Snippets" && dlg.list.count === 2, "snippets dialog");
            expect(dlg.nameField.text === "uptime" && dlg.commandField.text === "uptime" && dlg.enterBox.checked, "first snippet shown");
            dlg.remove();
            expect(dlg.list.count === 1 && dlg.nameField.text === "logs" && !dlg.enterBox.checked, "after delete " + dlg.nameField.text);
            dlg.add();
            expect(dlg.list.count === 2 && dlg.nameField.text === "New snippet", "added");
            dlg.commandField.text = "df -h {host}";
            expect(dlg.nameField.text === "New snippet", "name kept");
            dlg.nameField.text = "";
            dlg.store();
            same(dlg.snippets().map(s => s.name), ["logs", "df -h {host}"], "name from the command");
            dlg.nameField.text = "disk";
            dlg.store();
            return shot(dlg, "snippets");
        },
        () => {
            dlg.accept();
        },
        () => {
            same(menuTexts(shell.shellActions.menus["Edit/Snippets"]), "logs|disk|-|Manage snippets…", "refilled menu");
        },
        // Bulk edit: only what is ticked comes out.
        () => {
            got = null;
            dlg = shell.openBulkEdit(3, ["lab"], changes => got = changes);
            expect(dlg.visible && dlg.title === "Edit 3 entries", "bulk title " + dlg.title);
            same(dlg.changes(), {}, "nothing ticked");
            dlg.rows.tags.checked = true;
            dlg.tagMode.currentIndex = 1;
            dlg.tagsField.text = "old, stale";
            expect(dlg.rows.credential.enabled, "credential tick disabled with a credential defined");
            dlg.rows.credential.checked = true;
            dlg.credentialBox.currentIndex = 1;
            same(dlg.changes(), {
                "tags_remove": ["old", "stale"],
                "credential": "ops"
            }, "bulk changes");
            return shot(dlg, "bulk");
        },
        () => {
            dlg.accept();
            same(got, {
                "tags_remove": ["old", "stale"],
                "credential": "ops"
            }, "handed over");
            let again = null;
            dlg = shell.openBulkEdit(2, [], c => again = c);
            dlg.accept();
            expect(again === null, "an empty change was handed over");
        },
        // The disconnect notice: a connection tab ends on its own.
        () => {
            area.tabReopened.connect((o, n) => ++check.reopened);
            tab = area.addTab({
                "title": "gone",
                "inner": "sleep 0.4",
                "conn": {
                    "label": "gone",
                    "host": "127.0.0.1",
                    "user": "u",
                    "port": 1
                }
            });
        },
        () => {
            firstNotice = tools.lastNotice;
            if (!firstNotice || !firstNotice.visible || firstNotice.tail === "")
                return false;
            expect(firstNotice.title === "Termigator — session ended", "notice title " + firstNotice.title);
            expect(firstNotice.message.indexOf("<b>gone</b> ended after 0s — it likely never connected.") === 0, "lead " + firstNotice.message);
            expect(firstNotice.modality === Qt.NonModal, "notice is modal");
            return shot(firstNotice, "notice");
        },
        () => {
            // Test connection… opens the probe window over it.
            button(firstNotice, "Test connection…").clicked();
            dlg = tools.lastTest;
            expect(dlg.visible && dlg.title === "Test connection — gone", "probe title " + dlg.title);
        },
        () => {
            if (dlg.running)
                return false;
            expect(dlg.headline.startsWith("✘"), "probe headline " + dlg.headline);
            dlg.close();
            // Reconnect: the same session restarts in its slot.
            button(firstNotice, "Reconnect").clicked();
            expect(!firstNotice.visible && reopened === 1, "reconnect " + reopened);
        },
        // The restarted session ends again: a second notice, muted this time.
        () => {
            const n = tools.lastNotice;
            if (n === firstNotice || !n.visible)
                return false;
            n.muteBox.checked = true;
            n.close();
        },
        () => {
            expect(shell.controller.windowBool("notify_on_disconnect", true) === false, "notice not muted");
            // The tab menu's Test connection….
            area.testConnection(JSON.stringify({
                "label": "tab menu",
                "host": "127.0.0.1",
                "port": 1
            }));
            dlg = tools.lastTest;
            expect(dlg.visible && dlg.title === "Test connection — tab menu", "tab menu probe " + dlg.title);
        },
        () => {
            if (dlg.running)
                return false;
            dlg.close();
            const started = shell.contentArea.grabToImage(result => {
                if (!result.saveToFile(shell.controller.smokeTestShot))
                    check.fail("cannot save the grab");
                console.info("termigator tools smoke: done");
                // The close path ends the tunnels and the sessions.
                shell.requestQuit();
            });
            if (!started)
                fail("grab not started");
            return true;
        }
    ]

    Timer {
        interval: 150
        repeat: true
        running: check.step < check.steps.length
        onTriggered: {
            if (check.step >= check.steps.length)
                return;
            if (check.waitFor) {
                if (!check.waitFor() && ++check.waited <= 100)
                    return;
                check.waitFor = null;
                check.waited = 0;
            }
            const result = check.steps[check.step]();
            if (typeof result === "function") {
                check.waitFor = result;
                ++check.step;
                return;
            }
            if (result === false) {
                if (++check.waited > 100)
                    check.fail("step " + check.step + " did not finish");
                return;
            }
            check.waited = 0;
            ++check.step;
        }
    }
}
