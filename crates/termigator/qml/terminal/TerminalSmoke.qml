import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Window
import Termigator.Terminal.Probe

// Offscreen check (tests/terminal_smoke.rs): real sessions through the shell's hooks, keys
// through Probe, the action table. Every step runs every 100 ms until it returns something other
// than false (a step that waits for a session returns false until the condition holds; 10 s at
// most). It ends with a quit through the close path, so the processes are killed by their tabs.
Item {
    id: check

    required property var shell
    property var area: null
    property var host: null
    property int step: 0
    property int waited: 0
    property bool failed: false
    property int reopened: 0
    property bool grown: false
    property real w0: 0
    property var lastNew: null
    property var held: ({})
    readonly property string stem: shell.controller.smokeTestShot.replace(/shot\.png$/, "")

    function fail(reason) {
        failed = true;
        console.error("termigator u4: FAILED at step " + step + " (" + steps[step].name + "): " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok && !failed)
            fail(what);
        return ok;
    }

    function findArea(item) {
        if (item.objectName === "panesArea")
            return item;
        for (const c of item.children) {
            const f = findArea(c);
            if (f)
                return f;
        }
        return null;
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

    function key(k, modifiers, text) {
        Probe.key(shell, k, modifiers || Qt.NoModifier, text || "");
    }

    // Letters and digits only: their Qt key is the upper-case ASCII code.
    function type(text) {
        for (const ch of text)
            key(ch.toUpperCase().charCodeAt(0), Qt.NoModifier, ch);
        key(Qt.Key_Return, Qt.NoModifier, "\r");
    }

    function has(tab, text) {
        return tab.session !== null && tab.session.ext.history.indexOf(text) >= 0;
    }

    function view(tab) {
        return host.viewFor(tab);
    }

    function focusTab(tab) {
        const pv = area.viewOf(tab);
        area.select(pv, pv.tabIds.indexOf(tab.tabId));
        view(tab).term.forceActiveFocus();
    }

    function altWheel(tab) {
        focusTab(tab);
        const v = view(tab);
        const c = v.term.mapToItem(null, 40, v.term.height / 2);
        Probe.wheel(shell, c.x, c.y, 0, 120, Qt.NoModifier);
    }

    function action(id) {
        shell.shellActions.actions[id].trigger();
    }

    function open(title, inner, conn, extra) {
        return area.addTab(Object.assign({
            "title": title,
            "inner": inner,
            "conn": conn || null
        }, extra || {}));
    }

    readonly property var steps: [
        {
            "name": "area",
            "fn": () => {
                area = findArea(shell.contentItem);
                if (!expect(area !== null, "no PanesArea"))
                    return;
                host = area.terminalHost;
                expect(shell.liveSessions === 0, "live " + shell.liveSessions);
                shell.requestActivate();
            }
        },
        // A tab through the shell's hook: a real session, its view, the settings applied.
        {
            "name": "open",
            "fn": () => {
                shell.openInner("printf 'READY_%s\\n' ONE; exec cat", "one", "embedded", "");
                held.one = area.activeTab;
            }
        },
        {
            "name": "output",
            "fn": () => {
                const t = held.one;
                if (!has(t, "READY_ONE"))
                    return false;
                const v = view(t);
                expect(t.session.core.isRunning() && t.live && shell.liveSessions === 1, "not running / live");
                expect(v.term !== null && v.term.colorScheme !== "" && v.term.font.family === t.session.fontFamily, "look not applied");
                expect(v.term.font.pointSize === t.session.fontSize && t.session.fontSize >= 1, "font size");
                // Preferences → Terminal background: the look's opacity reaches the view; at
                // 100 % the window stays opaque.
                expect(v.term.backgroundOpacity === t.session.look.opacity && t.session.look.opacity === 1 && !shell.seeThrough && shell.color.a === 1, "background " + v.term.backgroundOpacity + " " + shell.seeThrough);
                v.term.backgroundOpacity = 0.7;
                expect(Math.abs(v.term.backgroundOpacity - 0.7) < 1e-6, "opacity not set");
                // The padding's see-through background lies beside the terminal, never under it
                // (two layers would be more opaque than the padding).
                const left = find("termPadLeft", v), right = find("termPadRight", v);
                const termX = v.term.mapToItem(left.parent, 0, 0).x;
                expect(left && right && Math.abs(left.color.a - 0.7) < 0.01 && left.x + left.width <= termX + 0.5 && right.x >= termX + v.term.width - 0.5, "padding under the terminal " + JSON.stringify([left.x, left.width, termX, v.term.width, right.x]));
                v.term.backgroundOpacity = t.session.look.opacity;
                expect(t.iconName === "" && t.toolTip === "", "icon / tip " + t.iconName);
                v.term.forceActiveFocus();
                type("abc");
            }
        },
        {
            "name": "typed",
            "fn": () => has(held.one, "abc")
        },
        // Zoom: Ctrl+=, Ctrl+-, Ctrl+0 on the focused terminal (ADR-11).
        {
            "name": "zoom in",
            "fn": () => {
                held.size = view(held.one).term.font.pointSize;
                key(Qt.Key_Equal, Qt.ControlModifier, "=");
            }
        },
        {
            "name": "zoomed",
            "fn": () => {
                const v = view(held.one);
                expect(v.term.font.pointSize === held.size + 1, "Ctrl+= " + v.term.font.pointSize);
                key(Qt.Key_Equal, Qt.ControlModifier, "=");
            }
        },
        {
            "name": "zoom twice",
            "fn": () => {
                expect(view(held.one).term.font.pointSize === held.size + 2, "second Ctrl+=");
                key(Qt.Key_Minus, Qt.ControlModifier, "-");
            }
        },
        {
            "name": "zoom out",
            "fn": () => {
                expect(view(held.one).term.font.pointSize === held.size + 1, "Ctrl+-");
                key(Qt.Key_0, Qt.ControlModifier, "0");
            }
        },
        {
            "name": "zoom reset",
            "fn": () => {
                expect(view(held.one).term.font.pointSize === held.size, "Ctrl+0 " + view(held.one).term.font.pointSize);
                action("view.zoom_in");
            }
        },
        {
            "name": "zoom action",
            "fn": () => {
                expect(view(held.one).term.font.pointSize === held.size + 1, "View → Zoom in");
                action("view.zoom_reset");
            }
        },
        // Find bar: Edit → Find…, typing finds, closing returns the focus.
        {
            "name": "find",
            "fn": () => {
                expect(view(held.one).term.font.pointSize === held.size, "View → Reset zoom");
                action("edit.find");
            }
        },
        {
            "name": "find typed",
            "fn": () => {
                const v = view(held.one);
                if (!expect(v.findBar.visible, "find bar not shown"))
                    return;
                v.findBar.field.text = "READY_ONE";
            }
        },
        {
            "name": "found",
            "fn": () => {
                if (view(held.one).term.selectedText() !== "READY_ONE")
                    return false;
                view(held.one).findBar.close();
            }
        },
        {
            "name": "find closed",
            "fn": () => {
                const v = view(held.one);
                return !v.findBar.visible && v.term.activeFocus;
            }
        },
        // Clipboard: paste, and a pasted script asks first.
        {
            "name": "paste",
            "fn": () => {
                host.copyText("PASTED_TEXT");
                action("edit.paste");
            }
        },
        {
            "name": "pasted",
            "fn": () => {
                if (!has(held.one, "PASTED_TEXT"))
                    return false;
                host.copyText("ML_ONE\nML_TWO");
                // Through the context menu, which hands the focus back to the terminal after its entry.
                const v = view(held.one);
                v.showMenu(10, 10);
                // "Show menubar" and a separator come first while the menu bar is hidden (the default).
                const item = v.menu.itemAt(3);
                expect(item.text === "Paste", "menu entry " + item.text);
                item.triggered();
                v.menu.close();
            }
        },
        {
            "name": "multiline asked",
            "fn": () => {
                const v = view(held.one);
                if (!v.pasteDialog.visible)
                    return false;
                expect(!has(held.one, "ML_TWO") && v.pasteDialog.text.indexOf("ML_TWO") >= 0, "pasted before the answer");
                // Enter right away answers Yes, without a Tab or a click first.
                key(Qt.Key_Return, Qt.NoModifier, "\r");
            }
        },
        {
            "name": "multiline pasted",
            "fn": () => has(held.one, "ML_TWO")
        },
        // The context menu, in its order.
        {
            "name": "menu",
            "fn": () => {
                const v = view(held.one);
                v.showMenu(10, 10);
                const texts = [];
                for (let i = 0; i < v.menu.count; ++i) {
                    const item = v.menu.itemAt(i);
                    texts.push(item.text === undefined || item.text === "" ? "-" : item.text);
                }
                expect(texts.join("|") === "Show menubar|-|Copy|Paste|Paste Selection|-|Enable folder tracking in this shell|-|Find…|-|Zoom in|Zoom out|Reset zoom|-|Clear|-|Log output to file…", "menu " + texts.join("|"));
                v.menu.close();
            }
        },
        // OSC 7 gives the Files panel the shell's directory.
        {
            "name": "osc7",
            "fn": () => {
                held.osc = open("osc7", "printf '\\033]7;file://h/tmp/x%%20y\\007OSC_DONE\\n'; exec cat");
            }
        },
        {
            "name": "osc7 cwd",
            "fn": () => {
                if (held.osc.session.remoteCwd === "")
                    return false;
                expect(held.osc.session.remoteCwd === "/tmp/x y", "cwd " + held.osc.session.remoteCwd);
            }
        },
        // The Files panel follows a local shell: its folder is read from the process.
        {
            "name": "local cwd",
            "fn": () => {
                held.lcwd = open("lcwd", "cd /tmp && exec cat");
            }
        },
        {
            "name": "local cwd read",
            "fn": () => {
                // The login shell starts in $HOME; the typed "cd /tmp" reaches it a moment later.
                // Wait for the change instead of failing on the starting directory.
                const cwd = held.lcwd.session.localCwd();
                if (cwd !== "/tmp") {
                    held.lcwdWaited = (held.lcwdWaited || 0) + 1;
                    if (held.lcwdWaited > 100)
                        expect(false, "cwd " + cwd);
                    return false;
                }
            }
        },
        // The distro lands in the saved entry (F-distro), once.
        {
            "name": "distro",
            "fn": () => {
                held.dist = open("dist", "printf 'ID=ubuntu\\n'; exec cat", {
                    "label": "dist",
                    "host": "dh"
                }, {
                    "detectDistro": true
                });
            }
        },
        {
            "name": "distro seen",
            "fn": () => {
                if (held.dist.conn.distro === undefined)
                    return false;
                expect(held.dist.conn.distro === "ubuntu", "distro " + held.dist.conn.distro);
            }
        },
        // Startup commands: typed once the output has been quiet, never into a prompt.
        {
            "name": "startup",
            "fn": () => {
                held.su = open("su", "printf 'login> \\n'; exec cat", {
                    "name": "su",
                    "host": "sh",
                    "startup_commands": "echo STARTUP_OK"
                });
            }
        },
        {
            "name": "startup typed",
            "fn": () => has(held.su, "echo STARTUP_OK")
        },
        // Session log and recording: the tab shows both; the Rust side reads the files.
        {
            "name": "log",
            "fn": () => {
                held.lg = open("lg", "exec cat");
                if (!held.lg.session)
                    return false;
                const err = held.lg.session.core.startLog(stem + "u4.log");
                expect(err === "", "log: " + err);
            }
        },
        {
            "name": "logging",
            "fn": () => {
                const t = held.lg;
                expect(t.logActive && t.textColor.a > 0 && t.toolTip.indexOf("Logging to ") === 0, "log marker " + t.textColor + " " + t.toolTip);
                t.session.ext.sendText("LOGGED_TEXT\n");
            }
        },
        {
            "name": "logged",
            "fn": () => {
                if (!has(held.lg, "LOGGED_TEXT"))
                    return false;
                const err = held.lg.session.startRecording(stem + "u4.cast", "lg");
                expect(err === "", "recording: " + err);
            }
        },
        {
            "name": "recording",
            "fn": () => {
                const t = held.lg;
                expect(t.recordingActive && t.iconName === "media-record" && t.toolTip === "Recording to " + stem + "u4.cast", "recording state " + t.iconName + " " + t.toolTip);
                t.session.ext.sendText("REC_TEXT\n");
            }
        },
        {
            "name": "recorded",
            "fn": () => {
                if (!has(held.lg, "REC_TEXT"))
                    return false;
                held.lg.session.stopRecording();
                held.lg.session.core.stopLog();
            }
        },
        {
            "name": "stopped",
            "fn": () => {
                const t = held.lg;
                if (t.logActive)
                    return false;
                expect(!t.recordingActive && t.iconName === "" && t.textColor.a === 0, "markers not cleared");
                console.info("U4_LOG " + stem + "u4.log");
            }
        },
        // "Session stopped": the overlay, R restarts in place, Enter closes the tab.
        {
            "name": "dead",
            "fn": () => {
                held.dead = open("dead", "printf 'DEAD_OUT\\n'; exit 3");
            }
        },
        {
            "name": "overlay",
            "fn": () => {
                const t = held.dead;
                if (!t.session.finished)
                    return false;
                expect(t.session.overlayText === "Session stopped\n    –  Press R to restart session\n    –  Press <Enter> to exit tab\n    –  Press S to save terminal output to\u00a0file", "overlay " + t.session.overlayText);
                expect(t.iconName === "network-disconnect" && t.toolTip === "Session stopped — R restarts it, Enter closes the tab, S saves the output", "icon / tip " + t.iconName);
                expect(!t.live && has(t, "DEAD_OUT"), "live / output");
                expect(view(t).term.activeFocus, "the overlay did not keep the focus in its tab");
                key(Qt.Key_R, Qt.NoModifier, "r");
            }
        },
        {
            "name": "restarted",
            "fn": () => {
                if (reopened < 1)
                    return false;
                held.dead2 = lastNew;
                expect(area.tabList.indexOf(held.dead) < 0 && lastNew.title === "dead", "not replaced");
            }
        },
        {
            "name": "dead again",
            "fn": () => {
                const t = held.dead2;
                if (!t.session || !t.session.finished || !t.session.overlayActive)
                    return false;
                focusTab(t);
                held.tabsBeforeFind = area.tabList.length;
                action("edit.find");
            }
        },
        // R and S typed into the find bar of a stopped session are text, not the overlay's
        // restart / save keys.
        {
            "name": "find on a stopped session",
            "fn": () => {
                const v = view(held.dead2);
                if (!v.findBar.visible || !v.findBar.field.activeFocus)
                    return false;
                key(Qt.Key_R, Qt.NoModifier, "r");
                key(Qt.Key_S, Qt.NoModifier, "s");
            }
        },
        {
            "name": "find kept the keys",
            "fn": () => {
                const t = held.dead2, v = view(t);
                expect(v.findBar.field.text === "rs", "find field " + JSON.stringify(v.findBar.field.text));
                expect(area.tabList.indexOf(t) >= 0 && area.tabList.length === held.tabsBeforeFind, "R restarted the session");
                expect(t.session.finished && t.session.overlayActive, "the overlay is gone");
                v.findBar.close();
            }
        },
        {
            "name": "find closed on a stopped session",
            "fn": () => {
                const v = view(held.dead2);
                if (v.findBar.visible || !v.term.activeFocus)
                    return false;
                key(Qt.Key_Return, Qt.NoModifier, "\r");
            }
        },
        {
            "name": "closed by Enter",
            "fn": () => area.tabList.indexOf(held.dead2) < 0
        },
        // Ctrl+letter reaches the program as its control byte (the event text Qt really sends is
        // the byte itself): Ctrl+C interrupts a sleep, Ctrl+D ends a cat (k9s, vim, shells use them).
        {
            "name": "ctrl open",
            "fn": () => {
                held.ctrl = open("ctrl", "trap 'echo GOT_INT' INT; printf 'CTRL_READY\\n'; sleep 30; echo AFTER_INT; cat; echo CAT_ENDED; sleep 30");
            }
        },
        {
            "name": "ctrl c",
            "fn": () => {
                if (!has(held.ctrl, "CTRL_READY"))
                    return false;
                focusTab(held.ctrl);
                key(Qt.Key_C, Qt.ControlModifier, "\x03");
            }
        },
        {
            "name": "interrupted",
            "fn": () => has(held.ctrl, "GOT_INT") && has(held.ctrl, "AFTER_INT")
        },
        {
            "name": "ctrl d",
            "fn": () => {
                key(Qt.Key_D, Qt.ControlModifier, "\x04");
            }
        },
        {
            "name": "eof",
            "fn": () => {
                if (!has(held.ctrl, "CAT_ENDED"))
                    return false;
                area.closeTab(held.ctrl);
            }
        },
        // Shift+Tab is Qt's Key_Backtab: the program gets CSI Z (shell completion, mc, dialog forms).
        {
            "name": "backtab open",
            "fn": () => {
                held.backtab = open("backtab", "stty -icanon -echo; printf 'BT_READY\\n'; exec cat -v");
            }
        },
        {
            "name": "backtab key",
            "fn": () => {
                if (!has(held.backtab, "BT_READY"))
                    return false;
                focusTab(held.backtab);
                key(Qt.Key_Backtab, Qt.ShiftModifier, "");
            }
        },
        {
            "name": "backtab reached",
            "fn": () => {
                if (!has(held.backtab, "^[[Z"))
                    return false;
                area.closeTab(held.backtab);
            }
        },
        // Auto-reconnect: the countdown on the tab, then a new session in the same slot.
        {
            "name": "auto",
            "fn": () => {
                reopened = 0;
                held.auto = open("ar", "exit 1", {
                    "name": "ar",
                    "host": "x",
                    "auto_reconnect": true
                });
            }
        },
        {
            "name": "countdown",
            "fn": () => {
                if (held.auto.text.indexOf("ar · retry in ") !== 0)
                    return false;
                expect(!held.auto.session.overlayActive, "overlay during the countdown");
            }
        },
        {
            "name": "reconnected",
            "fn": () => {
                if (reopened < 1)
                    return false;
                held.auto2 = lastNew;
                expect(held.auto2.conn.auto_reconnect === true && area.tabList.indexOf(held.auto) < 0, "not replaced");
            }
        },
        {
            "name": "backoff",
            "fn": () => {
                // The replacement died at once too: attempt 1 waits 6 s.
                if (held.auto2.text !== "ar · retry in 6s" && held.auto2.text.indexOf("ar · retry in ") !== 0)
                    return false;
                expect(held.auto2.text.indexOf("ar · retry in ") === 0, "second countdown " + held.auto2.text);
                area.closeTab(held.auto2);
            }
        },
        // Bell / activity in background tabs; looking at the tab clears them.
        {
            "name": "background",
            "fn": () => {
                held.bell = open("bell", "sleep 1; printf '\\a'; exec cat");
                held.act = open("act", "sleep 1; echo ACTIVITY; exec cat");
                held.fg = open("fg", "exec cat");
            }
        },
        {
            "name": "marked",
            "fn": () => {
                if (held.bell.iconName !== "notifications" || held.act.iconName !== "dialog-information")
                    return false;
                expect(held.bell.toolTip === "Bell in this session" && held.act.toolTip === "New output in this session", "tips");
                focusTab(held.bell);
            }
        },
        {
            "name": "seen",
            "fn": () => {
                if (held.bell.iconName !== "")
                    return false;
                expect(held.bell.toolTip === "", "tip after looking");
            }
        },
        // MultiExec: a key typed in one remote session goes to all of them; Exclude; Broadcast.
        {
            "name": "multiexec",
            "fn": () => {
                held.mx1 = open("mx1", "exec cat", {
                    "name": "mx1",
                    "host": "m1",
                    "environment": "production"
                });
                held.mx2 = open("mx2", "exec cat", {
                    "name": "mx2",
                    "host": "m2"
                });
            }
        },
        {
            "name": "multiexec on",
            "fn": () => {
                if (!held.mx2.session)
                    return false;
                // The frame is opt-in (window.environment_frame); a production session then
                // gets one, a session without an environment never does.
                // (A tab in the background is not visible at all: look at the mark's own state.)
                const mark = v => v.children.find(c => c.objectName === "envFrame");
                const framed = v => mark(v).envColor.a > 0 && mark(v).frameEnabled;
                expect(held.mx1.environment === "production" && mark(view(held.mx1)).envColor.a > 0, "production colour");
                expect(!framed(view(held.mx1)), "no frame by default");
                shell.controller.setWindowBool("environment_frame", true);
                shell.controller.persist();
                expect(framed(view(held.mx1)), "production frame once enabled");
                expect(held.mx2.environment === "" && !framed(view(held.mx2)), "no frame without an environment");
                focusTab(held.mx1);
                action("session.multiexec");
            }
        },
        {
            "name": "multiexec typed",
            "fn": () => {
                const n = host.multiexecTargets().length;
                expect(host.multiexecOn && n >= 2 && shell.multiexecTargets === n, "targets " + shell.multiexecTargets);
                expect(host.multiexecTargets().indexOf(held.osc) < 0 && host.multiexecTargets().indexOf(held.mx2) >= 0, "target rules");
                expect(held.mx2.multiexecTarget && !held.osc.multiexecTarget, "tab markers");
                expect(shell.multiexecNames.indexOf("mx2 (m2)") >= 0, "banner names: " + shell.multiexecNames);
                // The text sits inside the frame with a padding, not under it (mx1 is shown now).
                const v1 = view(held.mx1);
                const frame = v1.children.find(c => c.objectName === "envFrame");
                const at = v1.term.mapToItem(v1, 0, 0);
                expect(frame.visible && at.x >= frame.frameWidth + 2 && at.y >= frame.frameWidth, "text under the frame: " + at.x + "," + at.y);
                held.targets = n;
                type("mxkey");
                view(held.mx1).term.pasteText("mxpaste", false); // A paste goes to the targets too
                // A session opened while MultiExec is on stays out until included.
                held.mx3 = open("mx3", "exec cat", {
                    "name": "mx3",
                    "host": "m3"
                });
            }
        },
        {
            "name": "multiexec seen",
            "fn": () => {
                if (!has(held.mx1, "mxkey") || !has(held.mx2, "mxkey") || !has(held.mx2, "mxpaste") || !held.mx3.multiexecExcluded)
                    return false;
                expect(!held.mx3.multiexecTarget && shell.multiexecTargets === held.targets, "a new session joined MultiExec");
                expect(!has(held.osc, "mxkey") && !has(held.lg, "mxkey"), "a local shell got the keys");
                held.mx2.multiexecExcluded = true;
                expect(shell.multiexecTargets === held.targets - 1, "Exclude: " + shell.multiexecTargets);
                focusTab(held.mx1); // mx3 opened last and took the focus
                type("afterexcl");
            }
        },
        {
            "name": "excluded",
            "fn": () => {
                if (!has(held.mx1, "afterexcl"))
                    return false;
                expect(!has(held.mx2, "afterexcl"), "an excluded session got the keys");
                action("session.multiexec");
                expect(!host.multiexecOn && shell.multiexecTargets === -1, "MultiExec not off");
                host.broadcastSend(host.broadcastTargets(), "bcastline");
            }
        },
        // A window Shortcut still fires with the terminal focused.
        {
            "name": "shortcut beats the terminal",
            "fn": () => {
                focusTab(held.mx1);
                key(Qt.Key_M, Qt.ControlModifier | Qt.AltModifier, "\r");
                return host.multiexecOn;
            }
        },
        {
            "name": "shortcut beats the terminal off",
            "fn": () => {
                expect(host.multiexecTargets().indexOf(held.mx1) >= 0, "targets via Ctrl+Alt+M");
                key(Qt.Key_M, Qt.ControlModifier | Qt.AltModifier, "\r");
                return !host.multiexecOn;
            }
        },
        {
            "name": "broadcast",
            "fn": () => {
                if (!has(held.mx1, "bcastline") || !has(held.su, "bcastline"))
                    return false;
                expect(!has(held.mx2, "bcastline"), "an excluded session got the broadcast");
            }
        },
        // The scroll indicator is a slider (patch 0018) and still auto-hides; the wheel and
        // the selection keep working. The session prints 3000 lines to have a scrollback.
        {
            "name": "scrollback",
            "fn": () => {
                held.scroll = open("scroll", "sleep 2; seq 1 3000; exec cat");
            }
        },
        // The bar is shown even before any scrollback (a full-length handle, like Konsole), so the
        // terminal keeps its width when output starts to scroll (a width change re-wraps the output).
        {
            "name": "scrollback width kept",
            "fn": () => {
                const v = view(held.scroll);
                const thumb = v ? find("termScrollThumb", v) : null;
                if (!v || !v.term || !thumb || v.term.lines <= 0) // not laid out yet
                    return false;
                expect(thumb.visible && thumb.size > 0.99, "no full-length bar without scrollback: " + thumb.visible + " " + thumb.size);
                held.scrollWidth = v.term.width;
                const left = v.term.mapToItem(thumb.parent, 0, 0).x; // the text's left padding
                expect(Math.abs(left + v.term.width + thumb.width - thumb.parent.width) < 1, "no room kept for the bar: " + left + " + " + v.term.width + " + " + thumb.width + " != " + thumb.parent.width);
            }
        },
        {
            "name": "scrollback ready",
            "fn": () => {
                const t = held.scroll, v = view(t);
                if (!v.term || !has(t, "3000") || v.term.scrollbarMaximum <= 0)
                    return false;
                expect(v.term.width === held.scrollWidth, "the terminal width changed with the bar: " + held.scrollWidth + " -> " + v.term.width);
                // Let the last output arrive: another chunk would scroll us back to the end.
                const max = v.term.scrollbarMaximum;
                if (held.scrollMax !== max) {
                    held.scrollMax = max;
                    return false;
                }
                expect(v.term.scrollbarCurrentValue === max, "not at the end: " + v.term.scrollbarCurrentValue + "/" + max);
                const mid = Math.floor(max / 2);
                v.scrollToValue(mid);
                expect(v.term.scrollbarCurrentValue === mid, "slider set " + v.term.scrollbarCurrentValue + " != " + mid);
                expect(!t.session.finished, "the session ended");
            }
        },
        // A plain left-button drag over the text still selects it (the terminal's local mouse
        // handling, which the wheel path shares).
        {
            "name": "scrollback selection",
            "fn": () => {
                const v = view(held.scroll);
                const a = v.term.mapToItem(null, 20, v.term.height / 2);
                Probe.drag(shell, a.x, a.y, a.x + 160, a.y, Qt.LeftButton, Qt.NoModifier);
                expect(v.term.selectedText().length > 0, "a mouse drag did not select text");
            }
        },
        // Mouse presses and moves are delivered synchronously; the value is already moved here.
        {
            "name": "scrollback dragged",
            "fn": () => {
                const v = view(held.scroll);
                const thumb = find("termScrollThumb", v);
                if (!thumb || !thumb.visible)
                    return false; // the fade-in after the value change
                const p = thumb.mapToItem(null, thumb.width / 2, thumb.height * (thumb.position + thumb.size / 2));
                held.dragFrom = v.term.scrollbarCurrentValue;
                Probe.drag(shell, p.x, p.y, p.x, p.y + 40, Qt.LeftButton, Qt.NoModifier);
                expect(v.term.scrollbarCurrentValue > held.dragFrom, "the thumb did not drag: " + held.dragFrom + " -> " + v.term.scrollbarCurrentValue);
            }
        },
        // Wheel events go through the window system queue, so the value is read on the next tick.
        {
            "name": "scrollback wheel",
            "fn": () => {
                const v = view(held.scroll);
                const c = v.term.mapToItem(null, 40, v.term.height / 2);
                held.beforeWheel = v.term.scrollbarCurrentValue;
                Probe.wheel(shell, c.x, c.y, 0, 120, Qt.NoModifier);
            }
        },
        {
            "name": "scrollback wheel scrolled",
            "fn": () => {
                const v = view(held.scroll);
                expect(v.term.scrollbarCurrentValue < held.beforeWheel, "the wheel stopped scrolling: " + held.beforeWheel + " -> " + v.term.scrollbarCurrentValue);
                v.scrollToValue(v.term.scrollbarMaximum);
                expect(v.term.scrollbarCurrentValue === v.term.scrollbarMaximum, "not back at the end");
            }
        },
        // Shift+PgUp pages back through the scrollback; the next key typed returns to the bottom.
        {
            "name": "scrollback shift page up",
            "fn": () => {
                focusTab(held.scroll);
                key(Qt.Key_PageUp, Qt.ShiftModifier, "");
            }
        },
        {
            "name": "scrollback paged",
            "fn": () => {
                const v = view(held.scroll);
                if (v.term.scrollbarCurrentValue === v.term.scrollbarMaximum)
                    return false;
                expect(v.term.scrollbarMaximum - v.term.scrollbarCurrentValue === v.term.lines - 1, "not a page: " + v.term.scrollbarCurrentValue + "/" + v.term.scrollbarMaximum);
                key(Qt.Key_X, Qt.NoModifier, "x");
            }
        },
        {
            "name": "scrollback back at the bottom",
            "fn": () => {
                const v = view(held.scroll);
                return v.term.scrollbarCurrentValue === v.term.scrollbarMaximum;
            }
        },
        // On the alternate screen (less, vim, k9s) there is no scrollback: without mouse tracking
        // a notch is three Up keys (alternate scroll, DECSET 1007); with tracking it is one SGR
        // report. `cat -v` shows what the program got, and that it got it once.
        {
            "name": "alternate wheel",
            "fn": () => {
                const raw = "stty -icanon -echo; printf '\\033[?1049h";
                held.altKeys = open("altkeys", raw + "ready'; exec cat -v");
                held.altMouse = open("altmouse", raw + "\\033[?1000h\\033[?1006hready'; exec cat -v");
            }
        },
        {
            "name": "alternate wheel keys",
            "fn": () => {
                for (const t of [held.altKeys, held.altMouse]) {
                    const v = view(t);
                    if (!v || !v.term || v.term.lines <= 0 || !has(t, "ready"))
                        return false;
                }
                altWheel(held.altKeys);
            }
        },
        // Wheel events are queued: the tab changes only after the first one arrived.
        {
            "name": "alternate wheel mouse",
            "fn": () => {
                if (!has(held.altKeys, "^[[A"))
                    return false;
                altWheel(held.altMouse);
            }
        },
        {
            "name": "alternate wheel received",
            "fn": () => {
                const keys = held.altKeys.session.ext.history;
                const mouse = held.altMouse.session.ext.history;
                if (keys.indexOf("^[[A^[[A^[[A") < 0 || mouse.indexOf("^[[<64;") < 0)
                    return false;
                expect(keys.split("^[[A").length - 1 === 3, "a notch is not three Up keys: " + keys);
                expect(mouse.split("^[[<64;").length - 1 === 1, "a notch is not one report: " + mouse);
                expect(keys.indexOf("^[[<") < 0 && mouse.indexOf("^[[A") < 0, "the wheel went both ways");
                held.altDrag = open("altdrag", "stty -icanon -echo; printf '\\033[?1049h\\033[?1002h\\033[?1006hready'; exec cat -v");
            }
        },
        // A program that tracks drags only (DECSET 1002: vim with 'mouse=a', tmux, mc) is told
        // about a motion only with the held button in it.
        {
            "name": "drag tracking",
            "fn": () => {
                const v = view(held.altDrag);
                if (!v || !v.term || v.term.lines <= 0 || !has(held.altDrag, "ready"))
                    return false;
                focusTab(held.altDrag);
                const a = v.term.mapToItem(null, 20, 20);
                const b = v.term.mapToItem(null, 220, 20);
                Probe.drag(shell, a.x, a.y, b.x, b.y, Qt.LeftButton, Qt.NoModifier);
            }
        },
        {
            "name": "drag tracking received",
            "fn": () => {
                const got = held.altDrag.session.ext.history;
                if (got.indexOf("^[[<32;") < 0)
                    return false;
                expect(got.indexOf("^[[<0;") >= 0, "no press before the drag: " + got);
                // Asks for the cell size (CSI 16 t) once the view has sized the session: a key.
                held.pixels = open("pixels", "stty -icanon -echo; printf ready; read -r -n 1 _; printf '\\033[16t'; exec cat -v");
                focusTab(held.scroll); // the autoscroll steps drag over it
            }
        },
        // Image tools (chafa, sixel, the kitty protocol) size their output by the cell size the
        // terminal reports: the view's font, not a fixed 8×16.
        {
            "name": "pixel size report",
            "fn": () => {
                const v = view(held.pixels);
                if (!v || !v.term || v.term.lines <= 0 || !has(held.pixels, "ready"))
                    return false;
                held.pixels.session.ext.sendText("x");
            }
        },
        {
            "name": "pixel size report received",
            "fn": () => {
                const got = held.pixels.session.ext.history;
                if (got.indexOf("^[[6;") < 0)
                    return false;
                const v = view(held.pixels);
                const want = "^[[6;" + Math.floor(v.term.cellHeight) + ";" + Math.floor(v.term.cellWidth) + "t";
                expect(got.indexOf(want) >= 0, "the cell size report is not the view's (" + want + "): " + got);
            }
        },
        // Dragging a selection above the rows scrolls the scrollback under it until the button
        // goes up; the selection keeps its anchor, so it covers every row scrolled through.
        {
            "name": "autoscroll up",
            "fn": () => {
                const v = view(held.scroll);
                held.autoFrom = v.term.scrollbarCurrentValue;
                const a = v.term.mapToItem(null, 30, 2);
                held.autoAt = v.term.mapToItem(null, 30, -48);
                Probe.hold(shell, a.x, a.y, held.autoAt.x, held.autoAt.y, Qt.LeftButton, Qt.NoModifier);
            }
        },
        {
            "name": "autoscroll up scrolled",
            "fn": () => {
                const v = view(held.scroll);
                if (v.term.scrollbarCurrentValue > held.autoFrom - 30)
                    return false;
                Probe.release(shell, held.autoAt.x, held.autoAt.y, Qt.LeftButton, Qt.NoModifier);
                held.autoStop = v.term.scrollbarCurrentValue;
                held.autoText = v.term.selectedText();
                const lines = held.autoText.trim().split("\n");
                expect(lines.length >= held.autoFrom - held.autoStop, "the selection lost rows: " + lines.length + " for " + (held.autoFrom - held.autoStop) + " scrolled");
                for (let i = 1; i + 2 < lines.length; ++i)
                    expect(parseInt(lines[i + 1]) === parseInt(lines[i]) + 1, "the selection is not contiguous at '" + lines[i] + "', '" + lines[i + 1] + "'");
                held.autoTicks = 0;
            }
        },
        {
            "name": "autoscroll up stopped",
            "fn": () => {
                const v = view(held.scroll);
                if (++held.autoTicks < 4)
                    return false;
                expect(v.term.scrollbarCurrentValue === held.autoStop, "kept scrolling after the release: " + held.autoStop + " -> " + v.term.scrollbarCurrentValue);
                expect(v.term.selectedText() === held.autoText, "the release changed the selection");
                // The other way: from the last row down past the bottom edge.
                const a = v.term.mapToItem(null, 30, v.term.height - 4);
                held.autoAt = v.term.mapToItem(null, 30, v.term.height + 48);
                Probe.hold(shell, a.x, a.y, held.autoAt.x, held.autoAt.y, Qt.LeftButton, Qt.NoModifier);
            }
        },
        {
            "name": "autoscroll down scrolled",
            "fn": () => {
                const v = view(held.scroll);
                if (v.term.scrollbarCurrentValue < held.autoStop + 20)
                    return false;
                Probe.release(shell, held.autoAt.x, held.autoAt.y, Qt.LeftButton, Qt.NoModifier);
                const lines = v.term.selectedText().trim().split("\n");
                expect(lines.length >= 20, "the downward selection has " + lines.length + " rows");
                for (let i = 1; i + 2 < lines.length; ++i)
                    expect(parseInt(lines[i + 1]) === parseInt(lines[i]) + 1, "the selection is not contiguous at '" + lines[i] + "', '" + lines[i + 1] + "'");
                v.scrollToValue(v.term.scrollbarMaximum);
            }
        },
        // A double-click selects a word, a middle-click pastes it and leaves the clipboard alone
        // (offscreen has no primary selection, so this is the app's own copy, as on Windows).
        {
            "name": "middle open",
            "fn": () => {
                held.mid = open("mid", "printf 'MID%s\\n' WORD; exec cat");
            }
        },
        {
            "name": "middle click",
            "fn": () => {
                const t = held.mid, v = view(t);
                if (!v || !v.term || !has(t, "MIDWORD"))
                    return false;
                focusTab(t);
                host.copyText("CLIPKEEP");
                const p = v.term.mapToItem(null, 4, 4);
                Probe.click(shell, p.x, p.y, Qt.LeftButton, 2, Qt.NoModifier);
                expect(v.term.selectedText() === "MIDWORD", "double-click selected " + v.term.selectedText());
                Probe.click(shell, p.x, p.y, Qt.MiddleButton, 1, Qt.NoModifier);
            }
        },
        {
            "name": "middle pasted",
            "fn": () => {
                const t = held.mid;
                if (t.session.ext.history.split("MIDWORD").length < 3)
                    return false;
                expect(!has(t, "CLIPKEEP"), "the middle-click pasted the clipboard");
            }
        },
        // A program that reports the mouse gets plain drags; Shift selects and pastes anyway.
        {
            "name": "shift open",
            "fn": () => {
                held.mouse = open("mouse", "printf '\\033[?1000hSHIFT%s\\n' WORD; exec cat");
            }
        },
        {
            "name": "shift select",
            "fn": () => {
                const t = held.mouse, v = view(t);
                if (!v || !v.term || !has(t, "SHIFTWORD"))
                    return false;
                focusTab(t);
                const p = v.term.mapToItem(null, 4, 4);
                Probe.drag(shell, p.x, p.y, p.x + 160, p.y, Qt.LeftButton, Qt.NoModifier);
                expect(v.term.selectedText() === "", "a drag selected text the program asked for: " + v.term.selectedText());
                Probe.drag(shell, p.x, p.y, p.x + 160, p.y, Qt.LeftButton, Qt.ShiftModifier);
                held.shiftSel = v.term.selectedText().trim();
                expect(held.shiftSel.length > 2 && "SHIFTWORD".startsWith(held.shiftSel), "Shift+drag selected '" + held.shiftSel + "'");
                Probe.click(shell, p.x, p.y + 40, Qt.MiddleButton, 1, Qt.ShiftModifier);
            }
        },
        {
            "name": "shift pasted",
            "fn": () => held.mouse.session.ext.history.split(held.shiftSel).length >= 3
        },
        // The option off: a middle-click pastes nothing (the typed marker follows it on the line).
        {
            "name": "middle off",
            "fn": () => {
                const t = held.mid, v = view(t);
                focusTab(t);
                v.term.pasteOnMiddleClick = false;
                const p = v.term.mapToItem(null, 4, 4);
                Probe.click(shell, p.x, p.y, Qt.MiddleButton, 1, Qt.NoModifier);
                type("MIDOFF");
            }
        },
        // A selection of two lines asks before a middle-click pastes it, like any paste.
        {
            "name": "middle multiline",
            "fn": () => {
                const t = held.mid, v = view(t);
                if (!has(t, "MIDOFF"))
                    return false;
                // cat still holds the earlier paste: its line is MIDWORDMIDOFF unless the click pasted.
                expect(has(t, "MIDWORDMIDOFF") && !has(t, "MIDWORDMIDWORD"), "pasted with the option off");
                v.term.pasteOnMiddleClick = true;
                const row = v.term.height / v.term.lines;
                const a = v.term.mapToItem(null, 4, row / 2), b = v.term.mapToItem(null, 60, row * 1.5);
                Probe.drag(shell, a.x, a.y, b.x, b.y, Qt.LeftButton, Qt.NoModifier);
                expect(v.term.selectedText().indexOf("\n") > 0, "two-line selection " + JSON.stringify(v.term.selectedText()));
                Probe.click(shell, b.x, b.y, Qt.MiddleButton, 1, Qt.NoModifier);
            }
        },
        {
            "name": "middle multiline asked",
            "fn": () => {
                const v = view(held.mid);
                if (!v.pasteDialog.visible)
                    return false;
                expect(v.pasteDialog.text.indexOf("MIDWORD\n") === 0, "asked about " + JSON.stringify(v.pasteDialog.text));
                // The first Tab goes to Yes, the second to No, and Enter answers with it.
                const yes = v.pasteDialog.footer.standardButton(QQC2.DialogButtonBox.Yes);
                const no = v.pasteDialog.footer.standardButton(QQC2.DialogButtonBox.No);
                expect(!yes.activeFocus && !no.activeFocus, "a button focused before Tab");
                key(Qt.Key_Tab);
                expect(yes.activeFocus, "first Tab on " + shell.activeFocusItem);
                key(Qt.Key_Tab);
                expect(no.activeFocus, "second Tab on " + shell.activeFocusItem);
                held.midAccepted = false;
                v.pasteDialog.accepted.connect(() => held.midAccepted = true);
                key(Qt.Key_Return, Qt.NoModifier, "\r");
            }
        },
        {
            "name": "middle multiline answered",
            "fn": () => {
                if (view(held.mid).pasteDialog.visible)
                    return false;
                expect(!held.midAccepted, "Enter on No pasted");
            }
        },
        // The copy and paste keys, copy on select, paste on a right-click,
        // bracketed paste and links, all through real input on a `cat`.
        {
            "name": "clip open",
            "fn": () => {
                held.clip = open("clip", "printf 'COPY%s\\n' WORD; printf '\\033]8;;https://example.test/ok\\033\\\\OKLINK\\033]8;; \\033\\\\ \\033]8;;javascript:alert(1)\\033\\\\BADLINK\\033]8;;\\033\\\\\\n'; exec cat");
            }
        },
        {
            "name": "copy keys",
            "fn": () => {
                const t = held.clip, v = view(t);
                // The paste dialog rejected above stays visible (and modal) during its exit
                // transition and would take this click.
                if (!v || !v.term || !has(t, "BADLINK") || view(held.mid).pasteDialog.visible)
                    return false;
                expect(!v.pasteDialog.visible, "a paste dialog left open");
                focusTab(t);
                host.copyText("NOT_COPIED");
                const p = v.term.mapToItem(null, 4, 4);
                Probe.click(shell, p.x, p.y, Qt.LeftButton, 2, Qt.NoModifier);
                expect(v.term.selectedText() === "COPYWORD", "selected " + v.term.selectedText());
                key(Qt.Key_C, Qt.ControlModifier | Qt.ShiftModifier, "");
                key(Qt.Key_V, Qt.ControlModifier | Qt.ShiftModifier, "");
            }
        },
        {
            "name": "copy keys pasted",
            "fn": () => {
                const t = held.clip;
                if (t.session.ext.history.split("COPYWORD").length < 3)
                    return false;
                expect(!has(t, "NOT_COPIED"), "Ctrl+Shift+C did not copy");
            }
        },
        {
            "name": "copy on select",
            "fn": () => {
                const t = held.clip, v = view(t);
                host.copyText("NOT_SELECTED");
                v.term.copyOnSelect = true;
                const row = v.term.height / v.term.lines;
                const p = v.term.mapToItem(null, 4, row * 1.5);
                Probe.click(shell, p.x, p.y, Qt.LeftButton, 2, Qt.NoModifier);
                expect(v.term.selectedText() === "OKLINK", "selected " + v.term.selectedText());
                v.term.copyOnSelect = false;
                action("edit.paste");
            }
        },
        {
            "name": "copy on select pasted",
            "fn": () => {
                const t = held.clip;
                if (t.session.ext.history.split("OKLINK").length < 3)
                    return false;
                expect(!has(t, "NOT_SELECTED"), "the selection did not reach the clipboard");
            }
        },
        // OSC 52: a program (vim, tmux over ssh) sets the clipboard, unless the setting is off.
        {
            "name": "osc 52 write",
            "fn": () => {
                const ext = held.clip.session.ext;
                expect(ext.clipboardWriteAllowed, "OSC 52 is off by default");
                ext.feedBytes("\x1b]52;c;T1NDNTJURVhU\x07"); // OSC52TEXT
                action("edit.paste");
            }
        },
        {
            "name": "osc 52 pasted",
            "fn": () => {
                const t = held.clip;
                if (!has(t, "OSC52TEXT"))
                    return false;
                t.session.ext.clipboardWriteAllowed = false;
                t.session.ext.feedBytes("\x1b]52;c;T1NDNTJCTE9DS0VE\x07"); // OSC52BLOCKED
                action("edit.paste");
            }
        },
        {
            "name": "osc 52 refused",
            "fn": () => {
                const t = held.clip;
                if (t.session.ext.history.split("OSC52TEXT").length < 3)
                    return false;
                expect(!has(t, "OSC52BLOCKED"), "OSC 52 set the clipboard with the setting off");
                t.session.ext.clipboardWriteAllowed = true;
            }
        },
        // A triple click selects the whole line (the second row: OKLINK BADLINK).
        {
            "name": "triple click",
            "fn": () => {
                const v = view(held.clip);
                const row = v.term.height / v.term.lines;
                const p = v.term.mapToItem(null, 4, row * 1.5);
                Probe.click(shell, p.x, p.y, Qt.LeftButton, 3, Qt.NoModifier);
                const text = v.term.selectedText().trim();
                expect(text.startsWith("OKLINK") && text.endsWith("BADLINK"), "a triple click selected " + JSON.stringify(text));
            }
        },
        // A single click clears the selection and selects nothing itself (not even its own cell),
        // once the double-click interval is over (a click inside it would be a fourth click).
        {
            "name": "click clears the selection",
            "fn": () => {
                const v = view(held.clip);
                if (!held.clickAt) {
                    held.clickAt = Date.now();
                    return false;
                }
                if (Date.now() - held.clickAt < Qt.styleHints.mouseDoubleClickInterval + 100)
                    return false;
                // On text ("COPYWORD"): the old click selected the letter under it.
                const p = v.term.mapToItem(null, 4, 4);
                Probe.click(shell, p.x, p.y, Qt.LeftButton, 1, Qt.NoModifier);
                expect(v.term.selectedText() === "", "a click left a selection: " + JSON.stringify(v.term.selectedText()));
            }
        },
        // Ctrl+click opens an http(s) link and refuses javascript:.
        {
            "name": "links",
            "fn": () => {
                const t = held.clip, v = view(t);
                const opened = [];
                v.openExternally = url => opened.push(url);
                const row = v.term.height / v.term.lines, y = row * 1.5;
                const at = {};
                for (let x = 1; x < v.term.width; x += 2) {
                    const url = v.term.hotspotUrlAt(x, y);
                    if (url !== "" && at[url] === undefined)
                        at[url] = x + 2;
                }
                const ok = "https://example.test/ok", bad = "javascript:alert(1)";
                expect(at[ok] !== undefined && at[bad] !== undefined, "links on the row " + JSON.stringify(at));
                for (const url of [ok, bad]) {
                    const p = v.term.mapToItem(null, at[url], y);
                    Probe.click(shell, p.x, p.y, Qt.LeftButton, 1, Qt.ControlModifier);
                }
                expect(JSON.stringify(opened) === JSON.stringify([ok]), "opened " + JSON.stringify(opened));
                v.openExternally = url => Qt.openUrlExternally(url);
            }
        },
        // With the option on, a right-click without a selection pastes the clipboard.
        {
            "name": "right click",
            "fn": () => {
                held.rc = open("rc", "printf 'RC_READY\\n'; exec cat");
            }
        },
        {
            "name": "right click paste",
            "fn": () => {
                const t = held.rc, v = view(t);
                if (!v || !v.term || !has(t, "RC_READY"))
                    return false;
                focusTab(t);
                v.term.pasteOnRightClick = true;
                host.copyText("RIGHTPASTE");
                const p = v.term.mapToItem(null, 40, v.term.height / 2);
                Probe.click(shell, p.x, p.y, Qt.RightButton, 1, Qt.NoModifier);
                expect(!v.menu.visible, "the menu opened instead of pasting");
            }
        },
        {
            "name": "right click pasted",
            "fn": () => has(held.rc, "RIGHTPASTE")
        },
        // Bracketed paste: a program that asks for it gets the markers, unless it is turned off.
        {
            "name": "bracketed",
            "fn": () => {
                held.bp = open("bp", "printf '\\033[?2004hBP_READY\\n'; exec cat");
            }
        },
        {
            "name": "bracketed paste",
            "fn": () => {
                const t = held.bp, v = view(t);
                if (!v || !v.term || !has(t, "BP_READY"))
                    return false;
                focusTab(t);
                host.copyText("BRACKETED");
                action("edit.paste");
            }
        },
        {
            "name": "bracketed pasted",
            "fn": () => {
                const t = held.bp, v = view(t);
                if (!has(t, "BRACKETED"))
                    return false;
                expect(has(t, "[200~BRACKETED") && has(t, "BRACKETED^[[201~"), "markers missing: " + t.session.ext.history.slice(-80));
                v.term.bracketedPasteDisabled = true;
                host.copyText("PLAINPASTE");
                action("edit.paste");
            }
        },
        {
            "name": "bracketed off",
            "fn": () => {
                const t = held.bp, v = view(t);
                if (!has(t, "PLAINPASTE"))
                    return false;
                expect(!has(t, "[200~PLAINPASTE"), "markers with bracketed paste off");
                v.term.bracketedPasteDisabled = false;
            }
        },
        // The automatic log starts for a remote session only, not for a local shell.
        {
            "name": "auto log on",
            "fn": () => {
                if (!shell.preferencesDialog)
                    shell.openPreferences();
                const pc = shell.preferencesDialog.controller;
                const d = JSON.parse(pc.load());
                d.session_log_auto = true;
                d.session_log_template = stem + "auto-{label}.log";
                pc.apply(JSON.stringify(d));
                held.autoLocal = open("autolocal", "printf 'AL_READY\\n'; exec cat");
                held.autoRemote = open("autoremote", "printf 'AR_READY\\n'; exec cat", {
                    "name": "autoremote",
                    "host": "ah"
                });
            }
        },
        {
            "name": "auto log",
            "fn": () => {
                const l = held.autoLocal, r = held.autoRemote;
                if (!has(l, "AL_READY") || !has(r, "AR_READY") || !r.logActive)
                    return false;
                expect(!l.logActive, "a local shell logs itself");
                expect(r.toolTip.indexOf(stem + "auto-autoremote") >= 0, "log tip " + r.toolTip);
                r.session.core.stopLog();
                const pc = shell.preferencesDialog.controller;
                const d = JSON.parse(pc.load());
                d.session_log_auto = false;
                pc.apply(JSON.stringify(d));
            }
        },
        // Settings → Apply reaches the open terminals (font, clipboard options).
        {
            "name": "settings",
            "fn": () => {
                if (!shell.preferencesDialog)
                    shell.openPreferences();
                const pc = shell.preferencesDialog.controller;
                const d = JSON.parse(pc.load());
                d.font_size = 15;
                d.copy_on_select = true;
                d.blinking_cursor = true;
                pc.apply(JSON.stringify(d));
                shell.controller.persist();
            }
        },
        {
            "name": "settings applied",
            "fn": () => {
                const v = view(held.one);
                expect(v.term.font.pointSize === 15 && held.one.session.fontSize === 15, "font " + v.term.font.pointSize);
                expect(v.term.copyOnSelect && v.term.blinkingCursor, "options");
                focusTab(held.one);
                action("view.zoom_in");
                expect(v.term.font.pointSize === 16, "zoomed " + v.term.font.pointSize);
                area.split(false);
            }
        },
        // The zoom belongs to the tab, not to its view: a split, a drag to another pane, a new
        // place in the tab bar and Duplicate make a new view at the same size.
        {
            "name": "zoom kept by split",
            "fn": () => {
                const v = view(held.one);
                if (area.paneCount !== 2 || !v || !v.term)
                    return false;
                expect(v.term.font.pointSize === 16, "after split " + v.term.font.pointSize);
                held.splitPane = area.viewOf(held.one).paneId;
                const other = Object.keys(area.paneViews).map(id => area.paneViews[id]).filter(pv => pv.paneId !== held.splitPane)[0];
                area.syncSizes();
                expect(area.controller.moveTab(held.one.tabId, false, other.paneId, "center", 0, 0), "drag to the other pane");
                area.sync();
            }
        },
        {
            "name": "zoom kept by drag",
            "fn": () => {
                const pv = area.viewOf(held.one);
                const v = view(held.one);
                if (!pv || pv.paneId === held.splitPane || !v || !v.term)
                    return false;
                expect(v.term.font.pointSize === 16, "after drag " + v.term.font.pointSize);
                expect(pv.tabIds.length >= 2 && pv.tabIds[0] === held.one.tabId, "dragged to the front " + pv.tabIds);
                area.controller.moveTab(held.one.tabId, false, pv.paneId, "center", -1, 0);
                area.sync();
            }
        },
        {
            "name": "zoom kept by reorder",
            "fn": () => {
                const pv = area.viewOf(held.one);
                const v = view(held.one);
                if (pv.tabIds[pv.tabIds.length - 1] !== held.one.tabId || !v || !v.term)
                    return false;
                expect(v.term.font.pointSize === 16, "after reorder " + v.term.font.pointSize);
                held.dup = area.duplicateTab(held.one);
            }
        },
        {
            "name": "zoom kept by duplicate",
            "fn": () => {
                const v = view(held.dup);
                if (!v || !v.term || !has(held.dup, "READY_ONE"))
                    return false;
                expect(v.term.font.pointSize === 16 && held.one.session.fontSize === 15, "duplicate " + v.term.font.pointSize);
                area.closeTab(held.dup, false, true);
                focusTab(held.one);
                action("view.zoom_reset");
                expect(view(held.one).term.font.pointSize === 15, "reset " + view(held.one).term.font.pointSize);
                if (area.paneCount < 2)
                    area.split(false);
                held.hostTab = open("zoomhost", "printf 'ZH_READY\\n'; exec cat", {
                    "name": "zoomhost",
                    "host": "zh",
                    "font_size": 13
                });
            }
        },
        // A Hosts entry with its own font size: zoomed over that size, kept in a detached window.
        {
            "name": "host zoom",
            "fn": () => {
                const t = held.hostTab, v = view(t);
                if (!v || !v.term || !has(t, "ZH_READY"))
                    return false;
                expect(t.session.fontSize === 13 && v.term.font.pointSize === 13, "entry font " + v.term.font.pointSize);
                focusTab(t);
                action("view.zoom_in");
                area.detachTab(t);
            }
        },
        {
            "name": "host zoom detached",
            "fn": () => {
                const t = held.hostTab, v = view(t);
                if (area.tabLoc[t.tabId].window === undefined || !v || !v.term)
                    return false;
                expect(v.term.font.pointSize === 14 && t.session.fontSize === 13, "detached " + v.term.font.pointSize);
                area.closeTab(t, false, true);
            }
        },
        // Terminal features on the Ghostty engine through the app's own view: search (plain / regex / case / invalid),
        // the compiled-in colour schemes, historySize, DECSCUSR and clear().
        {
            "name": "features",
            "fn": () => {
                const t = held.one;
                const v = view(t);
                // The view the split just made: laid out first (before, its grid is one column wide).
                if (!v || !v.term || v.term.width < 100)
                    return false;
                const ext = t.session.ext;
                for (const lines of [1000, -1, 150000]) {
                    ext.historySize = lines;
                    expect(ext.historySize === lines, "historySize " + lines + " read back as " + ext.historySize);
                }
                const schemes = v.term.availableColorSchemes;
                expect(schemes.indexOf("DarkPastels") >= 0 && schemes.indexOf("Linux") >= 0, "schemes " + schemes);
                expect(v.term.findNext("ready_one", false, false, true, false), "case-insensitive search");
                expect(v.term.selectedText() === "READY_ONE", "selection " + v.term.selectedText());
                expect(v.term.findNext("READY_.+", true, true, true, false), "regular expression search");
                expect(!v.term.findNext("ready_one", true, false, true, false), "case-sensitive search matched the wrong case");
                expect(!v.term.findNext("(", false, true, true, false), "an invalid regular expression matched");
                ext.feedBytes("\x1b[5 q"); // DECSCUSR 5: blinking bar
                const bar = JSON.parse(ext.snapshotJson()).cursor;
                expect(bar.blink && bar.shape === 1, "DECSCUSR 5 → " + JSON.stringify(bar));
                ext.feedBytes("\x1b[2 q"); // DECSCUSR 2: steady block
                const block = JSON.parse(ext.snapshotJson()).cursor;
                expect(!block.blink && block.shape === 0, "DECSCUSR 2 → " + JSON.stringify(block));
                ext.feedBytes("\x1b[0 q");
                ext.clear();
                expect(!has(t, "READY_ONE"), "clear() kept the scrollback");
            }
        },
        // A terminal follows its pane when the splitter moves and when the window resizes.
        {
            "name": "split resize",
            "fn": () => {
                const pv = area.viewOf(held.one);
                const v = view(held.one);
                if (area.paneCount !== 2 || !v.term || v.term.width < 1)
                    return false;
                const fits = () => v.term.width <= pv.width && v.term.x + v.term.width <= v.width;
                if (check.w0 === 0)
                    check.w0 = v.term.width;
                const w0 = check.w0;
                if (!check.grown) {
                    check.grown = true;
                    shell.width += 200;
                }
                if (v.term.width <= w0)
                    return false;
                expect(fits(), "terminal wider than its pane after the splitter moved");
                expect(v.term.x + v.term.width <= v.width, "terminal wider than its view");
                console.info("U4_DONE");
                shell.requestQuit();
            }
        }
    ]

    Connections {
        target: check.area

        function onTabReopened(oldTab, newTab) {
            check.reopened += 1;
            check.lastNew = newTab;
        }
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
                    check.fail("timed out waiting");
                return;
            }
            check.waited = 0;
            ++check.step;
        }
    }
}
