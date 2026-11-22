import QtQuick

// Smoke-only check: quick-connects over ssh to the target in TERMIGATOR_SSH_TARGET and saves the window content, so terminal
// rendering (prompt glyphs, colours, alignment) can be inspected without a real click.
Item {
    id: check

    required property var shell
    property int tries: 0
    // The target comes from `--ssh-target=user@host` on the command line.
    readonly property string target: Qt.application.arguments.filter(a => a.startsWith("--ssh-target=")).map(a => a.slice(13))[0] || ""

    // Optional `--ssh-password=...` (throwaway test account only): opens a saved-style entry so the password path
    // (sshpass on Unix, askpass on Windows) is exercised instead of quick connect.
    readonly property string password: Qt.application.arguments.filter(a => a.startsWith("--ssh-password="))
        .map(a => a.slice(15))[0] || ""
    // Optional `--ssh-jump=user@bastion:port`: the entry goes through that jump host (proxy_jump).
    readonly property string jump: Qt.application.arguments.filter(a => a.startsWith("--ssh-jump="))
        .map(a => a.slice(11))[0] || ""

    function entry() {
        const m = /^(?:([^@]+)@)?([^:]+)(?::(\d+))?$/.exec(target);
        return JSON.stringify({ label: "pw check", host: m[2], user: m[1] || "", port: Number(m[3] || 22),
            password: password, proxy_jump: jump, terminal: "embedded",
            extra_ssh_args: "-o StrictHostKeyChecking=accept-new" });
    }

    Timer {
        interval: 300
        repeat: true
        running: true
        onTriggered: {
            if (check.shell.quickConnectHandler) {
                stop();
                if ((check.password || check.jump) && check.shell.openConnection)
                    check.shell.openConnection(check.entry());
                else
                    check.shell.quickConnectHandler(check.target);
                grab.start();
            } else if (++check.tries > 30) {
                console.error("ssh check: no newLocalShell");
                Qt.exit(1);
            }
        }
    }

    Timer {
        id: grab

        interval: 6000
        onTriggered: check.shell.contentArea.grabToImage(result => {
            const ok = result.saveToFile(check.shell.controller.smokeTestShot);
            console.log("ssh check: wrote", check.shell.controller.smokeTestShot, ok);
            Qt.exit(ok ? 0 : 1);
        })
    }
}
