import QtQuick

// window.follow_konsole offscreen check (tests/follow_smoke.rs): with a stub `xdotool` first in
// PATH the window must end up under the "terminal window" (centred, 2 px below); with
// --follow-off (follow_konsole false) the timer must not run and the window must stay put.
Item {
    id: check

    required property var shell
    readonly property bool off: Qt.application.arguments.indexOf("--follow-off") >= 0
    property int ticks: 0
    property int startX: -1
    property int startY: -1

    function fail(reason) {
        console.error("termigator follow: FAILED: " + reason);
        Qt.exit(1);
    }

    Component.onCompleted: {
        startX = shell.x;
        startY = shell.y;
        if (off && shell.followTimer.running)
            fail("timer runs although follow_konsole is off");
        if (!off && !shell.followTimer.running)
            fail("timer not running");
        if (!off && shell.followTimer.interval !== 100)
            fail("interval " + shell.followTimer.interval);
    }

    // The stub reports a terminal window of 700x400 at (50, 60).
    Timer {
        interval: 100
        running: true
        repeat: true
        onTriggered: {
            check.ticks += 1;
            if (check.off) {
                if (check.shell.x !== check.startX || check.shell.y !== check.startY)
                    check.fail("moved to " + check.shell.x + "," + check.shell.y);
                else if (check.ticks >= 15)
                    Qt.exit(0);
                return;
            }
            const wantX = 50 + Math.max(0, Math.floor((700 - check.shell.width) / 2));
            if (check.shell.x === wantX && check.shell.y === 462) {
                console.info("termigator follow: ok");
                Qt.exit(0);
            } else if (check.ticks >= 100) {
                check.fail("window at " + check.shell.x + "," + check.shell.y + ", want " + wantX + ",462");
            }
        }
    }
}
