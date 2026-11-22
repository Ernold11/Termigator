import QtQuick
import Termigator

// Vault, credentials and SSH keys.
// One instance in ShellWindow. It
//   - sets shell.unlockVault(retry, cancelled): asks for the master password once per run, lazily;
//     PanesArea.unlockVault follows it, and so does the restore of tabs;
//   - handles Tools → Master password…, Credentials… and SSH keys…;
//   - saves again after the user entered the master password for new plain-text passwords
//     (controller.sealNeeded);
//   - offers ensureUnlocked / passwordEnv to the other areas (Files, tunnels, scp).
//
// The slow work (scrypt, ssh-keygen -l) runs in the backend's worker thread: start(begin, done)
// calls begin(token) and done(result) with the parsed answer of the signal for that token.
// Passwords typed here go straight to the backend; they are never kept, logged or shown.
Item {
    id: host

    required property var shell
    readonly property alias backend: vaultBackend
    // The message or question window that is open, else null; the text of the last one.
    property var messageWindow: null
    property string lastMessage: ""
    // The windows opened last (the smoke check follows them).
    property var lastPrompt: null
    property var masterDialog: null
    property var credentialsDialog: null
    property var sshKeys: null
    property var lastGenerate: null
    property var lastChoice: null
    property int sequence: 0
    property var waiting: ({})
    property bool sealing: false

    function openMessage(title, text, parentWindow, question, yes) {
        const dlg = messageComponent.createObject(shell, {
            "host": host,
            "title": title,
            "text": text,
            "question": question,
            "transientParent": parentWindow || shell
        });
        if (yes)
            dlg.accepted.connect(yes);
        lastMessage = text;
        messageWindow = dlg;
        dlg.show();
    }

    // QMessageBox.warning / information.
    function message(title, text, parentWindow) {
        openMessage(title, text, parentWindow, false, null);
    }

    // QMessageBox.question with Yes / No: yes() runs on Yes.
    function ask(title, text, parentWindow, yes) {
        openMessage(title, text, parentWindow, true, yes);
    }

    function start(begin, done) {
        const token = ++sequence;
        waiting[token] = done;
        begin(token);
    }

    function answer(token, json) {
        const done = waiting[token];
        delete waiting[token];
        if (done)
            done(JSON.parse(json));
    }

    // Asks for a password: onEntered(password), onCancelled() on Cancel.
    function askPassword(title, prompt, confirm, parentWindow, onEntered, onCancelled, error) {
        const dlg = passwordComponent.createObject(shell, {
            "host": host,
            "title": title,
            "prompt": prompt,
            "error": error || "",
            "confirm": confirm,
            "transientParent": parentWindow || shell
        });
        dlg.entered.connect(onEntered);
        if (onCancelled)
            dlg.rejected.connect(onCancelled);
        lastPrompt = dlg;
        dlg.show();
        return dlg;
    }

    // Ensures the vault is unlocked: done() when sealed passwords can be used (no vault, the key
    // is in memory, or the right master was just typed), else cancelled() (Cancel; also when the
    // vault itself is unusable, which is reported).
    function ensureUnlocked(reason, done, cancelled, parentWindow, error) {
        if (!vaultBackend.needsUnlock()) {
            done();
            return;
        }
        const prompt = (reason ? reason + "\n\n" : "") + qsTr("Master password:");
        askPassword(qsTr("Unlock saved passwords"), prompt, false, parentWindow, master => {
            start(t => vaultBackend.startUnlock(t, master), r => {
                if (r.ok) {
                    done();
                } else if (r.fatal) {
                    message(qsTr("Termigator"), r.error, parentWindow);
                    if (cancelled)
                        cancelled();
                } else {
                    ensureUnlocked(reason, done, cancelled, parentWindow, r.error);
                }
            });
        }, cancelled, error);
    }

    // The environment (SSHPASS) for a command that needs the entry's saved password, after the
    // unlock; done(env) with env null when there is none or it could not be read (the password is
    // then typed in the terminal).
    function passwordEnv(entryJson, done, parentWindow) {
        const r = JSON.parse(vaultBackend.passwordEnv(entryJson));
        if (r.state === "env") {
            done(r.env);
        } else if (r.state === "locked") {
            ensureUnlocked(qsTr("This connection uses a saved password."), () => passwordEnv(entryJson, done, parentWindow), () => done(null), parentWindow);
        } else {
            if (r.state === "error")
                message(qsTr("Termigator"), r.warning, parentWindow);
            done(null);
        }
    }

    // Runs a command of the SSH keys window: the command in an embedded terminal tab.
    function run(command, env) {
        if (!shell.openInner)
            return;
        const hasEnv = env && Object.keys(env).length > 0;
        shell.openInner(command.inner, command.title, "embedded", hasEnv ? JSON.stringify(env) : "");
    }

    function copyText(text) {
        clipboard.text = text;
        clipboard.selectAll();
        clipboard.copy();
    }

    function openGenerate(parentWindow) {
        const dlg = lastGenerate = generateComponent.createObject(shell, {
            "host": host,
            "transientParent": parentWindow || shell
        });
        dlg.show();
        return dlg;
    }

    function openChoice(title, label, choices, parentWindow) {
        const dlg = lastChoice = choiceComponent.createObject(shell, {
            "host": host,
            "title": title,
            "label": label,
            "choices": choices,
            "transientParent": parentWindow || shell
        });
        dlg.show();
        return dlg;
    }

    function openMasterPassword() {
        masterDialog = masterComponent.createObject(shell, {
            "host": host,
            "transientParent": shell
        });
        masterDialog.show();
        return masterDialog;
    }

    // Edit the named credentials, then follow renames / user changes / deletes into the entries
    // that use them; saving seals new passwords (the vault).
    function openCredentials() {
        const dlg = credentialsDialog = credentialsComponent.createObject(shell, {
            "host": host,
            "transientParent": shell,
            "creds": JSON.parse(vaultBackend.credentialsJson())
        });
        dlg.accepted.connect(() => {
            vaultBackend.applyCredentials(JSON.stringify(dlg.credentials()), JSON.stringify(dlg.renames()));
            shell.controller.persist();
        });
        dlg.show();
        return dlg;
    }

    function openSshKeys() {
        if (sshKeys === null)
            sshKeys = sshKeysComponent.createObject(shell, {
                "host": host,
                "transientParent": shell
            });
        sshKeys.show();
        sshKeys.raise();
        sshKeys.requestActivate();
        return sshKeys;
    }

    Component {
        id: passwordComponent

        PasswordDialog {}
    }
    Component {
        id: choiceComponent

        ChoiceDialog {}
    }
    Component {
        id: generateComponent

        GenerateKeyDialog {}
    }
    Component {
        id: masterComponent

        MasterPasswordDialog {}
    }
    Component {
        id: credentialsComponent

        CredentialsDialog {}
    }
    Component {
        id: messageComponent

        MessageBoxDialog {}
    }
    Component {
        id: sshKeysComponent

        SshKeysDialog {}
    }

    VaultBackend {
        id: vaultBackend
    }

    // QML's way to the clipboard.
    TextEdit {
        id: clipboard

        visible: false
    }

    Connections {
        target: vaultBackend

        function onUnlockFinished(token, json) {
            host.answer(token, json);
        }

        function onMasterFinished(token, json) {
            host.answer(token, json);
        }

        function onKeysListed(token, json) {
            host.answer(token, json);
        }
    }

    // New plain-text passwords were left out of the saved file because the vault is locked: ask,
    // save again; on Cancel say so instead of failing silently.
    Connections {
        target: host.shell.controller

        function onSealNeeded() {
            if (host.sealing)
                return;
            host.sealing = true;
            host.ensureUnlocked(qsTr("Unlock to save the new password encrypted."), () => {
                host.sealing = false;
                host.shell.controller.persist();
            }, () => {
                host.sealing = false;
                host.message("Termigator", qsTr("The new password was not saved to disk — saved passwords are encrypted and the master password was not entered.\nIt works until you quit; enter the master password on the next save or connect to keep it."), host.shell);
            }, host.shell);
        }
    }

    Component.onCompleted: {
        shell.unlockVault = (retry, cancelled) => ensureUnlocked(qsTr("This connection uses a saved password."), retry, cancelled, shell);
        const a = shell.shellActions;
        a.handle("tools.master_password", openMasterPassword);
        a.handle("tools.credentials", openCredentials);
        a.handle("tools.ssh_keys", openSshKeys);
    }
}
