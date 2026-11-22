import QtQuick

// Smoke-only (Windows): opens a local shell (the default, or --shell=<id> of the shell menu: cmd, powershell, ...),
// runs `cd <--cd>` and checks (the pane must end at --expect=..., default --cd) that the terminal reports it (remoteCwd, localCwd) and that the Files local pane follows
// and lists the folder. The shell's own PROMPT is used unless --prompt=... is given (cmd only). With
// TERMIGATOR_SMOKE_TEST set the window is saved there at the end.
Item {
    id: check

    required property var shell
    property int step: 0
    property int waited: 0
    function arg(name) {
        return Qt.application.arguments.filter(a => a.startsWith(name + "=")).map(a => a.slice(name.length + 1))[0] || "";
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
    function session() {
        const t = shell.panesArea ? shell.panesArea.activeTab : null;
        return t ? t.session : null;
    }

    readonly property var steps: [
        () => {
            if (!shell.newLocalShell)
                return false;
            shell.newLocalShell(arg("--shell"));
        },
        () => {
            if (!session())
                return false;
            if (arg("--prompt"))
                session().ext.sendText("set PROMPT=" + arg("--prompt") + "\r");
            session().ext.sendText("cd " + arg("--cd") + "\r");
        },
        () => {
            if (session().remoteCwd === "")
                return false;
            console.log("local cwd check: remoteCwd =", JSON.stringify(session().remoteCwd), "localCwd =", JSON.stringify(session().localCwd()));
            shell.sideTabBar.currentIndex = 1;
        },
        () => {
            const panel = find("filesPanel", shell.contentItem);
            if (!panel || !panel.local || panel.local.currentDir() !== (arg("--expect") || arg("--cd")) || panel.local.model.count === 0)
                return false;
            console.log("local cwd check: Files local pane follows to", panel.local.currentDir(), "entries", panel.local.model.count);
            if (!shell.controller.smokeTestShot)
                return Qt.exit(0);
            shell.sideTabBar.currentIndex = 1;
            shell.contentArea.grabToImage(result => {
                console.log("local cwd check: wrote", shell.controller.smokeTestShot, result.saveToFile(shell.controller.smokeTestShot));
                Qt.exit(0);
            });
        },
        () => {
            return false; // waits for the grab above
        }
    ]

    Timer {
        interval: 300
        repeat: true
        running: true
        onTriggered: {
            if (++check.waited > 60) {
                const panel = check.find("filesPanel", check.shell.contentItem);
                console.error("local cwd check: timeout at step", check.step, "remoteCwd =", JSON.stringify(check.session() ? check.session().remoteCwd : "-"),
                    "pane =", panel && panel.local ? JSON.stringify(panel.local.currentDir()) + " entries " + panel.local.model.count : "-");
                return Qt.exit(1);
            }
            if (check.steps[check.step]() !== false) {
                check.step++;
                check.waited = 0;
            }
        }
    }
}
