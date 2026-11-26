import QtQuick
import QtQuick.Dialogs
import QtQuick.Window

// Smoke-only check (tests/close_choice_smoke.rs) of the close button's choice (window.close_action)
// with the real system dialog. Offscreen has no tray, so trayReady is set by hand. Modes:
//   (none)                 ask: the dialog, a repeated close, Cancel / Esc, Hide in Tray with
//                          and without "Remember my choice", Settings back to "Always ask", then
//                          Exit remembered (quits)
//   --close-remembered     the stored choice of an earlier run: no dialog
//   --close-session-end    logout with the window hidden in the tray: the shutdown still runs
//   --close-session-end-visible  logout with the window shown: closes without asking
// It prints "termigator close: shutting down …" from shuttingDown(), "termigator close: ok" before
// a Qt.exit(0) of its own, and "termigator close: FAILED …" before Qt.exit(1).
Item {
    id: check

    required property var shell
    readonly property var args: Qt.application.arguments
    readonly property string mode: args.includes("--close-remembered") ? "remembered" : args.includes("--close-session-end") ? "session-end" : args.includes("--close-session-end-visible") ? "session-end-visible" : "ask"
    property int step: 0
    property int waited: 0
    property bool quitting: false
    property var box: null
    property var dlg: null

    function fail(reason) {
        console.error("termigator close: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok)
            fail(what);
    }

    function findByName(name, item) {
        if (item.objectName === name)
            return item;
        for (const c of item.children) {
            const f = findByName(name, c);
            if (f)
                return f;
        }
        return null;
    }

    // The tray icons in the window (a SystemTrayIcon is the only child with showMessage).
    function trayIcons() {
        let n = 0;
        for (const o of shell.contentData) {
            if (o && typeof o.showMessage === "function" && o.supportsMessages !== undefined)
                ++n;
        }
        return n;
    }

    // The tray icon shows the app's own image: a theme name, or a file / resource that loads.
    function trayIconLoads() {
        for (const o of shell.contentData) {
            if (!(o && typeof o.showMessage === "function" && o.supportsMessages !== undefined))
                continue;
            if (o.icon.name !== "")
                return o.icon.name === "termigator";
            const img = Qt.createQmlObject('import QtQuick; Image { asynchronous: false }', check);
            img.source = o.icon.source;
            const ok = img.status === Image.Ready;
            img.destroy();
            return ok;
        }
        return false;
    }

    function liveCount() {
        return shell.panesArea ? shell.panesArea.liveCount : 0;
    }

    // Opens the dialog through a close request and returns the system dialog shown.
    function expectDialog(what) {
        const d = shell.closeChoiceBox.current;
        expect(shell.closeChoiceBox.visible && d !== null && d.native === true, what + ": no system close dialog");
        return d;
    }

    // A step returning false runs again on the next tick (at most 100 times).
    readonly property var askSteps: [
        () => {
            expect(trayIcons() === 1, "tray icons: " + trayIcons());
            expect(trayIconLoads(), "the tray icon is not the app icon");
            expect(shell.controller.closeAction() === "ask", "default " + shell.controller.closeAction());
            shell.newLocalShell("");
        },
        () => liveCount() > 0,
        // 1. The close button asks.
        () => shell.close(),
        () => {
            box = expectDialog("close in ask mode");
            expect(shell.visible && !quitting, "closed before an answer");
            expect(shell.closeChoiceBox.title === "Close Termigator?", "title " + shell.closeChoiceBox.title);
            expect(box.text === "Hide Termigator in the system tray, or exit?", box.text);
            expect(box.informativeText.startsWith("1 embedded session(s) are still running"), "what exit ends: " + box.informativeText);
            expect(box.buttonText(MessageDialog.Ok) === "Hide in Tray", "ok text " + box.buttonText(MessageDialog.Ok));
            expect(box.buttonText(MessageDialog.Close) === "Exit", "close text " + box.buttonText(MessageDialog.Close));
            expect(box.buttonText(MessageDialog.Cancel) !== "", "no Cancel");
            // Enter keeps everything running: never Exit by accident.
            expect(box.shownDefaultButton() === MessageDialog.Ok, "default button " + box.shownDefaultButton());
            expect(box.checkBoxText === "Remember my choice (don't ask again)" && !box.checked, "check box");
            // 11. Another close request (the X again, Alt+F4) while it is open: the same dialog.
            shell.close();
            shell.close();
        },
        () => {
            expect(shell.closeChoiceBox.current === box && box.visible, "a repeated close replaced the dialog");
            expect(!shell.closeBox.visible, "a second question opened");
            // 2. Cancel (what Esc and the dialog's own close button press).
            box.dismiss();
        },
        () => {
            expect(!shell.closeChoiceBox.visible && shell.visible && !quitting, "Cancel: visible " + shell.visible + ", quitting " + quitting);
            expect(shell.controller.closeAction() === "ask", "Cancel stored " + shell.controller.closeAction());
            shell.close();
        },
        // 3. Hide in Tray hides the window, the session keeps running; nothing is stored.
        () => expectDialog("second close").click(MessageDialog.Ok),
        () => {
            expect(!shell.visible && !quitting, "Hide in Tray: visible " + shell.visible + ", quitting " + quitting);
            expect(liveCount() === 1, "session ended in the tray: " + liveCount());
            expect(shell.controller.closeAction() === "ask", "stored without Remember: " + shell.controller.closeAction());
            shell.trayShowItem.triggered();
        },
        () => {
            expect(shell.visible && trayIcons() === 1, "back from the tray: visible " + shell.visible + ", icons " + trayIcons());
            shell.close();
        },
        // 5. Remember my choice + Hide in Tray.
        () => {
            box = expectDialog("close after the tray");
            expect(!box.checked, "Remember left ticked from before");
            box.checked = true;
            box.click(MessageDialog.Ok);
        },
        () => {
            expect(!shell.visible && !quitting && liveCount() === 1, "remembered tray: visible " + shell.visible + ", live " + liveCount());
            expect(shell.controller.closeAction() === "tray", "remembered " + shell.controller.closeAction());
            shell.restoreWindow();
        },
        () => {
            expect(shell.visible, "restore");
            shell.close();
        },
        () => {
            expect(!shell.visible && !shell.closeChoiceBox.visible && !quitting, "tray without asking: visible " + shell.visible + ", asked " + shell.closeChoiceBox.visible);
            shell.restoreWindow();
        },
        // 8. Settings → Close behavior back to "Always ask"; works without a restart.
        () => {
            shell.openPreferences();
            dlg = shell.preferencesDialog;
        },
        () => {
            const combo = findByName("close_action", dlg.contentItem);
            expect(combo !== null && combo.enabled, "no close behavior combo");
            expect(combo.currentValue === "tray", "settings show " + combo.currentValue);
            dlg.edit("close_action", "ask");
            dlg.apply();
            dlg.close();
        },
        () => {
            expect(shell.controller.closeAction() === "ask", "settings stored " + shell.controller.closeAction());
            shell.close();
        },
        // 6. Remember my choice + Exit: quits (shuttingDown prints the stored choice).
        () => {
            box = expectDialog("close after Settings");
            box.checked = true;
            box.click(MessageDialog.Close);
        },
        () => fail("Exit did not quit")
    ]

    readonly property var rememberedSteps: [
        () => {
            expect(!shell.closeChoiceBox.visible, "asked at start");
            shell.close();
        },
        () => {
            expect(!shell.closeChoiceBox.visible, "asked although remembered");
            if (shell.controller.closeAction() === "tray") {
                expect(!shell.visible && !quitting, "remembered tray did not hide");
                console.info("termigator close: ok");
                Qt.exit(0);
            }
        },
        () => fail("remembered " + shell.controller.closeAction() + " did nothing")
    ]

    readonly property var sessionEndSteps: [
        () => {
            shell.hideToTray();
        },
        () => {
            expect(!shell.visible, "not hidden");
            shell.sessionEnd.simulate();
            Qt.quit();
        },
        () => fail("did not quit")
    ]

    readonly property var sessionEndVisibleSteps: [
        () => {
            shell.sessionEnd.simulate();
            shell.close();
        },
        () => fail("closing at a session end did not quit")
    ]

    readonly property var steps: mode === "remembered" ? rememberedSteps : mode === "session-end" ? sessionEndSteps : mode === "session-end-visible" ? sessionEndVisibleSteps : askSteps

    Component.onCompleted: {
        shell.trayReady = true;
        shell.shuttingDown.connect(() => {
            check.quitting = true;
            console.info("termigator close: shutting down action=" + shell.controller.closeAction() + " asked=" + shell.closeChoiceBox.visible + " live=" + liveCount());
        });
    }

    Timer {
        interval: 150
        repeat: true
        running: check.step < check.steps.length && !check.quitting
        onTriggered: {
            if (check.steps[check.step]() === false && ++check.waited < 100)
                return;
            check.waited = 0;
            ++check.step;
        }
    }

    Timer {
        interval: 45000
        running: true
        onTriggered: check.fail("timed out")
    }
}
