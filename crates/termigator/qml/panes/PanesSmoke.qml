import QtQuick
import QtQuick.Dialogs
import QtQuick.Window
import Termigator.Terminal.Probe

// Offscreen check (tests/panes_smoke.rs). First run: tabs through shell.openInner, splits,
// the active pane, rename, tab keys, menus, duplicate / reconnect, detach and attach with the
// detached window's keys, quick connect and a favorite, closing; then a grab and a quit, which
// writes `last_session`. With
// --u3-restore (second run on the saved config): the layout and tabs come back.
Item {
    id: check

    required property var shell
    readonly property bool restoreRun: Qt.application.arguments.indexOf("--u3-restore") >= 0
    readonly property bool stressRun: Qt.application.arguments.indexOf("--u3-stress") >= 0
    property var area: null
    property var win: null
    property int step: 0
    property int waitTicks: 0
    readonly property string retry: "retry" // a step returns this to run again on the next tick
    property bool failed: false
    property int closings: 0
    property int reopened: 0
    readonly property string longTitle: "beta and a title longer than thirty-two characters"

    function fail(reason) {
        failed = true;
        console.error("termigator u3: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok && !failed)
            fail(what);
    }

    function press(window, key, modifiers, text) {
        Probe.key(window, key, modifiers, text || "");
    }

    function findArea(item) {
        if (item.objectName === "panesArea")
            return item;
        for (const c of item.children) {
            const f = findArea(c);
            if (f)
                return f;
        }
        return null;
    }

    function tree() {
        return JSON.parse(area.controller.tree());
    }

    // Pane views in the tree's depth-first order.
    function panes() {
        const out = [];
        const walk = n => n.children.forEach(c => c.type === "split" ? walk(c) : out.push(area.paneViews[c.id]));
        walk(tree().root);
        return out;
    }

    function titles(view) {
        return view.tabIds.map(id => area.tabById(id).title).join("|");
    }

    // The tab's TabButton: Repeater delegates are not in the Repeater's children but of its
    // parent, the Loader's content is under `item`, and QQC2 reparents declarative content
    // under `contentItem`, so the walk covers all three.
    function tabButton(view, tab) {
        function walk(item) {
            if (item.objectName === "tabButton" && item.tab === tab)
                return item;
            for (const c of item.children) {
                const f = walk(c);
                if (f)
                    return f;
            }
            if (item.item) {
                const f = walk(item.item);
                if (f)
                    return f;
            }
            if (item.contentItem) {
                const f = walk(item.contentItem);
                if (f)
                    return f;
            }
            return null;
        }
        return walk(view);
    }

    function texts(entries) {
        return entries.map(e => e === "-" ? "-" : e.text).join(",");
    }

    // Drags `tab`'s button to (dx, dy) relative to `item`'s centre, both in `window`
    // (default the main one).
    function dragTab(tab, item, dx, dy, window) {
        const btn = tabButton(area.viewOf(tab), tab);
        const a = btn.mapToItem(null, btn.width / 3, btn.height / 2);
        const b = item.mapToItem(null, item.width / 2 + dx, item.height / 2 + dy);
        Probe.drag(window || shell, a.x, a.y, b.x, b.y, Qt.LeftButton, 0);
    }

    // What a DropArea hands its handlers (DragEvent), carrying `data` as our MIME.
    function dropEvent(x, y, data) {
        const e = {
            "x": x,
            "y": y,
            "action": -1,
            "getDataAsString": f => f === area.tabMime ? data : "",
            "accept": a => e.action = a
        };
        return e;
    }

    // A drag of `tab` from `view` already switched to the system drag (startSystemDrag would
    // block in QDrag::exec; offscreen it returns at once).
    function beginSystemDrag(tab, view) {
        area.beginTabDrag(tab, view);
        area.tabDrag = Object.assign({}, area.tabDrag, {
            "system": true
        });
    }

    // The orientation of the splitter holding `paneId`.
    function parentOrientation(paneId) {
        let out = "";
        const walk = n => n.children.forEach(c => c.type === "split" ? walk(c) : (c.id === paneId ? out = n.orientation : null));
        walk(tree().root);
        return out;
    }

    function windowsList() {
        return Object.keys(area.windows).map(k => area.windows[k]);
    }

    readonly property var firstRun: firstRunA.concat(firstRunB)

    readonly property var firstRunA: [
        () => {
            area = findArea(shell.contentItem);
            expect(area !== null, "no PanesArea");
            expect(area.paneCount === 1 && area.activeTab === null, "not one empty pane");
            expect(shell.title === "Termigator", "title " + shell.title);
            expect(shell.liveSessions === 0, "live " + shell.liveSessions);
            shell.requestActivate();
        },
        // _open_inner: embedded tabs in the active pane; a blank command is refused.
        () => {
            shell.openInner("sleep 300", "alpha", "embedded", "");
            shell.openInner("exec bash -i", longTitle, "", JSON.stringify({
                "X": "1"
            }));
            const t = area.activeTab;
            expect(t.text === longTitle.slice(0, 31) + "…" && t.title === longTitle, "short text " + t.text);
            expect(shell.title === t.text + " — Termigator", "title " + shell.title);
            expect(shell.liveSessions === 2, "live " + shell.liveSessions);
            expect(t.extraEnv.X === "1" && t.inner === "exec bash -i", "env / inner");
            expect(titles(panes()[0]) === "alpha|" + longTitle, "tabs " + titles(panes()[0]));
            shell.openInner("   ", "x", "embedded", "");
            expect(area.messageBox.text === "No command for this entry.", area.messageBox.text);
            area.messageBox.close();
        },
        // Ctrl+( splits right; the current tab moves into the new, active pane (accent line).
        () => {
            shell.requestActivate(); // the closed box took the window's focus
            if (area.messageBox.visible || !shell.active)
                return retry;
            // A single pane has no accent line, even though it is the active one.
            expect(area.paneCount === 1 && area.activePane === panes()[0].paneId && !panes()[0].accent, "accent with one pane");
            press(shell, Qt.Key_ParenLeft, Qt.ControlModifier, "(");
        },
        () => {
            const [left, right] = panes();
            expect(area.paneCount === 2 && titles(left) === "alpha" && titles(right) === longTitle, "split right");
            expect(area.activePane === right.paneId && right.accent && !left.accent, "accent on the new pane");
            expect(shell.title.startsWith("beta and"), "title " + shell.title);
            // Focus in another pane's view makes that pane the active one.
            right.focusCurrent(); // the in-scene message box gives focus back to the left pane; move it away first
            left.focusCurrent();
            expect(area.activePane === left.paneId && left.accent && !right.accent, "focus moves the active pane");
            expect(shell.title === "alpha — Termigator", "title " + shell.title);
        },
        () => press(shell, Qt.Key_D, Qt.ControlModifier | Qt.ShiftModifier),
        () => {
            const t = tree();
            expect(area.paneCount === 3 && t.root.children[0].type === "split" && t.root.children[0].orientation === "v", "split down nests");
            const below = panes()[1];
            expect(area.activePane === below.paneId && below.tabIds.length === 0 && below.accent, "empty new pane");
            press(shell, Qt.Key_X, Qt.ControlModifier | Qt.ShiftModifier);
        },
        () => {
            expect(area.paneCount === 2 && tree().root.children[0].type === "pane", "close pane unwraps");
            expect(area.activePane === 0, "active " + area.activePane);
            press(shell, Qt.Key_S, Qt.ControlModifier | Qt.AltModifier);
        },
        // Session → Rename (Ctrl+Alt+S): the dialog, then a stripped title.
        () => {
            const left = panes()[0];
            expect(left.renameDialog.visible && left.renameField.text === "alpha", "rename dialog");
            left.renameField.text = "  gamma  ";
            left.renameDialog.accept();
            expect(area.activeTab.text === "gamma" && area.activeTab.title === "gamma", "renamed");
            expect(shell.title === "gamma — Termigator", "title " + shell.title);
        },
        // Once the modal dialog is gone (it blocks the window's shortcuts while it fades out).
        // The exit transition of the Qt 6.12 desktop style outlasts one 150 ms tick, so wait for it (bounded).
        () => {
            if (panes()[0].renameDialog.visible && waitTicks++ < 40)
                return retry;
            expect(!panes()[0].renameDialog.visible, "rename dialog still open");
            press(shell, Qt.Key_T, Qt.ControlModifier | Qt.ShiftModifier);
        },
        // Next / previous / Alt+n.
        () => {
            const left = panes()[0];
            expect(titles(left) === "gamma|Local shell" && left.current === 1, "new tab " + titles(left));
            press(shell, Qt.Key_Tab, Qt.ControlModifier);
            expect(left.current === 0, "Ctrl+Tab " + left.current);
            press(shell, Qt.Key_9, Qt.AltModifier);
            expect(left.current === 1, "Alt+9 " + left.current);
            press(shell, Qt.Key_PageUp, Qt.ControlModifier);
            expect(left.current === 0, "Ctrl+PgUp " + left.current);
        },
        // The tab menu (order and texts).
        () => {
            const left = panes()[0];
            const tab = area.activeTab;
            expect(texts(area.tabMenuEntries(tab, false)) === "Rename tab…,Duplicate tab,Reconnect,-,Log output to file…,Record (asciinema)…,-,Detach to window,Split right,Split down,Close pane,-,Close tab", texts(area.tabMenuEntries(tab, false)));
            left.openMenu(tab);
            expect(left.menu.visible && left.menu.count === 13, "menu items " + left.menu.count);
            left.menu.close();
            tab.closing.connect(() => ++check.closings);
            area.tabReopened.connect(() => ++check.reopened);
            area.duplicateTab(tab);
            expect(titles(left) === "gamma|Local shell|gamma" && left.current === 2, "duplicate " + titles(left));
            area.reconnectTab(tab);
            expect(titles(left) === "gamma|Local shell|gamma" && left.current === 0, "reconnect in place " + titles(left));
            expect(closings === 1 && reopened === 1 && area.activeTab !== tab, "reconnect made a new tab");
        },
        // Ctrl+Shift+L detaches (once the menu is gone: a popup holds the window's keys).
        () => {
            if (panes()[0].menu.visible && waitTicks++ < 40)
                return retry;
            expect(!panes()[0].menu.visible, "menu still open");
            shell.requestActivate();
            press(shell, Qt.Key_L, Qt.ControlModifier | Qt.ShiftModifier);
        },
        // The detached window has its own keys.
        () => {
            const w = windowsList();
            expect(w.length === 1, "windows " + w.length);
            win = w[0];
            expect(win.title === "gamma — Termigator" && titles(win.view) === "gamma", "detached " + win.title);
            expect(titles(panes()[0]) === "Local shell|gamma" && shell.liveSessions === 4, "left after detach, live " + shell.liveSessions);
            win.requestActivate();
        },
        () => press(win, Qt.Key_T, Qt.ControlModifier | Qt.ShiftModifier),
        () => {
            expect(titles(win.view) === "gamma|Local shell", "Ctrl+Shift+T in the window " + titles(win.view));
            expect(titles(panes()[0]) === "Local shell|gamma", "main untouched");
            expect(texts(area.tabMenuEntries(win.view.currentTab, true)) === "Attach to main window,Rename tab…,Duplicate tab,Reconnect,-,Log output to file…,Record (asciinema)…,-,Close tab", "detached menu");
            // A restart stays in the window.
            area.restartTab(win.view.currentTab);
            expect(titles(win.view) === "gamma|Local shell" && win.view.current === 1, "restart in the window");
            press(win, Qt.Key_A, Qt.ControlModifier | Qt.ShiftModifier);
        },
        () => {
            expect(titles(win.view) === "gamma", "attach " + titles(win.view));
            expect(titles(panes()[0]) === "Local shell|gamma|Local shell", "attached into the active pane");
            win.view.currentTab.closing.connect(() => console.info("termigator u3: closed detached"));
            const web = area.addTab({
                "title": "web",
                "inner": "ssh u@h",
                "conn": {
                    "host": "h",
                    "user": "u",
                    "port": 22,
                    "label": "web",
                    "password": "secret"
                }
            });
            const entries = area.tabMenuEntries(web, false);
            expect(texts(entries).startsWith("Rename tab…,Duplicate tab,Reconnect,Test connection…,-,Exclude from MultiExec,"), texts(entries));
            entries.find(e => e.text === "Exclude from MultiExec").fn();
            expect(web.multiexecExcluded, "exclude");
            // A split pane that loses its last tab goes away.
            area.select(panes()[0], 2);
            press(shell, Qt.Key_E, Qt.ControlModifier | Qt.ShiftModifier);
        },
        () => {
            expect(area.paneCount === 3 && titles(panes()[1]) === "Local shell", "split right " + area.paneCount);
            press(shell, Qt.Key_W, Qt.ControlModifier | Qt.ShiftModifier);
        },
        () => {
            expect(area.paneCount === 2 && titles(panes()[0]) === "Local shell|gamma|web", "closed " + titles(panes()[0]));
            // The shell's hooks for the toolbar and the host list (_quick_connect, _open_favorite_at).
            expect(shell.quickConnectHandler("nohost").startsWith("Use user@host"), "quick error");
            expect(shell.quickConnectHandler("u@quick") === "", "quick connect");
            const q = area.activeTab;
            expect(q.title === "SSH u@quick" && q.conn.host === "quick" && q.inner.indexOf("u@quick") >= 0, "quick tab " + q.title);
            area.closeTab(q);
            shell.openConnection(JSON.stringify({
                "label": "fav",
                "command": " top ",
                "terminal": "embedded"
            }));
            expect(area.activeTab.title === "fav" && area.activeTab.inner === "top" && area.activeTab.conn === null, "favorite");
            area.closeTab(area.activeTab);
            // A tab's own question before it closes (file transfers).
            const tmp = area.addTab({
                "title": "tmp",
                "inner": "true"
            });
            tmp.closeQuestion = () => "Stop it?";
            area.closeTab(tmp);
            expect(area.questionBox.text === "Stop it?" && area.tabById(tmp.tabId) === tmp, "asked first");
            area.questionBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            area.questionBox.close();
            expect(area.tabById(tmp.tabId) === tmp, "closed although answered No");
            area.closeTab(tmp);
            area.questionBox.buttonClicked(MessageDialog.Yes, MessageDialog.YesRole);
            area.questionBox.close();
            expect(titles(panes()[0]) === "Local shell|gamma|web", "not closed after Yes");
            // A live session asks before its tab closes (window.confirm_close); a detached window's tabs do not ask twice.
            const liveTab = area.tabList.find(t => t.title === "gamma"); // runs `sleep 300`
            expect(liveTab && liveTab.embedded && liveTab.live, "gamma is live");
            shell.controller.setWindowBool("confirm_close_tab", true);
            area.closeTab(liveTab);
            expect(area.questionBox.visible && area.questionBox.text === "This session is still running. Close the tab?", "live tab asked: " + area.questionBox.text);
            area.questionBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            area.questionBox.close();
            expect(area.tabById(liveTab.tabId) === liveTab, "live tab closed although answered No");
            // Reconnect and Close pane ask too while a session runs.
            expect(area.reconnectTab(liveTab) === null && area.questionBox.visible && area.questionBox.text.startsWith("This session is still running. Reconnect"), "reconnect asked: " + area.questionBox.text);
            area.questionBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            area.questionBox.close();
            expect(area.tabById(liveTab.tabId) === liveTab, "live tab reconnected although answered No");
            const panesBefore = panes().length;
            expect(area.tabLoc[liveTab.tabId].pane !== undefined, "gamma is in a pane");
            area.setActive(area.tabLoc[liveTab.tabId].pane);
            area.closePane();
            expect(area.questionBox.visible && area.questionBox.text.indexOf("still running. Close the pane?") > 0, "close pane asked: " + area.questionBox.text);
            area.questionBox.buttonClicked(MessageDialog.No, MessageDialog.NoRole);
            area.questionBox.close();
            expect(panes().length === panesBefore && area.tabById(liveTab.tabId) === liveTab, "pane closed although answered No");
            shell.controller.setWindowBool("confirm_close_tab", false);
            // The same question 3×: a fresh dialog each time (a reused MessageDialog gains buttons under the KDE theme).
            const seen = [];
            for (let i = 0; i < 3; i++) {
                area.questionBox.open();
                const d = area.questionBox.current;
                expect(area.questionBox.visible && d && !seen.includes(d) && d.buttons === (MessageDialog.Yes | MessageDialog.No), "question " + i + " is not a fresh Yes/No dialog");
                expect(d.native && d.dialogClass === "QMessageBox" && d.icon === "question", "question " + i + " is not a QMessageBox: " + d.dialogClass + " " + d.icon);
                seen.push(d);
            }
            area.questionBox.close();
            expect(!area.questionBox.visible && area.questionBox.current === null, "question still open after close()");
            // One click, one signal (ab9bc72); Esc / the window's close button answer with the escape button (No).
            const got = [];
            area.questionBox.buttonClicked.connect((b, r) => got.push(b + "/" + r));
            area.questionBox.open();
            area.questionBox.current.click(MessageDialog.Yes);
            expect(got.join() === (MessageDialog.Yes + "/" + MessageDialog.YesRole), "one click, one signal: " + got);
            area.questionBox.open();
            area.questionBox.current.dismiss();
            expect(got.length === 2 && got[1] === (MessageDialog.No + "/" + MessageDialog.NoRole), "close = No: " + got);
        },
        () => {
            // The dialogs are gone once their signals are handled.
            if (area.questionBox.visible || area.questionBox.current !== null)
                return retry;
            area.questionBox.open();
            expect(area.questionBox.current.dialogClass === "QMessageBox", "not reopened as a QMessageBox");
            area.questionBox.close();
            if (area.questionBox.visible)
                return retry;
        },
        // A double click on the tab renames it (the active pane, where addTab puts it).
        () => {
            shell.requestActivate();
            if (area.messageBox.visible || !shell.active)
                return retry;
            const view = area.paneViews[area.activePane];
            const tab = area.addTab({
                "title": "clk",
                "inner": "true"
            });
            const btn = tabButton(view, tab);
            expect(btn !== null, "tab button found");
            const p = btn.mapToItem(null, btn.width / 3, btn.height / 2);
            Probe.click(shell, p.x, p.y, Qt.LeftButton, 2, 0);
            expect(view.renameDialog.visible && view.renameField.text === "clk", "double click opens rename");
            view.renameDialog.reject();
            waitTicks = 0;
        },
        // Once the modal dialog is gone, a middle click on the same tab closes it (callLater).
        () => {
            const view = area.paneViews[area.activePane];
            if (view.renameDialog.visible && waitTicks++ < 40)
                return retry;
            expect(!view.renameDialog.visible, "rename dialog still open");
            const tab = area.tabList.find(t => t.title === "clk");
            const btn = tabButton(view, tab);
            const p = btn.mapToItem(null, btn.width / 3, btn.height / 2);
            Probe.click(shell, p.x, p.y, Qt.MiddleButton, 1, 0);
        },
        () => {
            expect(area.tabList.find(t => t.title === "clk") === undefined, "middle click closes the tab");
        },
        // Closing a detached window ends its sessions (confirm_close is off here).
        () => {
            shell.requestActivate();
            const tab = area.addTab({
                "title": "dw",
                "inner": "sleep 300"
            });
            tab.closing.connect(() => ++check.closings);
            check.closings = 0;
            const before = windowsList().length;
            area.detachTab(tab);
            const w = windowsList();
            expect(w.length === before + 1 && w.some(x => titles(x.view) === "dw"), "dw detached");
            w.find(x => titles(x.view) === "dw").close();
            expect(check.closings === 1 && area.tabList.find(t => t.title === "dw") === undefined, "closing the window ended its session");
            waitTicks = 0;
        },
        () => {
            if (windowsList().some(x => titles(x.view) === "dw") && waitTicks++ < 40)
                return retry;
            expect(!windowsList().some(x => titles(x.view) === "dw"), "detached window not destroyed");
        },
        // A tab "d1" in the right pane, dragged onto the right square of the left pane's cross.
        () => {
            shell.requestActivate();
            const [left, right] = panes();
            expect(area.paneCount === 2 && titles(left) === "Local shell|gamma|web", "before drag " + titles(left));
            area.addTab({
                "title": "d1",
                "inner": "true"
            }, {
                "pane": right.paneId
            });
        },
        () => {
            const d1 = area.tabList.find(t => t.title === "d1");
            dragTab(d1, panes()[0], area.dockGuide * 1.25, 0);
        },
        () => {
            const p = panes();
            expect(area.tabDrag === null, "drag state left over");
            expect(p.length === 3 && titles(p[0]) === "Local shell|gamma|web" && titles(p[1]) === "d1" && titles(p[2]) === longTitle, "cross right: " + p.map(titles).join(" / "));
            expect(parentOrientation(p[1].paneId) === "h" && area.activePane === p[1].paneId, "cross right: an h split, active");
            // The bottom sector of the long-title pane (below its cross): d1's emptied pane goes.
            const d1 = area.tabList.find(t => t.title === "d1");
            dragTab(d1, p[2], 0, p[2].height * 0.4);
        },
        () => {
            const p = panes();
            expect(p.length === 3 && titles(p[1]) === longTitle && titles(p[2]) === "d1", "bottom sector: " + p.map(titles).join(" / "));
            expect(parentOrientation(p[2].paneId) === "v" && parentOrientation(p[1].paneId) === "v", "bottom sector: a v split");
            // The centre square of the left pane.
            const d1 = area.tabList.find(t => t.title === "d1");
            dragTab(d1, p[0], 0, 0);
        },
        () => {
            const p = panes();
            expect(p.length === 2 && titles(p[0]) === "Local shell|gamma|web|d1" && p[0].current === 3, "centre: " + p.map(titles).join(" / "));
            expect(area.shape(tree().root) === "h(0," + p[1].paneId + ")", "centre: shape " + area.shape(tree().root));
            // A plain click still selects (below the drag threshold).
            const btn = tabButton(p[0], area.tabList.find(t => t.title === "gamma"));
            const c = btn.mapToItem(null, btn.width / 3, btn.height / 2);
            Probe.click(shell, c.x, c.y, Qt.LeftButton, 1, 0);
            expect(p[0].current === 1, "click selects " + p[0].current);
        },
        // Reorder: d1 before the first tab of its own bar.
        () => {
            const left = panes()[0];
            const d1 = area.tabList.find(t => t.title === "d1");
            const first = tabButton(left, area.tabById(left.tabIds[0]));
            dragTab(d1, first, -first.width / 2 + 2, 0);
        },
        () => {
            const left = panes()[0];
            expect(titles(left) === "d1|Local shell|gamma|web" && left.current === 0, "reorder " + titles(left));
            // The outer left edge arrow.
            const r = area.edgeRect("left");
            const d1 = area.tabList.find(t => t.title === "d1");
            dragTab(d1, area, r.x + r.width / 2 - area.width / 2, r.y + r.height / 2 - area.height / 2);
        },
        () => {
            const p = panes();
            expect(p.length === 3 && titles(p[0]) === "d1" && titles(p[1]) === "Local shell|gamma|web" && titles(p[2]) === longTitle, "outer left: " + p.map(titles).join(" / "));
            expect(tree().root.orientation === "h" && tree().root.children[0].id === p[0].paneId, "outer left: first in the root");
            area.closeTab(area.tabList.find(t => t.title === "d1"), false, true);
            expect(area.paneCount === 2 && titles(panes()[0]) === "Local shell|gamma|web", "layout back");
        },
        // Esc cancels (Probe.drag cannot stop half-way, so the drag's own calls stand in for it).
        () => {
            shell.requestActivate();
            const [left, right] = panes();
            right.focusCurrent(); // a real drag starts on a clicked tab, so its pane is the active one
            const before = area.controller.tree();
            area.beginTabDrag(right.currentTab, right);
            const p = left.mapToItem(area, left.width / 2 + area.dockGuide * 1.25, left.height / 2);
            area.updateTabDrag(p);
            expect(area.tabDrag.target && area.tabDrag.target.dock === "right" && area.tabDrag.target.view === left, "zone under the cursor");
            press(shell, Qt.Key_Escape, 0);
            expect(area.tabDrag === null, "Esc did not cancel");
            area.finishTabDrag();
            expect(area.controller.tree() === before, "a cancelled drag changed the tree");
        },
    ]

    // --u3-stress: after the steps above, 150 seeded docks of tabs onto any pane side, each a new
    // splitter tree (rebuild() once reparented the panes through a null parent: SIGSEGV in
    // QQuickSplitView::itemVisibilityChanged).
    readonly property var stressSteps: firstRunA.concat(Array.from({
        length: 150
    }, (_, i) => () => {
        let seed = (i + 1) * 2654435761 % 4294967296;
        const rnd = n => {
            seed = (seed * 1103515245 + 12345) % 2147483648;
            return seed % n;
        };
        const ps = panes();
        if (i % 5 === 0)
            area.addTab({
                "title": "s" + i,
                "inner": "true"
            }, {
                "pane": ps[rnd(ps.length)].paneId
            });
        const main = area.tabList.filter(x => area.tabLoc[x.tabId].window === undefined);
        const tab = main[rnd(main.length)];
        const target = ps[rnd(ps.length)];
        // At most four panes: the minimum sizes of more would not fit the window.
        const dock = ps.length >= 4 ? "center" : ["center", "left", "right", "top", "bottom"][rnd(5)];
        area.syncSizes();
        area.controller.moveTab(tab.tabId, false, target.paneId, dock, -1, 200);
        area.sync();
    }), [() => Qt.exit(failed ? 1 : 0)])

    readonly property var firstRunB: [
        // A second tab in the detached window, dragged before its first one (the
        // pointer stays in the window: the DragHandler path, no system drag).
        () => {
            shell.requestActivate();
            expect(windowsList().length === 1 && titles(win.view) === "gamma", "window before stage 3 " + titles(win.view));
            area.addTab({
                "title": "w2",
                "inner": "true"
            }, {
                "window": win.windowId
            });
            expect(titles(win.view) === "gamma|w2", "w2 in the window " + titles(win.view));
            // Offscreen the detached window comes up 1×1; room for its tab bar.
            win.width = 600;
            win.height = 400;
            win.requestActivate();
        },
        () => {
            const w2 = area.tabList.find(t => t.title === "w2");
            const first = tabButton(win.view, area.tabById(win.view.tabIds[0]));
            dragTab(w2, first, -first.width / 2 + 2, 0, win);
        },
        // The same target computation for a tab that lives in a detached window: the right
        // square of the left pane's cross; the PaneTab (and its session) moves along.
        () => {
            expect(titles(win.view) === "w2|gamma" && win.view.current === 0 && area.tabDrag === null, "window reorder " + titles(win.view));
            expect(windowsList().length === 1, "a system drag started inside the window");
            const w2 = area.tabList.find(t => t.title === "w2");
            const left = panes()[0];
            area.beginTabDrag(w2, win.view);
            area.updateTabDrag(left.mapToItem(area, left.width / 2 + area.dockGuide * 1.25, left.height / 2));
            const t = area.tabDrag.target;
            expect(t && t.dock === "right" && t.view === left && area.tabDrag.host === area, "zone for a window's tab");
            area.finishTabDrag();
            const p = panes();
            expect(p.length === 3 && titles(p[1]) === "w2" && titles(p[2]) === longTitle && titles(win.view) === "gamma", "window to pane: " + p.map(titles).join(" / ") + " | " + titles(win.view));
            expect(area.tabById(w2.tabId) === w2 && area.tabList.length === new Set(area.tabList).size, "the tab was not kept");
        },
        // The DropArea handlers (system drag): a pane's tab into the detached window. Another
        // instance's tab is refused; the window's middle appends, its tab bar inserts; no sides.
        () => {
            const w2 = area.tabList.find(t => t.title === "w2");
            const v = win.view;
            beginSystemDrag(w2, area.viewOf(w2));
            const mime = area.controller.pid() + ":" + w2.tabId;
            expect(!area.tabDragEntered(dropEvent(v.width / 2, v.height / 2, "1:" + w2.tabId), v), "another instance's tab accepted");
            expect(area.tabDragEntered(dropEvent(v.width / 2, v.height / 2, mime), v), "own tab refused");
            let t = area.tabDrag.target;
            expect(area.tabDrag.host === v && t && t.dock === "center" && t.at === -1 && t.rect, "window middle");
            const btn = tabButton(v, area.tabById(v.tabIds[0]));
            const p = btn.mapToItem(v, 2, btn.height / 2);
            area.updateTabDrag(p, v);
            t = area.tabDrag.target;
            expect(t && t.dock === "center" && t.at === 0 && t.line, "window tab bar");
            area.tabDragExited(area); // not the host: no change
            expect(area.tabDrag.host === v, "exit of another host");
            const drop = dropEvent(p.x, p.y, mime);
            area.tabDropped(drop, v);
            expect(drop.action === Qt.MoveAction, "drop not accepted");
            area.endSystemDrag(Qt.MoveAction, Date.now());
            expect(area.tabDrag === null && titles(win.view) === "w2|gamma" && win.view.current === 0, "pane to window " + titles(win.view));
            expect(area.paneCount === 2 && area.tabById(w2.tabId) === w2, "emptied pane left over");
        },
        // Ended outside every window: Esc over a window (it saw the drag leave just now) changes
        // nothing; a real drop outside detaches; the only tab of a detached window stays.
        () => {
            const w2 = area.tabList.find(t => t.title === "w2");
            beginSystemDrag(w2, win.view);
            area.tabDragEntered(dropEvent(win.view.width / 2, win.view.height / 2, area.controller.pid() + ":" + w2.tabId), win.view);
            area.tabDragExited(win.view);
            expect(area.tabDrag.host === null && area.tabDrag.target === null, "exit kept the target");
            area.endSystemDrag(Qt.IgnoreAction, Date.now());
            expect(windowsList().length === 1 && titles(win.view) === "w2|gamma", "Esc detached");
            beginSystemDrag(w2, win.view);
            area.endSystemDrag(Qt.IgnoreAction, Date.now());
            const w = windowsList();
            expect(w.length === 2 && titles(win.view) === "gamma" && w.some(x => titles(x.view) === "w2"), "drop outside did not detach");
            const own = w.find(x => titles(x.view) === "w2");
            beginSystemDrag(w2, own.view);
            area.endSystemDrag(Qt.IgnoreAction, Date.now());
            expect(windowsList().length === 2 && titles(own.view) === "w2", "the only tab was detached again");
            area.attachTab(w2);
            expect(titles(area.viewOf(w2)).endsWith("|w2"), "attach");
            waitTicks = 0;
        },
        // A real drag out of the main window: the DragHandler starts the system drag (offscreen
        // QDrag::exec ends at once with IgnoreAction, as a drop outside), so the tab detaches.
        () => {
            if (windowsList().length > 1 && waitTicks++ < 40)
                return retry;
            shell.requestActivate();
            const w2 = area.tabList.find(t => t.title === "w2");
            const btn = tabButton(area.viewOf(w2), w2);
            const a = btn.mapToItem(null, btn.width / 3, btn.height / 2);
            Probe.drag(shell, a.x, a.y, -40, a.y, Qt.LeftButton, 0);
        },
        () => {
            const w2 = area.tabList.find(t => t.title === "w2");
            const w = windowsList();
            expect(area.tabDrag === null && w.length === 2 && w.some(x => titles(x.view) === "w2") && area.tabById(w2.tabId) === w2, "drag out of the window: " + w.map(x => titles(x.view)).join(" / "));
            area.closeTab(w2, false, true);
            waitTicks = 0;
        },
        () => {
            if (windowsList().length > 1 && waitTicks++ < 40)
                return retry;
            const p = panes();
            expect(windowsList().length === 1 && titles(win.view) === "gamma", "windows after stage 3");
            expect(p.length === 2 && titles(p[0]) === "Local shell|gamma|web" && titles(p[1]) === longTitle, "layout after stage 3: " + p.map(titles).join(" / "));
        },
        () => {
            shell.requestActivate();
            area.activeTab.textColor = "red"; // shows in the grab
            const started = shell.contentArea.grabToImage(result => {
                if (!result.saveToFile(shell.controller.smokeTestShot))
                    check.fail("cannot save the grab");
                // confirm_close is off: straight to shuttingDown (snapshot) and the saved config.
                check.shell.requestQuit();
            });
            expect(started, "grab not started");
        }
    ]

    // The layout of the first run; the connection tab's command is built anew.
    readonly property var secondRun: [
        () => {
            area = findArea(shell.contentItem);
            const [left, right] = panes();
            expect(area.paneCount === 2 && titles(left) === "Local shell|gamma|web" && titles(right) === longTitle, "restored panes " + titles(left));
            const web = area.tabById(left.tabIds[2]);
            expect(web.conn.host === "h" && web.conn.password === undefined && web.inner.indexOf("u@h") >= 0, "restored connection " + web.inner);
            expect(right.currentTab.text === longTitle.slice(0, 31) + "…", "restored text");
            const w = windowsList();
            expect(w.length === 1 && titles(w[0].view) === "gamma" && w[0].title === "gamma — Termigator", "restored window");
            // The tabs run real sessions; the connection tab's ssh to "h" may have failed already.
            expect(shell.liveSessions === 5 || shell.liveSessions === 4, "live " + shell.liveSessions);
            Qt.exit(failed ? 1 : 0);
        }
    ]

    readonly property var steps: restoreRun ? secondRun : stressRun ? stressSteps : firstRun

    Timer {
        interval: 150
        repeat: true
        running: !check.failed && check.step < check.steps.length
        onTriggered: {
            if (check.steps[check.step]() !== check.retry)
                ++check.step;
        }
    }
}
