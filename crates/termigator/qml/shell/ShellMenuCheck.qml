import QtQuick

// Smoke-only check: logs the shell button's press-and-hold menu (text and enabled), opens the
// default local shell (or `--shell=<id>`, an id of that menu) and saves the window content.
Item {
    id: check

    required property var shell
    property int tries: 0
    readonly property string shellId: Qt.application.arguments.filter(a => a.startsWith("--shell=")).map(a => a.slice(8))[0] || ""

    function find(item, name) {
        if (!item)
            return null;
        if (item.objectName === name)
            return item;
        for (const c of item.children) {
            const f = find(c, name);
            if (f)
                return f;
        }
        return null;
    }

    function panelOf(item) {
        for (let p = item; p; p = p.parent)
            if (typeof p.buildShellMenu === "function")
                return p;
        return null;
    }

    Timer {
        interval: 300
        repeat: true
        running: true
        onTriggered: {
            const button = check.find(check.shell.contentItem, "sidebarShell");
            const panel = check.panelOf(button);
            if (!check.shell.newLocalShell || !panel) {
                if (++check.tries > 30) {
                    console.error("shell menu check: no shell button");
                    Qt.exit(1);
                }
                return;
            }
            stop();
            const m = panel.buildShellMenu();
            for (let i = 0; i < m.count; ++i) {
                const it = m.itemAt(i);
                if (it && it.text !== undefined)
                    console.log("shell menu check: item", JSON.stringify(it.text), "enabled", it.enabled);
            }
            m.destroy();
            const add = panel.buildAddMenu();
            for (let i = 0; i < add.count; ++i) {
                const it = add.itemAt(i);
                if (it && it.text)
                    console.log("shell menu check: add", JSON.stringify(it.text), "enabled", it.enabled);
            }
            add.destroy();
            for (const kind of ["ssh", "rdp", "vnc", "telnet", "serial", "import.remmina"]) {
                const act = check.shell.shellActions.actions[(kind.startsWith("import.") ? "file." : "file.new.") + kind];
                console.log("shell menu check: action", kind, JSON.stringify(act.text), "enabled", act.enabled);
            }
            check.shell.newLocalShell(check.shellId);
            grab.start();
        }
    }

    Timer {
        id: grab

        interval: 5000
        onTriggered: check.shell.contentArea.grabToImage(result => {
            const ok = result.saveToFile(check.shell.controller.smokeTestShot);
            console.log("shell menu check: wrote", check.shell.controller.smokeTestShot, ok);
            Qt.exit(ok ? 0 : 1);
        })
    }
}
