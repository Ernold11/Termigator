import QtQuick
import QtQuick.Dialogs
import Termigator

// The Files side of the window. ShellWindow instantiates one; it sets the
// shell's hooks:
//   shell.openFilesTab(entryJson)          a Files tab for an entry (files_only entries, "Open files")
//   shell.scpTransfer(entryJson, upload)   the host menu's one-shot scp (runs in a terminal tab)
// and keeps `shell.activeTransfers` (the close question counts them).
//
//   viewComponent        the content of a Files tab (a FilesPanel for `tab.session`)
//   newSession(key)      a FilesSession for the side panel
//   ensureSession(tab)   the tab's FilesSession, made on first use and kept in `tab.session`
//   reportTransfers      FilesSession tells how many of its transfers are active
//   runScp(...)          the end of the host menu's flow, once the paths are known
Item {
    id: host

    required property var shell
    readonly property alias viewComponent: filesViewComponent
    readonly property alias messageBox: box
    readonly property alias prompt: promptDialog
    property var counts: ({})
    readonly property var byTab: ({})
    // Names the queues; a box so that newSession() in a binding does not depend on itself.
    readonly property var serial: ({
            "n": 0
        })
    // Where a one-shot scp is asked for: the file, then the remote path.
    property string scpEntry: ""

    visible: false

    function reportTransfers(key, n) {
        const c = Object.assign({}, counts);
        if (n > 0)
            c[key] = n;
        else
            delete c[key];
        counts = c;
        shell.activeTransfers = Object.values(c).reduce((a, b) => a + b, 0);
    }

    // A local path as a `file:` URL (FilesHelper: QUrl, right for Windows drive paths too).
    function fileUrl(p) {
        return helper.fileUrl(p);
    }

    function warn(text) {
        box.text = text;
        box.open();
    }

    function newSession(twoPaneKey) {
        serial.n += 1;
        return sessionComponent.createObject(host, {
            "host": host,
            "key": "files" + serial.n,
            "twoPaneKey": twoPaneKey
        });
    }

    // Looked up by tab id in a plain object, not in `tab.session`: this is called from the view's
    // `session` binding, which must not depend on the property it ends up setting.
    function ensureSession(tab) {
        let s = byTab[tab.tabId];
        if (!s) {
            s = newSession("files_two_pane_tabs");
            byTab[tab.tabId] = s;
            tab.session = s;
        }
        return s;
    }

    // The host's files in a tab of their own, no terminal.
    function openTab(entryJson) {
        const area = shell.panesArea;
        if (!area)
            return null;
        const entry = JSON.parse(entryJson);
        const label = helper.entryLabel(entryJson);
        const tab = area.addTab({
            "title": "Files · " + label,
            "embedded": false,
            "conn": entry,
            "content": filesViewComponent,
            "toolTip": qsTr("%1 — files (SFTP)").arg(label)
        });
        const session = ensureSession(tab);
        session.files.setTarget(entryJson, label);
        tab.closeQuestion = () => session.transfers.question();
        tab.closing.connect(() => {
            session.shutdown(); // stop its scp jobs
            delete byTab[tab.tabId];
            session.destroy();
        });
        helper.touchRecent(entryJson);
        shell.controller.persist();
        return tab;
    }

    // ---- the host menu's one-shot scp (scp transfer from a list item)

    function scpTransfer(entryJson, upload) {
        if (!helper.isSsh(entryJson))
            return;
        scpEntry = entryJson;
        const label = helper.entryLabel(entryJson);
        if (upload) {
            uploadPick.title = qsTr("Upload file to %1").arg(label);
            uploadPick.open();
        } else {
            askRemote(label, false, "");
        }
    }

    function askRemote(label, upload, local) {
        if (upload)
            promptDialog.ask(qsTr("Upload file"), qsTr("Remote destination on %1\n(directory or target path; ~/ = home):").arg(label), "~/", null, 0, remote => {
                if (remote.trim() !== "")
                    runScp(scpEntry, local, remote.trim(), true);
            });
        else
            promptDialog.ask(qsTr("Download file"), qsTr("Remote file on %1 (e.g. /var/log/syslog or ~/file.txt):").arg(label), "", null, 0, remote => {
                if (remote.trim() === "")
                    return;
                saveDownload.remote = remote.trim();
                saveDownload.title = qsTr("Save file from %1").arg(label);
                saveDownload.currentFile = helper.fileUrl(helper.homeDir() + "/" + helper.suggestedName(remote));
                saveDownload.open();
            });
    }

    // Builds the scp line and runs it in a terminal tab (or the entry's external terminal).
    function runScp(entryJson, local, remote, upload) {
        const p = JSON.parse(helper.scpPlan(entryJson, local, remote, upload));
        if (p.error) {
            warn(p.error);
            return;
        }
        if (p.locked) {
            const area = shell.panesArea;
            if (area && area.unlockVault)
                area.unlockVault(() => host.runScp(entryJson, local, remote, upload));
            else
                warn(p.text);
            return;
        }
        if (p.cmd === undefined || !shell.openInner)
            return;
        shell.openInner(p.cmd, p.title, p.mode, JSON.stringify(p.env));
    }

    // The tab's SSH entry (embedded terminal tabs only — a Files tab has none), else null.
    function connOf(t) {
        if (!t || !t.embedded || t.conn === null || typeof t.conn !== "object")
            return null;
        return helper.isSsh(JSON.stringify(t.conn)) ? t.conn : null;
    }

    Component.onCompleted: {
        shell.openFilesTab = json => host.openTab(json);
        shell.scpTransfer = (json, upload) => host.scpTransfer(json, upload);
        // Files for the current tab: on only for an SSH tab.
        const conn = () => connOf(shell.panesArea ? shell.panesArea.activeTab : null);
        shell.shellActions.handle("session.files", () => {
            const c = conn();
            if (c !== null)
                host.openTab(JSON.stringify(c));
        });
        shell.shellActions.actions["session.files"].enabled = Qt.binding(() => conn() !== null);
    }

    FilesHelper {
        id: helper
    }

    // The window is closing for good: stop every panel's transfers.
    Connections {
        target: host.shell

        function onShuttingDown() {
            for (const c of host.children)
                if (typeof c.shutdown === "function")
                    c.shutdown();
        }
    }

    Component {
        id: sessionComponent

        FilesSession {}
    }

    Component {
        id: filesViewComponent

        // The panel is made with its session as an initial property: a binding to it would be
        // read once before it is set.
        Item {
            id: view

            required property var tab

            Component.onCompleted: {
                const s = host.ensureSession(tab);
                const p = panelComponent.createObject(view, {
                    "session": s,
                    "files": s.files,
                    "transfers": s.transfers
                });
                p.anchors.fill = view;
            }
        }
    }

    Component {
        id: panelComponent

        FilesPanel {}
    }

    MessageBox {
        id: box

        title: "Termigator"
        buttons: MessageDialog.Ok
    }

    PromptDialog {
        id: promptDialog
    }

    FileDialog {
        id: uploadPick

        fileMode: FileDialog.OpenFile
        onAccepted: host.askRemote(helper.entryLabel(host.scpEntry), true, helper.localPath(selectedFile.toString()))
    }

    FileDialog {
        id: saveDownload

        property string remote: ""

        fileMode: FileDialog.SaveFile
        onAccepted: host.runScp(host.scpEntry, helper.localPath(selectedFile.toString()), remote, false)
    }
}
