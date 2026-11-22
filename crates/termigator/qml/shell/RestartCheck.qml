import QtQuick

// Smoke-only check (tests/restart_smoke.rs): Preferences → Restart now. The first process opens a
// local shell and restarts; the new one (--restore-session) prints "restart: restored <n>" once the
// tab is back, and quits.
Item {
    id: check

    required property var shell
    readonly property bool second: Qt.application.arguments.includes("--restore-session")

    Timer {
        interval: 300
        repeat: true
        running: true
        property int ticks: 0
        onTriggered: {
            const area = check.shell.panesArea;
            if (!area)
                return;
            if (check.second) {
                if (area.tabList.length > 0) {
                    console.log("restart: restored " + area.tabList.length);
                    Qt.exit(0);
                }
            } else if (ticks === 0) {
                check.shell.newLocalShell("");
            } else if (area.liveCount > 0) {
                running = false;
                check.shell.restart();
            }
            ticks++;
        }
    }

    Timer {
        interval: 30000
        running: true
        onTriggered: {
            console.error("restart: timed out");
            Qt.exit(1);
        }
    }
}
