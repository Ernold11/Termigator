import QtQuick
import QtQuick.Dialogs
import QtCore
import Termigator

// File → Import / Export. One instance in ShellWindow. It handles file.import.mobaxterm|putty|
// ssh_config|remmina, file.export.ssh_config and file.export.backup; "Open config folder" is the
// shell's. The parsing and merging is ImportBackend (import_qobj.rs): an action returns
// {text, changed, warn}; when the config changed `finish` saves it, which also brings the host list
// back (controller.configChanged), then shows the text in a message box (vault host).
//
// Smoke checks drive the *Path / run* functions directly and read `lastMessage`, `lastSaved`;
// the pickers (`pickers.<name>`) are the file dialogs the actions opened.
Item {
    id: host

    required property var shell
    readonly property alias backend: importBackend
    readonly property var vault: shell.vaultHost
    property string lastMessage: ""
    property string lastSaved: ""
    readonly property alias pickers: pickerSet

    // file: URLs and local paths, right for Windows drive paths too (file:///C:/dir <-> C:/dir).
    function localPath(url) {
        const p = decodeURIComponent(url.toString().replace(/^file:\/\//, ""));
        return /^\/[A-Za-z]:/.test(p) ? p.slice(1) : p;
    }

    function fileUrl(p) {
        const s = /^[A-Za-z]:/.test(p) ? "/" + p.replace(/\\/g, "/") : p;
        return "file://" + s.split("/").map(x => /^[A-Za-z]:$/.test(x) ? x : encodeURIComponent(x)).join("/");
    }

    // Shows an ImportBackend result; saves the config when it changed. Returns the parsed result.
    function finish(json) {
        const res = JSON.parse(json);
        if (res.changed)
            shell.controller.persist();
        lastMessage = res.text;
        vault.message("Termigator", res.text, shell);
        return res;
    }

    // Windows: this machine's MobaXterm first (Yes), else an exported .ini / .mxtsessions (No).
    function importMoba() {
        const found = JSON.parse(importBackend.mobaLocal());
        if (!found.where) {
            pickerSet.moba.open();
            return;
        }
        vault.ask(qsTr("Import MobaXterm sessions"), qsTr("Found %1 session(s) in %2.\nImport them?\n\n(Choose No to pick an exported .ini / .mxtsessions file instead.)").arg(found.count).arg(found.where), shell, () => finish(importBackend.importMoba("")));
        vault.messageWindow.rejected.connect(() => pickerSet.moba.open());
    }

    // SuperPuTTY's Sessions.xml where SuperPuTTY keeps it (Yes), else a file to pick.
    function importSuperPutty() {
        const path = importBackend.superputtyPath();
        if (path === "") {
            pickerSet.superputty.open();
            return;
        }
        vault.ask(qsTr("Import SuperPuTTY sessions"), qsTr("Import the sessions of %1?\n\n(Choose No to pick another sessions .xml file.)").arg(path), shell, () => finish(importBackend.importPutty(path)));
        vault.messageWindow.rejected.connect(() => pickerSet.superputty.open());
    }

    // The local sessions folder first (Yes), else a .reg / SuperPuTTY .xml (No).
    function importPutty() {
        const n = importBackend.puttyLocalCount();
        if (n <= 0) {
            pickerSet.putty.open();
            return;
        }
        vault.ask(qsTr("Import PuTTY sessions"), qsTr("Found %1 SSH session(s) in %2.\nImport them?\n\n(Choose No to pick a .reg / .xml file instead.)").arg(n).arg(importBackend.puttySessionsDir()), shell, () => finish(importBackend.importPutty("")));
        vault.messageWindow.rejected.connect(() => pickerSet.putty.open());
    }

    // ~/.ssh/config when there is one, else a file to pick.
    function importSshConfig() {
        const path = importBackend.sshConfigPath();
        if (path !== "")
            finish(importBackend.importSshConfig(path));
        else
            pickerSet.sshConfig.open();
    }

    function importRemmina() {
        if (importBackend.remminaFound())
            finish(importBackend.importRemmina("[]"));
        else
            pickerSet.remmina.open();
    }

    function exportSshConfig() {
        if (importBackend.sshExportCount() === 0) {
            finish(importBackend.exportSshConfig(""));
            return;
        }
        pickerSet.exportSsh.currentFile = fileUrl(importBackend.defaultExportPath());
        pickerSet.exportSsh.open();
    }

    // Unencrypted passwords go into the zip as they are: ask first.
    function backupSettings() {
        const plain = importBackend.plaintextPasswords();
        if (plain === 0) {
            pickerSet.backup.currentFile = fileUrl(importBackend.defaultBackupPath());
            pickerSet.backup.open();
            return;
        }
        vault.ask(qsTr("Back up settings"), qsTr("%1 saved password(s) are stored in plain text and would be in the zip as they are.\nSet a master password first (Tools → Master password…) to have them encrypted.\n\nCreate the backup anyway?").arg(plain), shell, () => {
            pickerSet.backup.currentFile = fileUrl(importBackend.defaultBackupPath());
            pickerSet.backup.open();
        });
    }

    // The zip holds what is on screen now: save first.
    function writeBackup(path) {
        shell.controller.persist();
        const res = JSON.parse(importBackend.backup(path));
        lastMessage = res.text;
        if (res.warn)
            vault.message("Termigator", res.text, shell);
        else {
            lastSaved = path;
            shell.notify(res.text);
        }
        return res;
    }

    ImportBackend {
        id: importBackend
    }

    QtObject {
        id: pickerSet

        readonly property FileDialog moba: FileDialog {
            title: qsTr("Import MobaXterm sessions")
            currentFolder: StandardPaths.writableLocation(StandardPaths.HomeLocation)
            nameFilters: [qsTr("MobaXterm config (*.ini *.mxtsessions)"), qsTr("All files (*)")]
            onAccepted: host.finish(importBackend.importMoba(host.localPath(selectedFile)))
        }
        readonly property FileDialog putty: FileDialog {
            title: qsTr("Import PuTTY sessions")
            currentFolder: StandardPaths.writableLocation(StandardPaths.HomeLocation)
            nameFilters: [qsTr("PuTTY exports (*.reg *.xml)"), qsTr("All files (*)")]
            onAccepted: host.finish(importBackend.importPutty(host.localPath(selectedFile)))
        }
        readonly property FileDialog superputty: FileDialog {
            title: qsTr("Import SuperPuTTY sessions")
            currentFolder: StandardPaths.writableLocation(StandardPaths.DocumentsLocation)
            nameFilters: [qsTr("SuperPuTTY sessions (*.xml)"), qsTr("All files (*)")]
            onAccepted: host.finish(importBackend.importPutty(host.localPath(selectedFile)))
        }
        readonly property FileDialog sshConfig: FileDialog {
            title: qsTr("Import OpenSSH client config")
            currentFolder: StandardPaths.writableLocation(StandardPaths.HomeLocation)
            onAccepted: host.finish(importBackend.importSshConfig(host.localPath(selectedFile)))
        }
        readonly property FileDialog remmina: FileDialog {
            title: qsTr("Import Remmina profiles")
            fileMode: FileDialog.OpenFiles
            currentFolder: StandardPaths.writableLocation(StandardPaths.HomeLocation)
            nameFilters: [qsTr("Remmina profiles (*.remmina)"), qsTr("All files (*)")]
            onAccepted: host.finish(importBackend.importRemmina(JSON.stringify(selectedFiles.map(u => host.localPath(u)))))
        }
        readonly property FileDialog exportSsh: FileDialog {
            title: qsTr("Export SSH hosts")
            fileMode: FileDialog.SaveFile
            nameFilters: [qsTr("ssh config (*.conf)"), qsTr("All files (*)")]
            onAccepted: host.finish(importBackend.exportSshConfig(host.localPath(selectedFile)))
        }
        readonly property FileDialog backup: FileDialog {
            title: qsTr("Back up settings")
            fileMode: FileDialog.SaveFile
            nameFilters: [qsTr("Zip (*.zip)")]
            defaultSuffix: "zip"
            onAccepted: host.writeBackup(host.localPath(selectedFile))
        }
    }

    Component.onCompleted: {
        const a = shell.shellActions;
        a.handle("file.import.mobaxterm", () => importMoba());
        a.handle("file.import.putty", () => importPutty());
        a.handle("file.import.superputty", () => importSuperPutty());
        a.handle("file.import.ssh_config", () => importSshConfig());
        a.handle("file.import.remmina", () => importRemmina());
        // Remmina is a Linux program: nothing of it to import on Windows.
        if (Qt.platform.os === "windows") {
            const remmina = a.actions["file.import.remmina"];
            remmina.enabled = false;
            remmina.text = remmina.text + " " + qsTr("(Linux only)");
        }
        a.handle("file.export.ssh_config", () => exportSshConfig());
        a.handle("file.export.backup", () => backupSettings());
    }
}
