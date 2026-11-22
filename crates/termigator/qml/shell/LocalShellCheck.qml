import QtQuick

// Smoke-only check: opens the default local shell and saves the window content, so terminal
// rendering (prompt glyphs, colours, alignment) can be inspected without a real click.
Item {
    id: check

    required property var shell
    property int tries: 0

    Timer {
        interval: 300
        repeat: true
        running: true
        onTriggered: {
            if (check.shell.newLocalShell) {
                stop();
                check.shell.newLocalShell("");
                grab.start();
            } else if (++check.tries > 30) {
                console.error("local shell check: no newLocalShell");
                Qt.exit(1);
            }
        }
    }

    Timer {
        id: grab

        interval: 2000
        onTriggered: check.shell.contentArea.grabToImage(result => {
            const ok = result.saveToFile(check.shell.controller.smokeTestShot);
            console.log("local shell check: wrote", check.shell.controller.smokeTestShot, ok);
            Qt.exit(ok ? 0 : 1);
        })
    }
}
