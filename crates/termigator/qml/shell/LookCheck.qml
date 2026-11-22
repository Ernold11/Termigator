import QtQuick

// scripts/test-appimage-distros.sh: waits for the first frames, saves the whole window to
// TERMIGATOR_SMOKE_TEST's file (the content: sidebar and panes) and next to it <file>.bar.png (the
// toolbar), then quits.
Item {
    id: root

    required property var shell

    Timer {
        interval: 3000
        running: true
        onTriggered: {
            const file = root.shell.controller.smokeTestShot;
            const bar = root.shell.header.grabToImage(result => {
                result.saveToFile(file.replace(/\.png$/, ".bar.png"));
                const content = root.shell.contentArea.grabToImage(r => Qt.exit(r.saveToFile(file) ? 0 : 2));
                if (!content)
                    Qt.exit(3);
            });
            if (!bar)
                Qt.exit(3);
        }
    }

    // No frame at all: fail instead of hanging.
    Timer {
        interval: 30000
        running: true
        onTriggered: Qt.exit(4)
    }
}
