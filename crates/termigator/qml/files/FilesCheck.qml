import QtQuick

// Smoke-only, real host (not the stand-in ssh of FilesSmoke): connects with --ssh-target / --ssh-password, then uploads
// --upload=<local file> into the folder Files shows, and edits it in place (launch.editor appends a line; the save goes back).
// With --download-to=<local path> it then downloads the edited file back. Prints "files check: ..." lines; exits 0 only when both jobs end "done".
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
    readonly property string upload: arg("--upload")
    readonly property string downloadTo: arg("--download-to")
    readonly property string baseName: upload.split("/").pop()

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
    function rowOf(name) {
        for (let i = 0; i < panel.list.list.count; ++i)
            if (JSON.parse(panel.files.rowInfo(i)).name === name)
                return i;
        return -1;
    }
    function jobs() {
        return JSON.parse(panel.transfers.jobsJson());
    }
    function job(part) {
        return jobs().find(j => j.plainLabel.indexOf(part) >= 0) || null;
    }
    function finished(j) {
        return j !== null && j.state !== "queued" && j.state !== "running";
    }
    function bail(why) {
        console.error("files check: FAILED at step " + step + ": " + why);
        Qt.exit(1);
    }

    readonly property var steps: [
        () => {
            const m = /^(?:([^@]+)@)?([^:]+)(?::(\d+))?$/.exec(target);
            shell.openConnection(JSON.stringify({
                label: "files check",
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
            if (panel.files.busy || !panel.files.cwd)
                return false;
            console.log("files check: listing", panel.files.cwd, "status:", panel.files.status);
            panel.uploadPaths([upload], -1);
        },
        () => {
            const j = job(baseName);
            if (!finished(j) || rowOf(baseName) < 0)
                return false;
            console.log("files check: upload", JSON.stringify(j));
            if (j.state !== "done")
                return bail("upload " + j.stateText);
            panel.transfers.clearFinished();
            panel.files.editEntry(rowOf(baseName));
        },
        () => {
            const j = job("(save)");
            if (!finished(j))
                return false;
            console.log("files check: save", JSON.stringify(j), "status:", panel.files.status);
            if (j.state !== "done")
                return bail("save " + j.stateText);
            panel.transfers.clearFinished();
            if (!downloadTo)
                return Qt.exit(0);
            panel.files.downloadEntry(rowOf(baseName), downloadTo);
        },
        () => {
            const j = jobs().find(x => !x.upload) || null;
            if (!finished(j))
                return false;
            console.log("files check: download", JSON.stringify(j), "status:", panel.files.status);
            Qt.exit(j.state === "done" ? 0 : 1);
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
                console.log("files check: step", check.step, "busy", check.panel ? check.panel.files.busy : "-", "status:", check.panel ? check.panel.files.status : "-");
            if (++check.waited > 200)
                return check.bail("timeout, jobs: " + panel.transfers.jobsJson() + " status: " + panel.files.status);
            if (check.steps[check.step]() !== false) {
                check.step++;
                check.waited = 0;
            }
        }
    }
}
