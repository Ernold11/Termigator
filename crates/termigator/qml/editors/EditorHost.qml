import QtQuick
import Termigator

// Connection editors. One instance in ShellWindow; it sets `shell.openEditor` and
// handles File → Quick connect….
//
//   shell.openEditor(kind, entryJson, defaultGroup, onAccepted)
//       kind: ssh|rdp|vnc|telnet|serial|favorite; entryJson: the entry (Value::to_json) or "" for
//       a new one; onAccepted(dataJson) on OK with the entry to store, while the editor is still
//       open. Validating it, storing it and saving the config is the caller's; it returns "" (the editor closes) or the reason shown in the editor.
//       A favorite without label or command is answered here ("Enter label and command.") and
//       onAccepted is not called.
//   quickSsh()
//       File → Quick connect…: asks for a target, then calls shell.quickConnectHandler(text, mode)
//       (mode "embedded"/"external") and shows its error text.
Item {
    id: host

    required property var shell
    // The last "Test connection" window opened (the smoke check follows it).
    property var lastTest: null
    property var lastQuick: null
    // The message window that is open, else null (MessageBoxDialog's `host` contract, as in VaultHost).
    property var messageWindow: null

    function open(kind, entryJson, defaultGroup, onAccepted) {
        const entry = entryJson || "";
        const text = backend.prepare(kind, entry, defaultGroup || "");
        if (text === "")
            return;
        const dlg = components[kind].createObject(shell, {
            "controller": shell.controller,
            "backend": backend,
            "transientParent": shell,
            "ctx": JSON.parse(text)
        });
        // Stored before the window closes: an error keeps the form.
        dlg.validate = () => {
            const data = backend.build(kind, entry, JSON.stringify(dlg.collect()));
            if (data === "")
                return qsTr("Enter label and command.");
            const reason = onAccepted ? onAccepted(data) : "";
            return typeof reason === "string" ? reason : "";
        };
        dlg.testRequested.connect(() => testConnection(dlg, kind, entry));
        dlg.show();
        return dlg;
    }

    // Probes the values in the form, so a host can be checked before the entry is saved.
    function testConnection(dlg, kind, entry) {
        const data = backend.build(kind, entry, JSON.stringify(dlg.collect()));
        const conn = data === "" ? ({}) : JSON.parse(data);
        const target = String(conn.host || "").trim();
        if (target === "") {
            message(qsTr("Test connection"), qsTr("Enter a host first."), dlg);
            return;
        }
        const name = String(conn.label || conn.host || "connection");
        lastTest = testComponent.createObject(shell, {
            "backend": backend,
            "conn": data,
            "heading": name,
            "transientParent": dlg
        });
        lastTest.show();
    }

    function quickSsh() {
        const dlg = lastQuick = quickComponent.createObject(shell, {
            "controller": shell.controller,
            "transientParent": shell,
            "defaultTerminal": backend.defaultTerminal()
        });
        dlg.accepted.connect(() => {
            const handler = shell.quickConnectHandler;
            const err = handler ? handler(dlg.target, dlg.terminal) : "";
            if (err)
                message(qsTr("Quick SSH"), err, shell);
        });
        dlg.show();
        return dlg;
    }

    function message(title, text, parentWindow) {
        // A window of its own: the MessageDialog of QtQuick.Dialogs crashes once the editor it was shown over is destroyed.
        messageWindow = messageComponent.createObject(shell, {
            "host": host,
            "title": title,
            "text": text,
            "transientParent": parentWindow || shell
        });
        messageWindow.show();
    }

    readonly property var components: ({
            "ssh": sshComponent,
            "rdp": rdpComponent,
            "vnc": vncComponent,
            "telnet": telnetComponent,
            "serial": serialComponent,
            "favorite": favoriteComponent
        })

    EditorBackend {
        id: backend
    }

    Component {
        id: messageComponent

        MessageBoxDialog {}
    }

    Component {
        id: sshComponent

        SshDialog {}
    }
    Component {
        id: rdpComponent

        RdpDialog {}
    }
    Component {
        id: vncComponent

        VncDialog {}
    }
    Component {
        id: telnetComponent

        TelnetDialog {}
    }
    Component {
        id: serialComponent

        SerialDialog {}
    }
    Component {
        id: favoriteComponent

        FavoriteDialog {}
    }
    Component {
        id: quickComponent

        QuickSshDialog {}
    }
    Component {
        id: testComponent

        TestConnectionDialog {}
    }

    Component.onCompleted: {
        shell.openEditor = open;
        shell.shellActions.handle("file.quick_connect", quickSsh);
    }
}
