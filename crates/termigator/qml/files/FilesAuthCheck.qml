import QtQuick

// Smoke-only, real host (Windows): connects to --ssh-target with no saved password (the terminal
// asks on its own), so Files is refused; types a wrong password first, then --ssh-password through
// the panel's "Enter password…" prompt, and expects a listing. Prints "files auth check: ..." lines;
// exits 0 only when the listing comes with the right password.
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
    function bail(why) {
        console.error("files auth check: FAILED at step " + step + ": " + why);
        Qt.exit(1);
    }
    function type(pw) {
        panel.askPassword();
        if (!(panel.prompt.visible && panel.prompt.secret))
            return bail("no masked password prompt");
        panel.prompt.field.text = pw;
        panel.prompt.accept();
    }

    readonly property var steps: [
        () => {
            const m = /^(?:([^@]+)@)?([^:]+)(?::(\d+))?$/.exec(target);
            shell.openConnection(JSON.stringify({
                label: "files auth check",
                host: m[2],
                user: m[1] || "",
                port: Number(m[3] || 22),
                terminal: "embedded",
                extra_ssh_args: "-o StrictHostKeyChecking=accept-new"
            }));
            panel = find("filesPanel", shell.contentItem);
            if (!panel)
                return bail("no Files panel");
            shell.sideTabBar.currentIndex = 1;
        },
        () => {
            if (!panel.files.authRequired)
                return false;
            console.log("files auth check: refused, status:", panel.files.status);
            if (!find("filesPasswordButton", panel).visible)
                return bail("no password button");
            type("definitely-wrong");
        },
        () => {
            if (panel.files.busy || !panel.files.authRequired && !panel.files.cwd)
                return false;
            console.log("files auth check: after a wrong password authRequired", panel.files.authRequired, "cwd", JSON.stringify(panel.files.cwd), "status:", panel.files.status);
            if (!panel.files.authRequired)
                return bail("a wrong password was accepted");
            type(password);
        },
        () => {
            if (panel.files.busy || panel.files.authRequired || !panel.files.cwd)
                return false;
            console.log("files auth check: listed", panel.files.cwd, "rows", panel.list.list.count, "field empty", panel.prompt.field.text === "");
            Qt.exit(panel.list.list.count > 0 ? 0 : 1);
        }
    ]

    Timer {
        interval: 200
        repeat: true
        running: true
        onTriggered: {
            if (check.step === 0 && !check.shell.openConnection)
                return;
            if (check.waited % 25 === 5)
                console.log("files auth check: step", check.step, "busy", check.panel ? check.panel.files.busy : "-", "auth", check.panel ? check.panel.files.authRequired : "-", "status:", check.panel ? check.panel.files.status : "-");
            if (++check.waited > 200)
                return check.bail("timeout, status: " + panel.files.status);
            if (check.steps[check.step]() !== false) {
                check.step++;
                check.waited = 0;
            }
        }
    }
}
