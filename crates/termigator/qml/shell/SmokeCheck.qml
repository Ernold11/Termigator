import QtQuick
import QtQuick.Dialogs
import QtQuick.Window
import Termigator.Terminal.Probe

// Smoke-test mode only (TERMIGATOR_SMOKE_TEST, see shell_qobj.rs; driven by tests/smoke.rs): checks
// the shell step by step with real key presses, saves a grab, prints "termigator smoke: ready", waits
// for the test's `toggle` on the control socket, then quits through the close question.
Item {
    id: check

    required property var shell
    property int step: 0
    property bool quitting: false
    property int splitHits: 0
    property var dialog: null
    property var sizedWindow: null

    function fail(reason) {
        console.error("termigator smoke: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok)
            fail(what);
    }

    function press(key, modifiers, text) {
        Probe.key(shell, key, modifiers, text || "");
    }

    function actionOf(id) {
        return shell.shellActions.actions[id];
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

    function clickItem(item) {
        const p = item.mapToItem(null, item.width / 2, item.height / 2);
        Probe.click(shell, p.x, p.y, Qt.LeftButton, 1, 0);
    }

    readonly property var steps: [
        // Menu bar: the order and texts of the action table.
        () => {
            const a = shell.shellActions;
            const bar = shell.menuBar;
            const tops = [];
            for (let i = 0; i < bar.count; ++i)
                tops.push(a.plain(bar.menuAt(i).title));
            expect(tops.join(",") === "File,Edit,View,Session,Tools,Settings,Help", "menus " + tops);
            const file = bar.menuAt(0);
            expect(file.itemAt(0).text === "New &tab", "first File entry " + file.itemAt(0).text);
            expect(a.menus["File/New connection"].count === 8, "New connection entries");
            expect(actionOf("file.quit").shortcut === "Ctrl+Shift+Q", "Quit key");
            expect(actionOf("edit.copy").enabled, "Copy not handled by the terminal");
            expect(actionOf("help.about").enabled, "About disabled");
        },
        // Toolbar: hidden from View, remembered in window.toolbar.
        () => {
            expect(shell.header.visible && actionOf("view.toolbar").checked, "toolbar shown");
            actionOf("view.toolbar").trigger();
            expect(!shell.header.visible && !shell.controller.windowBool("toolbar", true), "toolbar hidden");
            actionOf("view.toolbar").trigger();
            expect(shell.header.visible && actionOf("view.toolbar").checked, "toolbar back");
        },
        // Ctrl+Shift+B and F11 as key presses.
        () => {
            shell.requestActivate();
            press(Qt.Key_B, Qt.ControlModifier | Qt.ShiftModifier);
            expect(!shell.sidebarVisible && !actionOf("view.sidebar").checked, "sidebar hidden");
            press(Qt.Key_B, Qt.ControlModifier | Qt.ShiftModifier);
            expect(shell.sidebarVisible && actionOf("view.sidebar").checked, "sidebar back");
            press(Qt.Key_F11, Qt.NoModifier);
        },
        () => {
            expect(actionOf("view.fullscreen").checked, "full screen checked");
            press(Qt.Key_F11, Qt.NoModifier);
        },
        () => {
            expect(!actionOf("view.fullscreen").checked, "full screen off");
            // An area's handler makes the primary key and the alternative live.
            shell.shellActions.handle("view.split_right", () => ++check.splitHits);
            press(Qt.Key_ParenLeft, Qt.ControlModifier, "(");
            expect(splitHits === 1, "Ctrl+( hits " + splitHits);
            press(Qt.Key_E, Qt.ControlModifier | Qt.ShiftModifier);
            expect(splitHits === 2, "split right hits " + splitHits);
            actionOf("view.split_right").handler = null;
            press(Qt.Key_E, Qt.ControlModifier | Qt.ShiftModifier);
            expect(splitHits === 2, "disabled action still fired");
        },
        // The toolbar's buttons trigger their actions (Files is in FilesSmoke); the arrow next to New opens its menu.
        () => {
            let hits = [];
            for (const id of ["file.new_tab", "view.split_right", "view.split_down", "session.multiexec", "tools.tunnels"]) {
                shell.shellActions.handle(id, () => hits.push(id));
                const btn = findByName("tb_" + id, shell.header);
                if (id === "file.new_tab")
                    clickItem(findByName("tbNew", shell.header));
                else {
                    expect(btn !== null, "toolbar button " + id);
                    clickItem(btn);
                }
                expect(hits.join() === id, "toolbar click on " + id + ": " + hits);
                actionOf(id).handler = null;
                hits = [];
            }
            clickItem(findByName("tbNewMenu", shell.header));
            expect(shell.header.newMenu !== null && shell.header.newMenu.visible, "New arrow opens the menu");
            shell.header.newMenu.close();
        },
        // File → Open config folder hands the config directory to the opener.
        () => {
            const urls = [];
            shell.openUrl = url => urls.push(url.toString());
            actionOf("file.open_config_folder").trigger();
            expect(urls.length === 1 && urls[0].startsWith("file:///"), "open config folder: " + urls);
        },
        // The shortcut list and its search.
        () => {
            actionOf("help.shortcuts").trigger();
            dialog = shell.shortcutsWindow;
            expect(dialog !== null, "no shortcuts dialog");
            expect(dialog.shownCount === JSON.parse(shell.controller.shortcutRowsJson).length, "rows");
            dialog.filter("quit");
            expect(dialog.shownCount === 1, "search quit: " + dialog.shownCount);
            dialog.close();
        },
        // Status messages replace the summary for a while.
        () => {
            shell.listSummary = "3 entries";
            shell.notify("Saved");
            expect(shell.footer.contentChildren[0].text === "Saved", "status message");
            shell.statusMessage = "";
            expect(shell.footer.contentChildren[0].text === "3 entries", "summary back");
        },
        // A dialog size survives in window.dialog_sizes.
        () => {
            sizedWindow = sizedComponent.createObject(check);
            sizedWindow.show();
            sizedWindow.width = 300;
            sizedWindow.height = 222;
            sizedWindow.hide();
            const s = shell.controller.dialogSize("SmokeDialog");
            expect(s.width === 300 && s.height === 222, "stored " + s.width + "x" + s.height);
            sizedWindow.destroy();
            const again = sizedComponent.createObject(check);
            expect(again.width === 300 && again.height === 222, "restored " + again.width);
            again.destroy();
        },
        // The close question (confirm_close) keeps the window open until answered.
        () => {
            shell.liveSessions = 1;
            shell.requestQuit();
        },
        () => {
            expect(shell.visible && shell.closeBox.visible, "close question not asked");
            expect(shell.closeBox.text.startsWith("1 embedded session(s) are still running"), shell.closeBox.text);
            shell.closeBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            shell.closeBox.close();
        },
        // The tray's Quit asks too, also with the window hidden in the tray, and
        // Cancel keeps Termigator running there.
        () => {
            expect(shell.visible, "closed although answered No");
            shell.shuttingDown.connect(() => check.quitting = true);
            shell.hide();
            shell.trayQuitItem.triggered();
        },
        () => {
            expect(shell.closeBox.visible, "the tray's Quit did not ask");
            shell.closeBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            shell.closeBox.close();
        },
        () => {
            expect(!check.quitting && !shell.visible, "Cancel in the tray's Quit: quitting " + check.quitting + ", visible " + shell.visible);
            shell.show();
        },
        () => {
            expect(shell.visible, "the window did not come back");
            const started = shell.contentArea.grabToImage(result => {
                if (!result.saveToFile(shell.controller.smokeTestShot))
                    check.fail("cannot save the grab");
                console.info("termigator smoke: ready");
            });
            if (!started)
                fail("grab not started");
        }
    ]

    Component {
        id: sizedComponent

        Window {
            id: sized

            width: 200
            height: 150

            DialogSize {
                window: sized
                name: "SmokeDialog"
                controller: check.shell.controller
            }
        }
    }

    Timer {
        interval: 150
        repeat: true
        running: check.step < check.steps.length
        onTriggered: {
            check.steps[check.step]();
            ++check.step;
        }
    }

    // The test's `termigator --toggle` equivalent: answer Yes and quit while the window may be hidden
    // (toggle hides an active window), which must still save the geometry.
    Connections {
        target: check.shell.controller

        function onRemoteCommand(command) {
            console.info("termigator smoke: remote=" + command);
            check.shell.requestQuit();
            Qt.callLater(() => {
                if (!check.shell.closeBox.visible)
                    check.fail("no close question for the final quit");
                // What a click on Yes delivers (the dialog may be a native QMessageBox).
                check.shell.closeBox.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
                check.shell.closeBox.close();
            });
        }
    }
}
