import QtQuick

// Smoke-only: opens a local shell and asks to close it (the "still running" question), so the
// message box sound runs; with QT_LOGGING_RULES="termigator.sound.debug=true" the log says what
// played. Then rings the bell once and quits.
Item {
    id: check

    required property var shell
    property int step: 0

    Timer {
        interval: 1500
        repeat: true
        running: true
        onTriggered: {
            const area = check.shell.panesArea;
            if (check.step === 0) {
                check.shell.newLocalShell("cmd");
            } else if (check.step === 1) {
                console.log("sound check: closing", area.activeTab ? area.activeTab.title : "-");
                area.closeTab(area.activeTab);
            } else if (check.step === 2) {
                const s = area.activeTab ? area.activeTab.session : null;
                if (s)
                    s.core.bell(true, true);
                console.log("sound check: bell rung", s !== null);
            } else {
                Qt.exit(0);
            }
            check.step++;
        }
    }
}
