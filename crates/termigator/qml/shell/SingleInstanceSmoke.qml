import QtQuick
import QtQuick.Window

// Single-instance offscreen check (tests/single_instance_smoke.rs). Opens a local shell, then for
// each window state (visible, minimized, hidden in the tray) prints "termigator single: ready
// <state>" and waits for the `show` a second start hands over; the window must come back with the
// shell still running. --single-quick: only "termigator single: primary" (this start got the
// window) and quit; --single-once: the visible state only.
Item {
    id: check

    required property var shell
    readonly property bool quick: Qt.application.arguments.indexOf("--single-quick") >= 0
    readonly property bool once: Qt.application.arguments.indexOf("--single-once") >= 0
    property var states: once ? ["visible"] : ["visible", "minimized", "tray"]
    property int index: -1 // -1: opening the shell
    property bool waiting: false
    property int ticks: 0

    function fail(reason) {
        console.error("termigator single: FAILED (" + (index >= 0 ? states[index] : "setup") + "): " + reason);
        Qt.exit(1);
    }

    function enter(state) {
        if (state === "minimized")
            shell.showMinimized();
        else if (state === "tray")
            shell.hideWindow();
    }

    function entered(state) {
        if (state === "minimized")
            return shell.visibility === Window.Minimized;
        if (state === "tray")
            return !shell.visible;
        return shell.visible;
    }

    Component.onCompleted: {
        console.info("termigator single: primary");
        if (quick)
            Qt.exit(0);
        else
            shell.newLocalShell("");
    }

    Timer {
        interval: 100
        repeat: true
        running: !check.quick
        onTriggered: {
            if (check.waiting)
                return;
            if (++check.ticks > 300)
                check.fail("timed out");
            if (check.index < 0) {
                if (check.shell.liveSessions === 1)
                    check.index = 0;
                return;
            }
            const state = check.states[check.index];
            if (!check.entered(state)) {
                check.enter(state);
                return;
            }
            check.waiting = true;
            console.info("termigator single: ready " + state);
        }
    }

    // The shell's own handler (ShellWindow.onRemoteCommand) restores the window; look a little later.
    Timer {
        id: verify

        interval: 300
        onTriggered: {
            const state = check.states[check.index];
            if (!check.shell.visible || check.shell.visibility === Window.Minimized)
                check.fail("not restored: visible " + check.shell.visible + ", visibility " + check.shell.visibility);
            if (!check.shell.active)
                check.fail("no focus");
            if (check.shell.liveSessions !== 1)
                check.fail("live sessions " + check.shell.liveSessions);
            console.info("termigator single: shown " + state);
            if (++check.index >= check.states.length) {
                console.info("termigator single: done");
                Qt.exit(0);
            }
            check.ticks = 0;
            check.waiting = false;
        }
    }

    Connections {
        target: check.shell.controller

        function onRemoteCommand(command) {
            console.info("termigator single: remote=" + command);
            // Several starts at once: the first show is checked, the others only logged.
            if (!check.waiting)
                return;
            if (command !== "show")
                check.fail("command " + command);
            else
                verify.restart();
        }
    }
}
