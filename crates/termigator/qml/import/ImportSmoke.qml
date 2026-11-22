import QtQuick

// Offscreen check (tests/import_smoke.rs): the File → Import / Export actions are live; the
// ssh_config, Remmina, PuTTY (Yes on the question) and MobaXterm imports merge, save and say what
// they skipped; a repeat adds nothing; the ssh_config export and the backup (question about plain
// text passwords, then the zip) write their files. HOME and XDG_DATA_HOME point into the
// fixture folder given as --u10-dir=<dir> (the Rust side writes the inputs and reads the outputs).
// The file pickers are not opened here (native dialogs); the functions behind them are driven.
Item {
    id: check

    required property var shell
    readonly property var host: shell.importHost
    readonly property string dir: Qt.application.arguments.find(s => s.startsWith("--u10-dir=")).slice("--u10-dir=".length)
    property int step: 0
    property int waits: 0
    property bool failed: false
    property int saved: 0

    function fail(reason) {
        failed = true;
        console.error("termigator import smoke: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok && !failed)
            fail(what);
    }

    function starts(text, prefix, what) {
        expect(text.startsWith(prefix), what + ": " + text);
    }

    readonly property var steps: [
        () => {
            for (const id of ["file.import.mobaxterm", "file.import.putty", "file.import.superputty", "file.import.ssh_config", "file.import.remmina", "file.export.ssh_config", "file.export.backup"])
                expect(shell.shellActions.actions[id].enabled, id + " has no handler");
            expect(host.backend.sshConfigPath() === dir + "/home/.ssh/config", "ssh config path " + host.backend.sshConfigPath());
            expect(host.backend.remminaFound() && host.backend.puttyLocalCount() === 1, "remmina / putty not found");
        },
        // The menu action runs the import; the config is saved (configChanged) and the text shown.
        () => {
            shell.controller.configChanged.connect(() => ++check.saved);
            shell.shellActions.actions["file.import.ssh_config"].trigger();
            starts(host.lastMessage, "Imported 1 host(s) from ", "ssh_config");
            expect(host.lastMessage.indexOf("(1 duplicate(s) skipped).") > 0, "duplicate not mentioned: " + host.lastMessage);
            expect(saved === 1 && shell.vaultHost.messageWindow.text === host.lastMessage, "not saved / shown: " + saved);
        },
        () => {
            host.importSshConfig();
            starts(host.lastMessage, "Nothing to import — all 2 host(s)", "second ssh_config");
            expect(saved === 1, "saved though nothing changed");
            host.importRemmina();
            starts(host.lastMessage, "Imported 1 profile(s) from Remmina", "remmina");
            expect(host.lastMessage.indexOf("Not supported: 1× SPICE.") > 0, "SPICE not reported: " + host.lastMessage);
        },
        // PuTTY: Yes on "Found 1 SSH session(s)…".
        () => {
            host.importPutty();
            const q = shell.vaultHost.messageWindow;
            expect(q !== null && q.question && q.text.indexOf("Found 1 SSH session(s) in " + dir + "/home/.putty/sessions.") === 0, "putty question");
            q.accept();
        },
        () => {
            starts(host.lastMessage, "Imported 1 new SSH connection(s) from PuTTY/SuperPuTTY.", "putty");
            host.finish(host.backend.importMoba(dir + "/s.mxtsessions"));
            starts(host.lastMessage, "Imported 1 new connection(s) from MobaXterm.", "moba");
            host.finish(host.backend.importMoba(dir + "/missing.ini"));
            starts(host.lastMessage, "Could not read file:", "moba missing");
            expect(shell.vaultHost.messageWindow.text === host.lastMessage, "warning not shown");
        },
        // Export: the file picker's work.
        () => {
            expect(host.backend.sshExportCount() === 5, "export count " + host.backend.sshExportCount());
            host.finish(host.backend.exportSshConfig(dir + "/out.conf"));
            starts(host.lastMessage, "Exported 5 SSH host(s) to " + dir + "/out.conf", "export");
        },
        // Backup: plain-text passwords are asked about; No writes nothing.
        () => {
            expect(host.backend.plaintextPasswords() === 1, "plain " + host.backend.plaintextPasswords());
            expect(/\/termigator-settings-\d{8}-\d{4}\.zip$/.test(host.backend.defaultBackupPath()), "backup name " + host.backend.defaultBackupPath());
            host.backupSettings();
            const q = shell.vaultHost.messageWindow;
            expect(q !== null && q.question && q.text.startsWith("1 saved password(s) are stored in plain text"), "backup question");
            q.reject();
        },
        () => {
            const res = host.writeBackup(dir + "/backup.zip");
            expect(!res.warn && host.lastSaved === dir + "/backup.zip", "backup " + res.text);
            expect(shell.statusMessage.startsWith("Backup saved: config.yaml, session.yaml, README.txt → "), "status " + shell.statusMessage);
            const bad = host.writeBackup(dir + "/no/such/dir/b.zip");
            expect(bad.warn && bad.text.startsWith("Could not write the backup:"), "bad backup " + bad.text);
        },
        () => {
            const grabbed = shell.contentArea.grabToImage(result => {
                if (!result.saveToFile(shell.controller.smokeTestShot))
                    check.fail("cannot save the grab");
                console.info("termigator import smoke: done");
                Qt.exit(0);
            });
            expect(grabbed, "grab not started");
        }
    ]

    Timer {
        interval: 200
        repeat: true
        running: !check.failed && check.step < check.steps.length
        onTriggered: {
            if (check.steps[check.step]() === false) {
                if (++check.waits > 100)
                    check.fail("timed out waiting");
                return;
            }
            check.waits = 0;
            ++check.step;
        }
    }
}
