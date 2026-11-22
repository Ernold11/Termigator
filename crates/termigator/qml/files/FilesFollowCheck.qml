import QtQuick

// Smoke-only, real host: connects with --ssh-target / --ssh-password, types `cd <--cd>` into the terminal and checks that
// the terminal reports the folder (OSC 7, session.remoteCwd) and that Files follows it. Exit 0 only when both happen.
Item {
    id: check

    required property var shell
    property var panel: null
    property int step: 0
    property int waited: 0
    function arg(name) {
        return Qt.application.arguments.filter(a => a.startsWith(name + "=")).map(a => a.slice(name.length + 1))[0] || "";
    }
    readonly property string target: arg("--ssh-target")
    readonly property string password: arg("--ssh-password")
    readonly property string dir: arg("--cd")

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
    function bail(why) {
        const s = session();
        console.error("follow check: FAILED at step " + step + ": " + why + " remoteCwd=" + (s ? s.remoteCwd : "-") + " files.cwd=" + (panel ? panel.files.cwd : "-"));
        Qt.exit(1);
    }

    readonly property var steps: [
        () => {
            const m = /^(?:([^@]+)@)?([^:]+)(?::(\d+))?$/.exec(target);
            shell.openConnection(JSON.stringify({
                label: "follow check",
                host: m[2],
                user: m[1] || "",
                port: Number(m[3] || 22),
                password: password,
                terminal: "embedded",
                extra_ssh_args: "-o StrictHostKeyChecking=accept-new"
            }));
            panel = find("filesPanel", shell.contentItem);
            if (!panel)
                return bail("no Files panel");
            shell.sideTabBar.currentIndex = 1;
        },
        () => {
            if (panel.files.busy || !panel.files.cwd || !session())
                return false;
            console.log("follow check: start folder", panel.files.cwd, "remoteCwd", session().remoteCwd);
            session().ext.sendText("cd " + dir + "\n");
        },
        () => {
            const s = session();
            if (s.remoteCwd !== dir)
                return false;
            console.log("follow check: terminal reports", s.remoteCwd);
        },
        () => {
            if (panel.files.busy || panel.files.cwd !== dir)
                return false;
            console.log("follow check: Files follows to", panel.files.cwd);
            Qt.exit(0);
        }
    ]

    Timer {
        interval: 200
        repeat: true
        running: true
        onTriggered: {
            if (check.step === 0 && !check.shell.openConnection)
                return;
            if (++check.waited > 150)
                return check.bail("timeout");
            if (check.steps[check.step]() !== false) {
                check.step++;
                check.waited = 0;
            }
        }
    }
}
