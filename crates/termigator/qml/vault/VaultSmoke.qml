import QtQuick
import QtQuick.Window
import Termigator.Terminal.Probe

// Offscreen check (tests/vault_smoke.rs). First run: the master password (set, change), the
// unlock hook that PanesArea and the tab restore use, opening a connection with a sealed password,
// the Credentials dialog and the sealing on save, the SSH keys window (list, copy, agent, generate,
// install); then quits through the close path (the snapshot has two connection tabs). With
// --u7-restore (second run on the saved config, vault locked): the restore asks for the master
// password, Cancel opens the tab without the password and the next tab asks again; then Remove.
// Passwords are never printed: failures compare, they do not quote.
Item {
    id: check

    required property var shell
    readonly property bool restoreRun: Qt.application.arguments.indexOf("--u7-restore") >= 0
    readonly property var host: shell.vaultHost
    readonly property var backend: host.backend
    property var area: null
    property var panel: null
    property var seen: null
    property var started: ({})
    property int step: 0
    property int waits: 0
    property bool failed: false
    property int retried: 0
    property int cancelled: 0
    property var commands: []

    function fail(reason) {
        failed = true;
        console.error("termigator vault smoke: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok && !failed)
            fail(what);
    }

    function argument(prefix) {
        const a = Qt.application.arguments.find(s => s.startsWith(prefix));
        return a ? a.slice(prefix.length) : "";
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

    function press(window, key, modifiers, text) {
        Probe.key(window, key, modifiers, text || "");
    }

    // Real key presses into `field` of `window`.
    function typeInto(window, field, text) {
        field.forceActiveFocus();
        for (const ch of text)
            press(window, ch.toUpperCase().charCodeAt(0), Qt.NoModifier, ch);
    }

    // The user typed `text` into a field (after a message box the offscreen window has no active
    // focus item, so the key presses of typeInto would go nowhere).
    function edit(field, text) {
        field.text = text;
        field.textEdited();
    }

    function enter(window) {
        press(window, Qt.Key_Return, Qt.NoModifier, "\r");
    }

    function isLive(dlg) {
        return dlg !== null && dlg !== undefined && dlg.visible;
    }

    // A password prompt that is not the one we saw before (the next question), or false.
    function nextPrompt() {
        const p = host.lastPrompt;
        if (p === seen || !isLive(p))
            return false;
        seen = p;
        return p;
    }

    function connectionJson(i) {
        return panel.hosts.entryJson("connection", i);
    }

    function status() {
        return JSON.parse(backend.statusJson());
    }

    function credNames() {
        return JSON.parse(backend.credentialsJson()).map(c => c.name).join(",");
    }

    readonly property var firstRun: [
        // The hooks are set; the config has plain passwords and no vault.
        () => {
            area = find("panesArea", shell.contentItem);
            panel = find("hostsPanel", shell.contentItem);
            expect(area !== null && panel !== null, "no panes area or host list");
            expect(typeof shell.unlockVault === "function" && typeof area.unlockVault === "function", "unlock hooks not set");
            for (const id of ["tools.master_password", "tools.credentials", "tools.ssh_keys"])
                expect(shell.shellActions.actions[id].enabled, id + " not handled");
            expect(status().text === "⚠ 2 saved password(s) stored in plain text in session.yaml.", "status " + status().text);
            expect(!backend.needsUnlock(), "locked without a vault");
            shell.shellActions.actions["tools.master_password"].trigger();
        },
        // Set master password: the two entries must match (real keys).
        () => {
            const m = host.masterDialog;
            expect(isLive(m), "master dialog not shown");
            expect(m.status.text === status().text && m.setButton.enabled && !m.changeButton.enabled && !m.removeButton.enabled, "buttons");
            m.setButton.click();
        },
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.title === "Set master password" && p.confirm, "set prompt " + p.title);
            typeInto(p, p.field, "m1");
            typeInto(p, p.repeat, "x");
            enter(p);
        },
        () => {
            const p = seen;
            expect(isLive(p) && p.problem.text === "The passwords do not match.", "mismatch not reported");
            p.repeat.text = "";
            typeInto(p, p.repeat, "m1");
            enter(p);
        },
        () => {
            if (!host.masterDialog.done.visible)
                return false;
            const m = host.masterDialog;
            expect(m.done.text === "Master password set — 2 saved password(s) encrypted.", "done " + m.done.text);
            expect(m.status.text === "✔ Master password is set — 2 saved password(s) encrypted.", "status " + m.status.text);
            expect(!m.setButton.enabled && m.changeButton.enabled && m.removeButton.enabled, "buttons after set");
            expect(!backend.needsUnlock(), "locked right after Set");
            expect(backend.isSealed(JSON.parse(backend.credentialsJson())[0].password), "credential not sealed");
            m.changeButton.click();
        },
        // Change: the current master first (wrong, then right), then the new one.
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.title === "Unlock saved passwords" && p.prompt === "Enter the current master password.\n\nMaster password:", "unlock prompt " + p.prompt);
            typeInto(p, p.field, "zz");
            enter(p);
        },
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.error === "Wrong master password — try again.", "wrong password not reported: " + p.error);
            typeInto(p, p.field, "m1");
            enter(p);
        },
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.title === "Change master password", "change prompt " + p.title);
            typeInto(p, p.field, "m2");
            typeInto(p, p.repeat, "m2");
            enter(p);
        },
        () => {
            const m = host.masterDialog;
            if (m.done.text !== "Master password changed — 2 password(s) re-encrypted.")
                return false;
            expect(m.status.text === "✔ Master password is set — 2 saved password(s) encrypted.", "status after change " + m.status.text);
            m.close();
            backend.forgetKey();
            expect(backend.needsUnlock(), "not locked after forgetting the key");
            shell.unlockVault(() => ++check.retried, () => ++check.cancelled);
        },
        // The hook PanesArea uses: Cancel → cancelled(); then the right master → retry().
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.prompt === "This connection uses a saved password.\n\nMaster password:", "hook prompt " + p.prompt);
            typeInto(p, p.field, "m2");
            press(p, Qt.Key_Escape, Qt.NoModifier, "");
        },
        () => {
            expect(cancelled === 1 && retried === 0 && backend.needsUnlock(), "Cancel: " + cancelled + "/" + retried);
            shell.unlockVault(() => ++check.retried, () => ++check.cancelled);
        },
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            typeInto(p, p.field, "m2");
            enter(p);
        },
        () => {
            if (retried !== 1)
                return false;
            expect(!backend.needsUnlock() && cancelled === 1, "unlock hook");
            // A connection with a sealed password asks through PanesArea, then opens its tab.
            backend.forgetKey();
            shell.openConnection(connectionJson(0));
        },
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.title === "Unlock saved passwords", "connect prompt");
            expect(!area.messageBox.visible, "the 'Cannot decrypt' warning instead of the prompt");
            typeInto(p, p.field, "m2");
            enter(p);
        },
        () => {
            if (area.tabList.length !== 1)
                return false;
            expect(!area.messageBox.visible, "warning after unlock: " + area.messageBox.text);
            expect(area.tabList[0].extraEnv !== null && !!area.tabList[0].extraEnv.SSHPASS, "no password in the tab's environment");
            shell.openConnection(connectionJson(0)); // unlocked: no question
        },
        () => {
            expect(area.tabList.length === 2 && nextPrompt() === false, "second tab asked again");
            // Credentials, locked: a new typed password makes the save ask (sealNeeded).
            backend.forgetKey();
            shell.shellActions.actions["tools.credentials"].trigger();
        },
        () => {
            const d = host.credentialsDialog;
            expect(isLive(d) && d.names.count === 1 && d.nameField.text === "ops" && d.userField.text === "root", "credential list");
            expect(d.passwordField.text === "" && d.passwordField.placeholderText === "saved (encrypted) — leave empty to keep" && d.forgetBox.visible, "sealed password shown");
            expect(d.usedLabel.text === "Used by 1 entry.", "used " + d.usedLabel.text);
            // Rename with real keys; an empty name is refused.
            d.nameField.forceActiveFocus();
            d.nameField.selectAll();
            press(d, Qt.Key_Backspace, Qt.NoModifier, "");
            d.accept();
        },
        () => {
            const d = host.credentialsDialog;
            expect(isLive(d) && host.lastMessage === "Every credential needs a name.", "empty name accepted");
            host.messageWindow.close();
        },
        () => {
            const d = host.credentialsDialog;
            if (host.messageWindow !== null)
                return false;
            edit(d.nameField, "prod");
            d.addButton.click();
        },
        () => {
            const d = host.credentialsDialog;
            expect(d.names.count === 2 && d.nameField.text === "credential1" && d.list.currentIndex === 1, "added");
            d.nameField.text = "prod";
            d.nameField.textEdited();
            d.accept();
        },
        () => {
            const d = host.credentialsDialog;
            expect(isLive(d) && host.lastMessage === "Credential names must be unique.", "duplicate name accepted");
            host.messageWindow.close();
        },
        () => {
            const d = host.credentialsDialog;
            if (host.messageWindow !== null)
                return false;
            edit(d.nameField, "late");
            edit(d.userField, "bob");
            edit(d.passwordField, "pw3");
            d.accept();
        },
        // The save finds the vault locked: the file gets no plain text, the host asks, saves again.
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.prompt === "Unlock to save the new password encrypted.\n\nMaster password:", "seal prompt " + p.prompt);
            typeInto(p, p.field, "m2");
            enter(p);
        },
        () => {
            if (backend.needsUnlock())
                return false;
            expect(credNames() === "prod,late", "credentials " + credNames());
            const late = JSON.parse(backend.credentialsJson())[1];
            expect(late.user === "bob" && backend.isSealed(late.password), "new password not sealed");
            expect(backend.credentialUsage("prod") === 1, "references not followed the rename");
            // The same, answered with Cancel: the user is told, nothing plain reaches the disk.
            backend.forgetKey();
            shell.shellActions.actions["tools.credentials"].trigger();
        },
        () => {
            const d = host.credentialsDialog;
            d.addButton.click();
            edit(d.nameField, "late2");
            edit(d.passwordField, "pw4");
            d.accept();
        },
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            press(p, Qt.Key_Escape, Qt.NoModifier, "");
        },
        () => {
            if (host.messageWindow === null)
                return false;
            expect(host.lastMessage.startsWith("The new password was not saved to disk — saved passwords are encrypted and the master password was not entered.\n"), "cancel message " + host.lastMessage);
            host.messageWindow.close();
            expect(!host.sealing, "still sealing");
            // SSH keys: the directory is the test's own.
            host.backend.sshDir = argument("--u7-ssh-dir=");
            shell.openInner = (inner, title, mode, env) => check.commands.push({
                    "inner": inner,
                    "title": title,
                    "mode": mode,
                    "env": env
                });
            shell.shellActions.actions["tools.ssh_keys"].trigger();
        },
        () => {
            const k = host.sshKeys;
            if (!isLive(k) || k.rows.count < 1)
                return false;
            const row = k.rows.get(0);
            expect(k.rows.count === 1 && row.name === "id_ed25519" && row.type === "ED25519 256", "key row " + row.name + " " + row.type);
            expect(row.fingerprint.startsWith("SHA256:") && row.comment === "u7@test", "key details");
            expect(!k.copyButton.enabled && !k.agentButton.enabled && !k.installButton.enabled, "buttons without a selection");
            k.table.currentIndex = 0;
            expect(k.copyButton.enabled && k.agentButton.enabled && k.installButton.enabled, "buttons with a selection");
            k.copyButton.click();
            expect(k.message.text.startsWith("Public key id_ed25519.pub copied — paste it into authorized_keys"), "copy message " + k.message.text);
            k.agentButton.click();
        },
        () => {
            const c = commands[0];
            expect(commands.length === 1 && c.title === "ssh-add: id_ed25519" && c.inner.startsWith("ssh-add ") && c.inner.endsWith("/id_ed25519") && c.mode === "embedded", "ssh-add command " + JSON.stringify(commands));
            host.sshKeys.generateButton.click();
        },
        // Generate: an existing name is refused, a free one runs ssh-keygen in a tab.
        () => {
            const g = host.lastGenerate;
            const k = host.sshKeys;
            expect(g !== null && g.visible, "generate dialog");
            expect(g.pathField.text.endsWith("/id_ed25519_2") && g.commentField.text.indexOf("@") > 0, "defaults " + g.pathField.text);
            g.typeCombo.currentIndex = 1;
            g.typeCombo.activated(1);
            expect(g.pathField.text.endsWith("/id_rsa"), "path for rsa " + g.pathField.text);
            g.pathField.text = k.keys[0].path;
            enter(g);
        },
        () => {
            const g = host.lastGenerate;
            expect(g.visible && g.errorMessage.text.endsWith("already exists — pick another name (an existing key is never overwritten)."), "overwrite allowed: " + g.errorMessage.text);
            g.pathField.text = host.sshKeys.keys[0].path.replace("id_ed25519", "id_new");
            enter(g);
        },
        () => {
            const c = commands[1];
            expect(commands.length === 2 && c.title === "ssh-keygen: id_new" && c.inner.startsWith("ssh-keygen -t rsa -b 4096 -f "), "keygen command " + JSON.stringify(commands[1]));
            host.sshKeys.installButton.click();
        },
        // Install on a host: the saved password goes through the environment, not the command.
        () => {
            const c = host.lastChoice;
            expect(c !== null && c.visible && c.label === "Append id_ed25519.pub to authorized_keys on:", "choice dialog");
            expect(c.choices.join(",") === "web,db", "choices " + c.choices);
            c.combo.currentIndex = 1;
            c.accept();
        },
        // The credential's password is sealed and the vault is locked again: the unlock comes first.
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.prompt === "This connection uses a saved password.\n\nMaster password:", "install prompt " + p.prompt);
            typeInto(p, p.field, "m2");
            enter(p);
        },
        () => {
            if (commands.length < 3 && host.messageWindow === null)
                return false;
            if (commands.length < 3) {
                // No ssh-copy-id / sshpass here: the reason is shown instead.
                expect(host.lastMessage.indexOf("is not installed") >= 0 || host.lastMessage.indexOf("sshpass is needed") >= 0, "install error " + host.lastMessage);
                host.messageWindow.close();
            } else {
                const c = commands[2];
                expect(c.title === "ssh-copy-id: db" && c.inner.startsWith("sshpass -e ssh-copy-id -i ") && c.inner.endsWith("root@127.0.0.1") && c.inner.indexOf("pw") < 0, "install command " + c.inner);
                expect(c.env && JSON.parse(c.env).SSHPASS !== undefined, "no SSHPASS in the environment");
            }
            host.sshKeys.requestActivate();
        },
        () => {
            const grabbed = host.sshKeys.contentItem.grabToImage(result => {
                if (!result.saveToFile(shell.controller.smokeTestShot))
                    check.fail("cannot save the grab");
                console.info("termigator vault smoke: first run done");
                shell.requestQuit();
            });
            expect(grabbed, "grab not started");
        }
    ]

    readonly property var secondRun: [
        // The restore asks for the master: Cancel opens the first tab without the password.
        () => {
            area = find("panesArea", shell.contentItem);
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.title === "Unlock saved passwords" && p.prompt === "This connection uses a saved password.\n\nMaster password:", "restore prompt " + p.prompt);
            expect(area.tabList.length === 0, "tabs before the answer");
            press(p, Qt.Key_Escape, Qt.NoModifier, "");
        },
        // ... and the next tab asks again; the right master gives it the password.
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(area.tabList.length === 1 && area.tabList[0].extraEnv === null, "first tab after Cancel");
            typeInto(p, p.field, "m2");
            enter(p);
        },
        () => {
            if (area.tabList.length !== 2)
                return false;
            expect(area.tabList[0].title === "web" && area.tabList[1].title === "web", "restored tabs");
            expect(area.tabList[1].extraEnv !== null && !!area.tabList[1].extraEnv.SSHPASS, "second tab without the password");
            expect(!backend.needsUnlock() && !area.messageBox.visible, "state after restore");
            shell.shellActions.actions["tools.master_password"].trigger();
        },
        // Remove: Yes, the current master again, then plain text.
        () => {
            const m = host.masterDialog;
            expect(isLive(m) && m.changeButton.enabled, "master dialog");
            m.removeButton.click();
        },
        () => {
            const q = host.messageWindow;
            expect(q !== null && q.question && q.text === "Saved passwords will be decrypted and stored in plain text in session.yaml again.\nContinue?", "remove question");
            // No keeps the vault; Yes goes on.
            q.reject();
        },
        () => {
            expect(host.messageWindow === null && host.masterDialog.removeButton.enabled && !backend.needsUnlock(), "No removed it");
            host.masterDialog.removeButton.click();
        },
        () => {
            host.messageWindow.accept();
        },
        () => {
            const p = nextPrompt();
            if (!p)
                return false;
            expect(p.prompt === "Enter the current master password.\n\nMaster password:", "remove prompt");
            typeInto(p, p.field, "m2");
            enter(p);
        },
        () => {
            const m = host.masterDialog;
            if (m.done.text !== "Master password removed — saved passwords are plain text again.")
                return false;
            expect(m.status.text.startsWith("⚠ 3 saved password(s) stored in plain text"), "status " + m.status.text);
            expect(m.setButton.enabled && !m.changeButton.enabled && !m.removeButton.enabled && !backend.needsUnlock(), "after remove");
            const grabbed = m.contentItem.grabToImage(result => {
                if (!result.saveToFile(shell.controller.smokeTestShot))
                    check.fail("cannot save the grab");
                console.info("termigator vault smoke: restore run done");
                shell.requestQuit();
            });
            expect(grabbed, "grab not started");
        }
    ]

    Timer {
        interval: 200
        repeat: true
        running: !check.failed && check.step < (check.restoreRun ? check.secondRun : check.firstRun).length
        onTriggered: {
            const steps = check.restoreRun ? check.secondRun : check.firstRun;
            if (steps[check.step]() === false) {
                if (++check.waits > 100)
                    check.fail("timed out waiting");
                return;
            }
            check.waits = 0;
            ++check.step;
        }
    }
}
