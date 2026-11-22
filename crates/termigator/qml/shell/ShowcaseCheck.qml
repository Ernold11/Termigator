import QtQuick

// scripts/readme-screenshots.sh: builds the README scenes with the demo profile and keeps the window
// open while the script takes the screenshots. Prints "showcase: <scene>" when a scene is ready;
// quits by itself after --hold=<s> (default 120).
// Scenes: "hosts" (folders expanded, a terminal split in two), "files" (the Files tab).
Item {
    id: check

    required property var shell
    property int step: 0
    property int waited: 0
    property var demo: []

    function arg(name, fallback) {
        return Qt.application.arguments.filter(a => a.startsWith(name + "=")).map(a => a.slice(name.length + 1))[0] || fallback;
    }
    function findItem(pred, item) {
        if (pred(item))
            return item;
        for (const c of item.children) {
            const f = findItem(pred, c);
            if (f)
                return f;
        }
        return null;
    }
    function session() {
        const t = shell.panesArea ? shell.panesArea.activeTab : null;
        return t ? t.session : null;
    }
    function hostsPanel() {
        return findItem(i => i.hosts !== undefined && typeof i.toggle === "function", shell.contentItem);
    }
    // Expands every folder; nested ones show up after their parent opens, hence a few passes.
    function expandAll(panel) {
        for (let pass = 0; pass < 4; pass++)
            for (let r = panel.list.count - 1; r >= 0; r--) {
                const i = panel.info(r);
                if (i && i.kind === "group" && !i.expanded)
                    panel.toggle(r);
            }
    }

    readonly property var steps: [
        () => {
            const panel = hostsPanel();
            if (!shell.newLocalShell || !panel || panel.list.count === 0)
                return false;
            expandAll(panel);
            shell.newLocalShell("");
        },
        () => {
            if (!session())
                return false;
            session().ext.sendText("clear; ls -C --color=always ~/projects; echo; git --no-pager -C ~/projects/web-api log --graph --oneline --color=always\n");
            shell.panesArea.split(false);
        },
        () => {
            if (shell.panesArea.paneCount !== 2)
                return false;
            shell.newLocalShell("");
        },
        () => {
            if (!session())
                return false;
            session().ext.sendText("clear; for c in 31 32 33 34 35 36; do printf '\\e[%sm%s\\e[0m  ' $c colour$c; done; echo; cat ~/notes.txt\n");
        },
        () => {
            shell.sideTabBar.currentIndex = 0; // a local shell switches the side dock to Files
            shell.requestActivate();
            console.log("showcase: hosts");
        },
        () => {
            if (++check.waited < 50) // time for the script's screenshot (15 s)
                return false;
            shell.sideTabBar.currentIndex = 1;
        },
        () => {
            const panel = findItem(i => i.objectName === "filesPanel", shell.contentItem);
            if (!panel || !panel.local || panel.local.model.count === 0)
                return false;
            console.log("showcase: files");
        },
        () => false
    ]

    Timer {
        interval: 300
        repeat: true
        running: true
        onTriggered: {
            if (check.steps[check.step]() !== false) {
                check.step++;
                check.waited = 0;
            }
        }
    }

    Timer {
        interval: 1000 * Number(check.arg("--hold", "120"))
        running: true
        onTriggered: Qt.exit(0)
    }
}
