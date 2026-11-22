import QtQuick
import QtQuick.Controls as QQC2
import Termigator

// Tunnels, snippets, bulk edit and the disconnect notice. One instance in ShellWindow.
//
// What it sets on the shell:
//   runningTunnels            the tunnels starting or running (the close question)
//   openBulkEdit              fn(count, groups, done): "Edit N entries…" — done(changes) after OK
//                             when something was ticked (the host list applies and saves it)
//   Tools → Tunnels…          the one Tunnels window (non-modal); autostart tunnels start with the app
//   Edit → Snippets           the saved snippets and "Manage snippets…"
// On the panes area (`shell.panes`, set by PanesArea):
//   disconnectNotice          fn(tab, label, livedSeconds): the popup (+ a tray message when the
//                             window is not the active one)
//   testConnection            fn(connJson): the tab menu's "Test connection…"
// The vault: a tunnel whose SSH entry has a sealed password asks `panes.unlockVault(retry)`.
Item {
    id: host

    required property var shell
    readonly property alias backend: backend
    readonly property alias prompt: prompt
    property int running: 0
    // The windows opened last, for the smoke check.
    property var tunnelsWindow: null
    property var lastNotice: null
    property var lastTest: null
    property var lastSnippets: null
    property var lastBulk: null

    function persist() {
        shell.controller.persist();
    }

    // ---- tunnels

    // A tunnel's start; a locked vault asks for the master password first, like Connect.
    function startTunnel(name) {
        if (backend.startTunnel(name) !== "locked")
            return;
        const panes = shell.panes;
        if (panes && panes.unlockVault)
            panes.unlockVault(() => host.startTunnel(name));
        else
            backend.failTunnel(name, qsTr("the saved password is locked (master password not entered)"));
    }

    function openTunnels() {
        if (!tunnelsWindow)
            tunnelsWindow = tunnelsComponent.createObject(shell, {
                "host": host,
                "controller": shell.controller,
                "transientParent": shell
            });
        tunnelsWindow.reload();
        tunnelsWindow.show();
        tunnelsWindow.raise();
        tunnelsWindow.requestActivate();
        return tunnelsWindow;
    }

    function autostart() {
        JSON.parse(backend.autostartJson()).forEach(name => startTunnel(name));
    }

    // ---- snippets

    function manageSnippets() {
        const dlg = lastSnippets = snippetsComponent.createObject(shell, {
            "controller": shell.controller,
            "initial": JSON.parse(backend.snippetsJson()),
            "transientParent": shell
        });
        dlg.accepted.connect(() => {
            backend.saveSnippets(JSON.stringify(dlg.snippets()));
            persist();
            fillSnippetsMenu();
        });
        dlg.show();
        return dlg;
    }

    // Fills the snippets menu: one entry per snippet, then "Manage snippets…".
    function fillSnippetsMenu() {
        const menu = shell.shellActions.menus["Edit/Snippets"];
        if (!menu)
            return;
        while (menu.count > 0)
            menu.removeItem(menu.itemAt(0));
        const list = JSON.parse(backend.snippetsJson());
        for (const sn of list)
            menu.addItem(snippetItem.createObject(null, {
                "text": sn.name,
                "snippet": sn
            }));
        if (list.length > 0)
            menu.addItem(separatorItem.createObject(null));
        menu.addItem(manageItem.createObject(null));
    }

    // Runs a snippet: typed into the current terminal, or with MultiExec on into every
    // MultiExec session, {host}/{user}/{label}/{port} filled per session.
    function runSnippet(sn) {
        const panes = shell.panes;
        const terminals = panes ? panes.terminalHost : null;
        let targets = [];
        if (terminals) {
            if (terminals.multiexecOn) {
                targets = terminals.multiexecTargets();
            } else {
                const t = terminals.currentTerminal();
                targets = t ? [t] : [];
            }
        }
        if (targets.length === 0) {
            panes.message(qsTr("Open an embedded terminal to send a snippet to."));
            return;
        }
        const asked = JSON.parse(backend.snippetQuestions(sn.command));
        const answers = {};
        const next = i => {
            if (i >= asked.length) {
                confirmSnippet(terminals, targets, sn, () => sendSnippet(targets, sn, answers));
                return;
            }
            prompt.ask(sn.name, asked[i] + ":", "", null, 0, text => {
                answers[asked[i]] = text;
                next(i + 1);
            });
        };
        next(0);
    }

    // A snippet for more than one session, or of several lines, is shown first.
    function confirmSnippet(terminals, targets, sn, send) {
        const lines = String(sn.command).split("\n");
        if (targets.length < 2 && lines.length < 2) {
            send();
            return;
        }
        const preview = lines.slice(0, 8).join("\n") + (lines.length > 8 ? "\n…" : "");
        shell.panes.ask(qsTr("Send the snippet “%1” to %2 session(s)?\n\n%3\n\n%4").arg(sn.name).arg(targets.length).arg(preview).arg(terminals.targetListing(targets)), send);
    }

    function sendSnippet(targets, sn, answers) {
        for (const t of targets) {
            const info = {};
            for (const k of ["host", "user", "label", "port"])
                if (t.conn && typeof t.conn === "object" && k in t.conn)
                    info[k] = t.conn[k];
            if (t.session)
                t.session.ext.sendText(backend.snippetText(sn.command, sn.enter !== false, JSON.stringify(info), t.title || "", JSON.stringify(answers)));
        }
    }

    // ---- bulk edit

    function bulkEdit(count, groups, done) {
        const dlg = lastBulk = bulkComponent.createObject(shell, {
            "controller": shell.controller,
            "count": count,
            "groups": groups,
            "credentials": JSON.parse(backend.credentialNamesJson()),
            "environments": JSON.parse(backend.environmentsJson()),
            "transientParent": shell
        });
        dlg.accepted.connect(() => {
            const changes = dlg.changes();
            if (Object.keys(changes).length > 0)
                done(changes);
        });
        dlg.show();
        return dlg;
    }

    // ---- connection health

    function testConnection(connJson, heading, parentWindow) {
        const conn = JSON.parse(connJson);
        lastTest = testComponent.createObject(shell, {
            "backend": backend,
            "conn": connJson,
            "heading": heading || String(conn.label || conn.host || "connection"),
            "transientParent": parentWindow || shell
        });
        lastTest.show();
        return lastTest;
    }

    // Only connection tabs qualify (a local shell exiting is not news).
    function notice(tab, label, lived) {
        if (!tab.conn || typeof tab.conn !== "object")
            return null;
        const panes = shell.panes;
        const n = lastNotice = noticeComponent.createObject(shell, {
            "host": host,
            "controller": shell.controller,
            "conn": JSON.stringify(tab.conn),
            "label": label,
            "lived": lived,
            "reconnect": () => panes.restartTab(tab),
            "mute": () => {
                backend.muteDisconnectNotices();
                host.persist();
            },
            "transientParent": shell
        });
        n.show();
        n.raise();
        if (!shell.active)
            shell.trayMessage(qsTr("Termigator — session ended"), qsTr("%1 disconnected.").arg(label));
        return n;
    }

    function wire() {
        const panes = shell.panes;
        if (!panes)
            return;
        panes.disconnectNotice = host.notice;
        panes.testConnection = connJson => host.testConnection(connJson, "", null);
    }

    visible: false

    ToolsBackend {
        id: backend

        onTunnelChanged: host.running = backend.runningCount()
    }

    // The tunnel processes report on their own threads; this applies what arrived.
    Timer {
        interval: 200
        running: true
        repeat: true
        onTriggered: backend.poll()
    }

    PromptDialog {
        id: prompt
    }

    Connections {
        target: host.shell

        function onPanesChanged() {
            host.wire();
        }

        // No `ssh -N` outlives the application.
        function onShuttingDown() {
            backend.stopAll();
        }
    }

    Component {
        id: tunnelsComponent

        TunnelsDialog {}
    }
    Component {
        id: snippetsComponent

        SnippetsDialog {}
    }
    Component {
        id: bulkComponent

        BulkEditDialog {}
    }
    Component {
        id: noticeComponent

        DisconnectNotice {}
    }
    Component {
        id: testComponent

        TestConnectionDialog {}
    }
    Component {
        id: snippetItem

        QQC2.MenuItem {
            property var snippet

            onTriggered: host.runSnippet(snippet)
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: snippet ? snippet.command : ""
        }
    }
    Component {
        id: separatorItem

        QQC2.MenuSeparator {}
    }
    Component {
        id: manageItem

        QQC2.MenuItem {
            text: qsTr("Manage snippets…")
            onTriggered: host.manageSnippets()
        }
    }

    Component.onCompleted: {
        shell.runningTunnels = Qt.binding(() => host.running);
        shell.openBulkEdit = host.bulkEdit;
        shell.shellActions.handle("tools.tunnels", () => host.openTunnels());
        host.wire();
        // After the window has built its menus, and after the shell is up (a zero-delay single shot).
        Qt.callLater(() => {
            host.fillSnippetsMenu();
            host.autostart();
        });
    }
}
