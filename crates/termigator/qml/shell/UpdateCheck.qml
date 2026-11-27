import QtQuick
import QtQuick.Dialogs
import Termigator

// Smoke-only check (tests/update_smoke.rs). TERMIGATOR_UPDATE_URL points at a local "latest release"
// JSON for 9.9.9. With window.check_updates on: the banner shows 9.9.9, "Skip this version" hides it
// and is remembered; the welcome box's check box (a real QMessageBox) reports its state. With it off:
// no banner. Prints "update: shown" / "update: none".
Item {
    id: check

    required property var shell
    readonly property bool checking: check.shell.controller.windowBool("check_updates", true)

    property bool failed: false

    // Qt.exit() only queues the exit: callers return right after, and nothing passes once failed.
    function fail(msg) {
        console.error("update: " + msg);
        failed = true;
        Qt.exit(1);
    }

    MessageBox {
        id: box

        text: "check box"
        checkBoxText: "Check for new versions"
        checked: true
    }

    Timer {
        interval: 200
        repeat: true
        running: true
        property int ticks: 0
        onTriggered: {
            ticks++;
            const b = check.shell.updates;
            if (!check.checking) {
                if (b.version !== "" || check.shell.updateStatus.latest !== "") {
                    running = false;
                    check.fail("update shown although check_updates is off: " + b.version);
                } else if (ticks > 30) { // 6 s: well past the 3 s start delay
                    console.log("update: none");
                    Qt.exit(0);
                }
                return;
            }
            if (b.version === "")
                return;
            running = false;
            if (b.version !== "9.9.9" || b.url !== "https://github.com/Ernold11/Termigator/releases/tag/v9.9.9")
                return check.fail("wrong release " + b.version + " " + b.url);
            if (!b.visible)
                return check.fail("banner not visible");
            // The status bar's corner offers it too, linked to its page.
            const st = check.shell.updateStatus;
            if (!st.newer || st.latest !== "9.9.9" || st.url !== b.url)
                return check.fail("status bar " + JSON.stringify(st));
            console.log("update: shown " + b.version);
            // A picture of the banner for a look (tests keep it next to the config).
            check.shell.contentArea.grabToImage(result => {
                console.log("update: grab", result.saveToFile(check.shell.controller.smokeTestShot));
                check.afterBanner(b);
            });
        }
    }

    function afterBanner(b) {
        shell.controller.skipUpdate(b.version);
        b.version = "";
        if (shell.controller.updateCheckResult('{"tag_name":"v9.9.9","html_url":"https://github.com/x/y"}') !== "")
            return fail("skipped version offered again");

        box.open();
        const d = box.current;
        if (!d || d.dialogClass !== "QMessageBox")
            return fail("welcome box is not a QMessageBox");
        d.checked = false; // unticks the real check box
        d.click(MessageDialog.Ok);
        if (box.checked !== false)
            return fail("check box state not reported");
        console.log("update: checkbox ok");
        Qt.exit(0);
    }

    Timer {
        interval: 30000
        running: true
        onTriggered: {
            if (!check.failed)
                check.fail("timed out");
        }
    }
}
