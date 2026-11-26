import QtQuick
import Termigator.Terminal.Probe

// Offscreen check (tests/editors_smoke.rs, --smoke-check=editors/EditorsSmoke.qml): opens each
// editor through shell.openEditor, fills it with real key presses where it matters, accepts and
// compares the entry handed to onAccepted. Quits with 0, or with 1 after a console.error.
Item {
    id: check

    required property var shell
    property int step: 0
    property int waited: 0
    property bool lateAnswer: false
    property var dlg: null
    property var got: null
    property var quick: null
    property var pending: null

    // The EditorHost instance of ShellWindow (found by its API; ShellWindow keeps no alias for it).
    readonly property var host: all(shell.contentItem, i => "lastTest" in i && "messageWindow" in i)[0]

    function fail(reason) {
        console.error("termigator editors smoke: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok)
            fail(what);
    }

    function same(actual, wanted, what) {
        expect(JSON.stringify(actual) === JSON.stringify(wanted), what + ": got " + JSON.stringify(actual) + ", wanted " + JSON.stringify(wanted));
    }

    // All items under `item` that satisfy `pred`.
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

    function fields(win) {
        return all(win.contentItem, i => "placeholderText" in i && "echoMode" in i);
    }

    function passwordField(win) {
        return all(win.contentItem, i => "echoMode" in i && i.echoMode === TextInput.Password)[0];
    }

    function button(win, text) {
        return all(win.contentItem, i => "icon" in i && "clicked" in i && "text" in i && i.text.replace("&", "") === text)[0];
    }

    function check_(win, start) {
        return all(win.contentItem, i => "checked" in i && "tristate" in i && String(i.text).replace("&", "").startsWith(start))[0];
    }

    function combos(win) {
        return all(win.contentItem, i => "currentIndex" in i && "model" in i && "popup" in i);
    }

    function warnings(win) {
        return all(win.contentItem, i => "type" in i && "showCloseButton" in i && i.visible).map(i => i.text);
    }

    function press(win, key, modifiers, text) {
        Probe.key(win, key, modifiers || Qt.NoModifier, text || "");
    }

    function typeText(win, text) {
        for (const ch of text)
            press(win, ch.toUpperCase().charCodeAt(0), Qt.NoModifier, ch);
    }

    // The editor offers "Test connection" before saving, like SSH.
    function testButton(kind) {
        const b = button(dlg, "Test connection");
        expect(dlg.testable && b && b.visible && b.enabled, kind + " has no usable Test connection button");
    }

    function open(kind, entry, group) {
        got = null;
        dlg = shell.openEditor(kind, entry, group || "", json => got = JSON.parse(json));
        dlg.requestActivate();
        return dlg;
    }

    // Saves the dialog next to the shell's grab (to look at it); a step returns this to wait for it.
    // The window's content item gets its size only once the window is exposed.
    function shot(name) {
        const win = dlg;
        const path = shell.controller.smokeTestShot.replace(/[^/]*$/, name + ".png");
        let state = 0;
        return () => {
            if (state === 0 && win.contentItem.width >= 1) {
                state = 1;
                win.contentItem.grabToImage(r => {
                    if (!r.saveToFile(path))
                        check.fail("cannot save " + path);
                    state = 2;
                });
            }
            return state === 2;
        };
    }

    readonly property var steps: [
        // SSH, new: typed with the keyboard from the first field, Enter accepts.
        () => {
            open("ssh", "", "prod");
            expect(dlg.title === "Add SSH connection", "title " + dlg.title);
            same(dlg.collect().group, "prod", "default folder");
            expect(dlg.testable, "SSH has a Test connection button");
        },
        () => {
            typeText(dlg, "web");
            press(dlg, Qt.Key_Tab);
            typeText(dlg, "10.0.0.5");
            press(dlg, Qt.Key_Tab);
            press(dlg, Qt.Key_Tab); // the credential combo (the config has one)
            typeText(dlg, "root");
            press(dlg, Qt.Key_Tab);
            typeText(dlg, "pw");
            same([dlg.collect().label, dlg.collect().host, dlg.collect().user, dlg.collect().password], ["web", "10.0.0.5", "root", "pw"], "typed fields");
            return shot("ssh");
        },
        () => {
            press(dlg, Qt.Key_Return);
        },
        () => {
            expect(got !== null, "Enter did not accept");
            same(Object.keys(got), ["label", "host", "user", "port", "terminal", "group", "password"], "new SSH keys");
            same(got.port, null, "default port");
            same(got.group, "prod", "group");
            const size = shell.controller.dialogSize("ConnectionDialog");
            expect(size.width > 0 && size.height > 0, "dialog size not kept");
        },
        // Esc cancels without answering.
        () => {
            open("ssh", "", "");
            press(dlg, Qt.Key_Escape);
        },
        () => {
            expect(got === null && (!dlg || !dlg.visible), "Esc did not cancel"); // a cancelled editor may already be destroyed
        },
        // The saved password and key can be removed; unknown keys survive.
        () => {
            open("ssh", JSON.stringify({
                "zz": {
                    "a": 1
                },
                "label": "x",
                "host": "h",
                "user": "u",
                "password": "pw",
                "identity_file": "~/.ssh/k"
            }), "");
            expect(dlg.title === "Edit SSH connection", "edit title");
            expect(fields(dlg).some(f => f.text === "~/.ssh/k"), "key shown");
            passwordField(dlg).text = "";
            fields(dlg).find(f => f.text === "~/.ssh/k").text = "";
            dlg.accept();
        },
        () => {
            same(Object.keys(got), ["zz", "label", "host", "user", "port", "terminal"], "cleared SSH keys");
            same(got.zz, {
                "a": 1
            }, "unknown key");
        },
        // The jump host's own key: shown on the Forwarding tab, checked, kept and cleared.
        () => {
            const key = shell.controller.smokeTestShot.replace(/[^/]*\/[^/]*$/, "termigator-u5-editors-keys/jump_key");
            open("ssh", JSON.stringify({
                "label": "x",
                "host": "h",
                "user": "u",
                "proxy_jump": "ops@bastion",
                "jump_identity_file": key
            }), "");
            const jk = all(dlg.contentItem, i => i.objectName === "jumpKey")[0];
            if (jk === undefined) {
                fail("no jump key field");
                return;
            }
            same(jk.text, key, "jump key shown");
            expect(jk.enabled, "jump key field disabled with a jump host");
            expect(jk.locked, "passphrase not recognised");
            // The Forwarding tab keeps its labels beside the fields (wide mode) at the default size.
            dlg.tabBar.currentIndex = 1;
        },
        // A step later: the passphrase message has finished opening.
        () => {
            return shot("ssh-forwarding");
        },
        () => {
            const form = all(dlg.contentItem, i => "wideMode" in i && i.visible)[0];
            expect(form !== undefined && form.wideMode, "Forwarding form in narrow mode at width " + dlg.width);
            // Lower than its form: the tab scrolls instead of cutting the form off.
            dlg.height = 360;
        },
        () => {
            const view = all(dlg.contentItem, i => "contentHeight" in i && "availableHeight" in i && i.visible && all(i, j => j.objectName === "jumpKey").length > 0)[0];
            expect(view !== undefined && view.contentHeight > view.availableHeight, "nothing to scroll");
            return shot("ssh-forwarding-low");
        },
        () => {
            const key = shell.controller.smokeTestShot.replace(/[^/]*\/[^/]*$/, "termigator-u5-editors-keys/jump_key");
            const jk = all(dlg.contentItem, i => i.objectName === "jumpKey")[0];
            const problem = all(jk, i => i.objectName === "keyProblem")[0];
            same(problem.text, "", "jump key warning");
            expect(button(dlg, "Browse…") !== undefined && button(dlg, "From SSH keys…") !== undefined, "jump key buttons");
            jk.text = key + "-missing";
            expect(problem.text.startsWith("SSH key file not found"), "missing key: " + problem.text);
            jk.text = key;
            dlg.accept();
        },
        () => {
            same([got.proxy_jump, got.jump_identity_file !== undefined], ["ops@bastion", true], "jump key saved");
            open("ssh", JSON.stringify({
                "label": "x",
                "host": "h",
                "user": "u",
                "proxy_jump": "ops@bastion",
                "jump_identity_file": "/k/jump"
            }), "");
            all(dlg.contentItem, i => i.objectName === "jumpKey")[0].text = "";
            dlg.accept();
        },
        () => {
            expect(!("jump_identity_file" in got), "cleared jump key kept");
        },
        // An encrypted password is never shown; "Remove the saved password" drops it.
        () => {
            open("ssh", JSON.stringify({
                "label": "x",
                "host": "h",
                "user": "u",
                "password": "enc:v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            }), "");
            expect(passwordField(dlg).text === "", "token in the field");
            expect(passwordField(dlg).placeholderText.startsWith("saved (encrypted)"), "placeholder " + passwordField(dlg).placeholderText);
            const forget = check_(dlg, "Remove the saved");
            expect(forget && forget.visible, "forget box hidden");
            dlg.accept();
        },
        () => {
            expect(got.password.startsWith("enc:v1:"), "sealed password lost");
            open("ssh", JSON.stringify({
                "label": "x",
                "host": "h",
                "user": "u",
                "password": "enc:v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            }), "");
            check_(dlg, "Remove the saved").checked = true;
            dlg.accept();
        },
        () => {
            expect(!("password" in got), "password kept although removed");
        },
        // A credential locks the user and the password.
        () => {
            open("ssh", JSON.stringify({
                "label": "x",
                "host": "h",
                "user": "me",
                "password": "own"
            }), "");
            const cred = combos(dlg).find(c => c.model[1] && c.model[1].value === "ops");
            expect(cred, "credential combo");
            cred.currentIndex = 1;
            const user = fields(dlg).find(f => f.text === "root");
            expect(user && !user.enabled, "user not locked to the credential");
            expect(passwordField(dlg).placeholderText === "from credential “ops”", "password placeholder");
            cred.currentIndex = 0;
            expect(fields(dlg).some(f => f.text === "me" && f.enabled), "own login not back");
            cred.currentIndex = 1;
            dlg.accept();
        },
        () => {
            same([got.credential, got.user, "password" in got], ["ops", "root", false], "credential entry");
        },
        // RDP.
        () => {
            open("rdp", "", "");
            expect(dlg.title === "Add RDP connection", "rdp title");
        },
        () => {
            typeText(dlg, "box");
            press(dlg, Qt.Key_Tab);
            typeText(dlg, "10.0.0.9");
            return shot("rdp");
        },
        () => {
            press(dlg, Qt.Key_Return);
        },
        () => {
            same(Object.keys(got), ["protocol", "label", "host", "port", "terminal"], "new RDP keys");
            same(got.protocol, "rdp", "protocol");
            open("rdp", JSON.stringify({
                "label": "w",
                "host": "h",
                "fullscreen": true,
                "resolution": "800x600",
                "clipboard": false
            }), "");
            expect(check_(dlg, "Full screen").checked && !check_(dlg, "C").checked, "RDP flags");
            testButton("RDP");
            dlg.accept();
        },
        () => {
            same([got.fullscreen, "resolution" in got, got.clipboard], [true, false, false], "RDP full screen");
        },
        // VNC, Telnet (with its warning), Serial (with the access problem), Favorite.
        () => {
            open("vnc", JSON.stringify({
                "label": "v",
                "host": "h",
                "password": "stray",
                "ssh_gateway": "bastion"
            }), "");
            const gw = combos(dlg).find(c => c.model[1] && c.model[1].value === "bastion");
            expect(gw && gw.currentValue === "bastion", "VNC gateway");
            testButton("VNC");
            dlg.accept();
        },
        () => {
            same([got.protocol, "password" in got, got.ssh_gateway], ["vnc", false, "bastion"], "VNC entry");
            open("telnet", JSON.stringify({
                "label": "sw",
                "host": "10.0.0.2",
                "port": 2323,
                "tags": ["x"]
            }), "");
            expect(warnings(dlg).includes("Telnet is not encrypted — use it only on trusted networks."), "telnet warning");
            dlg.accept();
        },
        () => {
            same([got.protocol, got.port, got.tags], ["telnet", 2323, ["x"]], "telnet entry");
            open("serial", JSON.stringify({
                "label": "con",
                "device": "/dev/definitely-not-here",
                "baud": 9600
            }), "");
            expect(!dlg.testable, "serial has no Test button");
            expect(warnings(dlg).length === 1, "serial warning " + JSON.stringify(warnings(dlg)));
            return shot("serial");
        },
        () => {
            dlg.accept();
        },
        () => {
            same([got.protocol, got.device, got.baud], ["serial", "/dev/definitely-not-here", 9600], "serial entry");
            open("favorite", "", "ops");
            press(dlg, Qt.Key_Return);
        },
        () => {
            // The editor stays open with the reason.
            expect(got === null, "empty favorite accepted");
            expect(dlg.visible && dlg.error === "Enter label and command." && !host.messageWindow, "no in-place error for an empty favorite: " + dlg.error);
        },
        () => {
            typeText(dlg, "sh");
            press(dlg, Qt.Key_Tab);
            typeText(dlg, "bash");
            press(dlg, Qt.Key_Return);
        },
        () => {
            same(got, {
                "label": "sh",
                "command": "bash",
                "terminal": "embedded",
                "group": "ops"
            }, "favorite");
        },
        // File → Quick connect…
        () => {
            quick = null;
            shell.quickConnectHandler = (text, mode) => {
                check.quick = [text, mode];
                return "";
            };
            shell.shellActions.actions["file.quick_connect"].trigger();
        },
        () => {
            dlg = host.lastQuick;
            expect(dlg && dlg.visible && dlg.title === "Quick SSH", "no Quick SSH dialog");
            dlg.requestActivate();
            same(dlg.terminal, "embedded", "default terminal");
            typeText(dlg, "me@h.lan:2200");
            press(dlg, Qt.Key_Return);
        },
        () => {
            same(quick, ["me@h.lan:2200", "embedded"], "quick connect call");
        },
        // "Test connection" probes the values in the form without leaving the dialog.
        () => {
            open("ssh", JSON.stringify({
                "label": "closed",
                "host": "127.0.0.1",
                "user": "u",
                "port": 1
            }), "");
            button(dlg, "Test connection").clicked();
        },
        () => {
            const t = host.lastTest;
            expect(t && t.visible, "no test window");
            pending = t;
            return () => !t.running;
        },
        () => {
            expect(pending.headline.startsWith("✘"), "headline " + pending.headline);
            expect(pending.steps.length >= 2 && pending.steps[0].name === "DNS", "steps " + JSON.stringify(pending.steps));
            pending.close();
            dlg.reject();
        },
        // Closing the test window and the editor while the probe runs; its answer arrives
        // later at the shared backend, after both are gone, and nothing breaks.
        () => {
            open("ssh", JSON.stringify({
                "label": "slow",
                "host": "192.0.2.1",
                "user": "u",
                "port": 22
            }), "");
            button(dlg, "Test connection").clicked();
        },
        () => {
            const t = host.lastTest;
            expect(t && t.visible && t.running, "no running test window");
            check.lateAnswer = false;
            t.backend.testFinished.connect(() => check.lateAnswer = true);
            t.close();
            dlg.reject();
            return () => check.lateAnswer;
        },
        () => {
            expect(check.lateAnswer, "the probe never answered");
            expect(!host.lastTest || !host.lastTest.visible, "the test window came back");
        },
        () => {
            open("ssh", "", "");
            button(dlg, "Test connection").clicked();
        },
        () => {
            expect(host.messageWindow.visible && host.messageWindow.text === "Enter a host first.", "no hint without a host");
            host.messageWindow.close();
            dlg.reject();
            return () => !host.messageWindow;
        },
        // A message after the editor it was shown over is gone (it used to crash: the MessageDialog
        // kept a raw pointer to that window).
        () => {
            host.message("Termigator", "after the editor", shell);
            expect(host.messageWindow.visible && host.messageWindow.text === "after the editor", "no message after a destroyed editor");
            host.messageWindow.close();
            return () => !host.messageWindow;
        },
        () => {
            console.info("termigator editors smoke: ok");
            Qt.exit(0);
        }
    ]

    Timer {
        interval: 200
        repeat: true
        running: check.step < check.steps.length
        onTriggered: {
            if (check.waitFor) {
                if (check.waitFor() || ++check.waited > 100)
                    check.waitFor = null;
                else
                    return;
            }
            const next = check.steps[check.step]();
            ++check.step;
            if (typeof next === "function")
                check.waitFor = next;
        }
    }

    property var waitFor: null
}
