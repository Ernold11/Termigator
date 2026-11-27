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
    property bool wasMaximized: false
    property int splitHits: 0
    property int waitTicks: 0
    readonly property string retry: "retry" // a step returns this to run again on the next tick
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
        // Menu bar: hidden by default, its menus behind the toolbar's ☰ (KHamburgerMenu layout);
        // Ctrl+Shift+M shows it (window.menubar) and the ☰ goes.
        () => {
            const menuButton = findByName("tbMenu", shell.header);
            expect(!shell.menuBar.visible && !actionOf("view.menubar").checked && menuButton.visible, "menu bar hidden, ☰ shown");
            menuButton.clicked();
        },
        () => {
            const all = shell.header.mainMenu;
            const label = it => it.subMenu ? shell.shellActions.plain(it.subMenu.title) + "›" : !it.text ? "-" : shell.shellActions.plain(it.text);
            const rows = m => {
                const out = [];
                for (let i = 0; i < m.count; ++i)
                    out.push(label(m.itemAt(i)));
                return out.join(",");
            };
            expect(all.visible && rows(all) === "Duplicate tab,Reconnect,Log output to file,-,Full screen,Show menubar,Zoom in,Reset zoom,Zoom out,-,Session›,Tools›,Settings›,Help›,-,More›", "☰ " + rows(all));
            const more = all.menuAt(all.count - 1);
            expect(rows(more) === "File›,Edit›,View›", "More " + rows(more));
            expect(all.menuAt(13).icon.name !== "" && more.menuAt(0).icon.name !== "", "submenu icons " + all.menuAt(13).icon.name);
            // Nothing the toolbar or the top of ☰ already shows.
            const session = rows(all.menuAt(10));
            expect(rows(more.menuAt(0)).indexOf("New tab") < 0 && session.indexOf("Rename tab") >= 0 && session.indexOf("MultiExec") < 0 && session.indexOf("Duplicate tab") < 0, "repeated entries " + session);
            all.close();
            waitTicks = 0;
        },
        () => {
            if (shell.header.mainMenu.visible && waitTicks++ < 40)
                return retry;
            shell.requestActivate();
            press(Qt.Key_M, Qt.ControlModifier | Qt.ShiftModifier);
            expect(shell.menuBar.visible && actionOf("view.menubar").checked && !findByName("tbMenu", shell.header).visible, "Ctrl+Shift+M shows the menu bar");
            expect(shell.controller.windowBool("menubar", false), "window.menubar not stored");
            press(Qt.Key_M, Qt.ControlModifier | Qt.ShiftModifier);
            expect(!shell.menuBar.visible && !shell.controller.windowBool("menubar", true), "Ctrl+Shift+M hides it");
        },
        // A lone Alt (Windows; TERMIGATOR_ALT_MENU here) shows the bar with its first menu open, a
        // second Alt closes both; Ctrl+Alt (AltGr on Windows) does nothing.
        () => {
            shell.requestActivate();
            press(Qt.Key_Alt, Qt.AltModifier);
            waitTicks = 0;
        },
        () => {
            if (shell.openMenuBarMenus().length === 0 && waitTicks++ < 20)
                return retry;
            expect(shell.menuBar.visible && shell.menubarPeek && shell.menuBar.menuAt(0).visible, "Alt opens the first menu");
            expect(!shell.controller.windowBool("menubar", true), "Alt changed window.menubar");
            press(Qt.Key_Alt, Qt.AltModifier);
            waitTicks = 0;
        },
        () => {
            if (shell.menubarPeek && waitTicks++ < 20)
                return retry;
            expect(!shell.menuBar.visible && shell.openMenuBarMenus().length === 0, "second Alt closes the menu and the bar");
            press(Qt.Key_Alt, Qt.ControlModifier | Qt.AltModifier);
        },
        () => {
            expect(!shell.menubarPeek && shell.openMenuBarMenus().length === 0, "Ctrl+Alt (AltGr) opened a menu");
        },
        // Toolbar: hidden from View, remembered in window.toolbar; its ☰ goes with it, so the menu
        // bar comes back (one of the two always shows).
        () => {
            expect(shell.header.visible && actionOf("view.toolbar").checked, "toolbar shown");
            actionOf("view.toolbar").trigger();
            expect(!shell.header.visible && !shell.controller.windowBool("toolbar", true), "toolbar hidden");
            expect(shell.menuBar.visible && shell.controller.windowBool("menubar", false), "menu bar with the toolbar hidden");
            actionOf("view.toolbar").trigger();
            expect(shell.header.visible && actionOf("view.toolbar").checked, "toolbar back");
            actionOf("view.menubar").trigger();
            expect(!shell.menuBar.visible && shell.header.visible, "menu bar hidden again");
            // Hiding the menu bar while the toolbar is hidden brings the toolbar back.
            actionOf("view.toolbar").trigger();
            actionOf("view.menubar").trigger();
            expect(!shell.menuBar.visible && shell.header.visible && shell.controller.windowBool("toolbar", false), "toolbar back for its ☰");
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
        // window.close_to_tray without a tray icon: the close button still asks to quit (the
        // window cannot be stranded hidden).
        () => {
            shell.trayReady = false;
            shell.controller.setWindowBool("close_to_tray", true);
            shell.close();
        },
        () => {
            expect(shell.visible && shell.closeBox.visible, "close to tray without a tray did not ask");
            shell.closeBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            shell.closeBox.close();
        },
        // With the tray: the close button hides the window, nothing asks or shuts down; the
        // tray menu's Show / Restore brings it back maximized as it was.
        () => {
            shell.trayReady = true;
            shell.showMaximized();
        },
        () => {
            check.wasMaximized = shell.visibility === Window.Maximized;
            expect(shell.trayHideItem.enabled && !shell.trayShowItem.enabled, "tray items while shown");
            shell.close();
        },
        () => {
            expect(!shell.visible && !shell.closeBox.visible && !check.quitting, "close to tray: visible " + shell.visible + ", asked " + shell.closeBox.visible + ", quitting " + check.quitting);
            expect(shell.trayShowItem.enabled && !shell.trayHideItem.enabled, "tray items while hidden");
            shell.trayShowItem.triggered();
        },
        () => {
            expect(shell.visible, "Show / Restore did not bring the window back");
            if (check.wasMaximized)
                expect(shell.visibility === Window.Maximized, "not maximized again: " + shell.visibility);
            shell.trayHideItem.triggered();
        },
        () => {
            expect(!shell.visible && !check.quitting, "the tray's Hide");
            shell.restoreWindow();
            shell.showNormal();
        },
        () => {
            const g = Qt.rect(shell.x, shell.y, shell.width, shell.height);
            shell.close();
            shell.restoreWindow();
            expect(shell.visible && shell.width === g.width && shell.height === g.height, "size after the tray: " + shell.width + "x" + shell.height + " for " + g.width + "x" + g.height);
            shell.controller.setWindowBool("close_to_tray", false);
            shell.trayReady = false;
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
            if (check.steps[check.step]() !== check.retry)
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
