import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// Terminal panes, tabs, detached windows and session restore. ShellWindow's
// `panesComponent`. The pane tree lives in PanesController (panes_qobj.rs); every change goes through it and then sync(), which updates the views.
//
// What it sets on the shell: openConnection, quickConnectHandler, openInner, newLocalShell,
// liveSessions, the window title, the handlers of the pane / tab actions (file.new_tab,
// view.split_*, view.close_pane, view.next_tab / prev_tab, tabs.goto_*, session.rename /
// duplicate / reconnect / detach / close_tab), and `last_session` on shuttingDown()
// (window.session_restore); restores it at start. Connection tabs keep their entry in
// PaneTab.conn (detectDistro: one opened from the host list).
//
// Tab contents (terminal, Files) — set or call these here:
//   tabContent        Component of a tab's view; the terminal area sets the terminal
//                     (qml/terminal/); the
//                     placeholder at the bottom is the fallback. Created for a tab wherever it is shown with `tab` (its PaneTab)
//                     and destroyed and created again when the tab moves (split, detach, attach),
//                     so keep the session in `tab.session`. `visible` = the tab is current in its
//                     pane / window; `visible && Window.active` = the user sees it (attention).
//                     A tab's own PaneTab.content (Files) replaces it for that tab.
//   PaneTab           the tab's model: what PanesArea sets, what the view's owner sets
//                     (live, iconName, toolTip, textColor, logActive, recordingActive, session,
//                     closeQuestion) and the signals closing / logRequested / recordingRequested.
//   addTab(props, where, at)  a new tab: props = PaneTab properties (title, inner, conn, extraEnv,
//                     embedded, content, iconName…); where = {pane: id} or {window: id}, default
//                     the active pane; at = index (only with `where`). Returns its PaneTab.
//   closeTab(tab, keepPane, sure)   asks tab.closeQuestion(), then about a live session (unless sure); emits tab.closing.
//   restartTab(tab)   the session again in the same slot, pane or window (the R key, the
//                     disconnect notice); tabReopened(old, new) follows, as for Reconnect.
//   unlockVault       the vault area sets: fn(retry, cancelled) asks for the master password,
//                     then calls retry() once unlocked, else cancelled() (optional).
//                     Unset: the "Cannot decrypt" warning.
//   testConnection    the sidebar sets: fn(connJson) for the tab menu's "Test connection…".
//   disconnectNotice  the sidebar sets: fn(tab, label, livedSeconds), called for a connection
//                     tab that
//                     ended on its own while window.notify_on_disconnect is on.
//   The snapshot stores every `embedded` tab as {title, conn (without password) or inner}.
FocusScope {
    id: area

    required property var shell
    property Component tabContent: placeholderContent
    property var unlockVault: null
    property var testConnection: null
    property var disconnectNotice: null

    readonly property alias controller: controller
    readonly property alias terminalHost: terminals
    readonly property alias messageBox: messageBox
    readonly property alias questionBox: questionBox
    property var tabList: []
    property var paneViews: ({})
    property var windows: ({})
    property var tabLoc: ({}) // tab id -> {pane, index} or {window, index}
    property int activePane: 0
    property int paneCount: 1
    property var activeTab: null
    readonly property int liveCount: tabList.filter(t => t.embedded && t.live).length
    property var rootSplit: null
    property string shapeKey: ""
    property size pendingWindowSize: Qt.size(0, 0)
    property alias tabDrag: tabDragger.tabDrag // see TabDrag.qml
    readonly property string tabMime: "application/x-termigator-tab"
    readonly property real dockGuide: 40 // the side of a dock cross square (hitZone's `guide`)

    signal tabReopened(var oldTab, var newTab)

    function tabById(id) {
        return tabList.find(t => t.tabId === id) || null;
    }

    // Titles over 32 characters are cut to 31 plus "…" (code points, not UTF-16 units).
    function shortTitle(title) {
        const cp = [...title];
        return cp.length <= 32 ? title : cp.slice(0, 31).join("") + "…";
    }

    function viewOf(tab) {
        const l = tab ? tabLoc[tab.tabId] : undefined;
        if (!l)
            return null;
        return l.window !== undefined ? windows[l.window].view : paneViews[l.pane];
    }

    function message(text) {
        messageBox.text = text;
        messageBox.open();
    }

    function ask(text, yes) {
        questionBox.text = text;
        questionBox.yes = yes;
        questionBox.open();
    }

    // ---- the tree -> the views

    function shape(node) {
        if (node.type === "pane")
            return String(node.id);
        return node.orientation + "(" + node.children.map(shape).join(",") + ")";
    }

    function sync() {
        const tree = JSON.parse(controller.tree());
        const loc = {};
        const panes = [];
        const walk = node => node.children.forEach(c => c.type === "split" ? walk(c) : panes.push(c));
        walk(tree.root);
        const seenPanes = {};
        for (const p of panes) {
            seenPanes[p.id] = true;
            p.tabs.forEach((t, i) => loc[t] = {
                    "pane": p.id,
                    "index": i
                });
            if (paneViews[p.id])
                paneViews[p.id].main = p.main;
            else
                paneViews[p.id] = paneComponent.createObject(paneHolder, {
                    "area": area,
                    "paneId": p.id,
                    "main": p.main
                });
        }
        const seenWindows = {};
        for (const d of tree.detached) {
            seenWindows[d.id] = true;
            d.tabs.forEach((t, i) => loc[t] = {
                    "window": d.id,
                    "index": i
                });
        }
        tabLoc = loc;
        for (const p of panes)
            paneViews[p.id].setTabs(p.tabs, p.current === null ? -1 : p.current);
        const hadFocus = area.activeFocus;
        const key = shape(tree.root);
        const rebuilt = key !== shapeKey;
        if (rebuilt) {
            rebuild(tree.root);
            shapeKey = key;
        }
        for (const id in paneViews) {
            if (!seenPanes[id]) {
                paneViews[id].destroy();
                delete paneViews[id];
            }
        }
        const fresh = [];
        for (const d of tree.detached) {
            let w = windows[d.id];
            if (!w) {
                // The size of the pane the tab came from, at least 480x320.
                const main = paneViews[0];
                const size = pendingWindowSize.width > 0 ? pendingWindowSize : Qt.size(Math.max(480, main.width), Math.max(320, main.height));
                pendingWindowSize = Qt.size(0, 0);
                w = windowComponent.createObject(area, {
                    "area": area,
                    "windowId": d.id,
                    "width": size.width,
                    "height": size.height
                });
                windows[d.id] = w;
                fresh.push(w);
            }
            w.view.setTabs(d.tabs, d.current === null ? -1 : d.current);
        }
        for (const id in windows) {
            if (!seenWindows[id]) {
                windows[id].destroy();
                delete windows[id];
            }
        }
        activePane = tree.active;
        paneCount = panes.length;
        activeTab = paneViews[activePane].currentTab;
        for (const w of fresh) {
            w.show();
            w.raise();
            w.requestActivate();
        }
        if (hadFocus && rebuilt)
            paneViews[activePane].focusCurrent(); // reparenting the panes dropped it
    }

    // Logs panes that overlap or have no room: the splitter did not lay them out after a rebuild.
    function checkLayout(when) {
        const ids = Object.keys(paneViews);
        const rects = ids.map(id => ({
                    "id": id,
                    "r": itemRect(paneViews[id])
                }));
        let bad = "";
        for (let i = 0; i < rects.length; ++i) {
            const a = rects[i].r;
            if (a.width < 1 || a.height < 1)
                bad += " pane " + rects[i].id + " has no room " + JSON.stringify(a);
            for (let j = i + 1; j < rects.length; ++j) {
                const b = rects[j].r;
                if (a.x < b.x + b.width - 1 && b.x < a.x + a.width - 1 && a.y < b.y + b.height - 1 && b.y < a.y + a.height - 1)
                    bad += " panes " + rects[i].id + "," + rects[j].id + " overlap " + JSON.stringify(a) + JSON.stringify(b);
            }
        }
        if (bad !== "")
            console.warn("termigator: stale pane layout (" + when + "):" + bad);
    }

    // A new splitter tree around the existing pane views (they and their tabs' views survive).
    function rebuild(json) {
        // Out of the old splitters before they go. Not to null: the panes would turn invisible and
        // visible again, which crashes QQuickSplitView (Qt 6); parkHolder keeps them visible.
        for (const id in paneViews)
            paneViews[id].parent = parkHolder;
        const old = rootSplit;
        rootSplit = buildSplit(json, splitHolder);
        rootSplit.anchors.fill = splitHolder;
        if (old)
            old.destroy();
        Qt.callLater(nudgeLayout, true);
        layoutCheck.restart();
    }

    // A new SplitView sometimes keeps the panes where the old one had them until something resizes
    // it (dragging a handle did: two layouts on top of each other, a second tab bar's close
    // button). Flipping a child's `fill` makes it lay out again; the next tick flips it back.
    function nudgeLayout(again) {
        const walk = s => {
            for (let i = 0; i < s.count; ++i) {
                const c = s.itemAt(i);
                if (c.isSplit)
                    walk(c);
                c.fill = !c.fill;
            }
        };
        if (rootSplit)
            walk(rootSplit);
        if (again)
            Qt.callLater(nudgeLayout, false);
    }

    Timer {
        id: layoutCheck

        interval: 300
        onTriggered: area.checkLayout("300 ms later")
    }

    function buildSplit(json, parentItem) {
        const s = splitComponent.createObject(parentItem, {
            "orientation": json.orientation === "h" ? Qt.Horizontal : Qt.Vertical
        });
        for (const c of json.children) {
            if (c.type === "split")
                buildSplit(c, s);
            else
                paneViews[c.id].parent = s; // a SplitView adopts a new child item as its last
        }
        // Sizes the view never reported (0) split the space evenly.
        const n = s.count;
        s.itemAt(n - 1).fill = true;
        const known = json.sizes.length === n && json.sizes.every(x => x > 0);
        for (let i = 0; i < n; ++i)
            s.itemAt(i).prefSize = known ? json.sizes[i] : Qt.binding(() => (s.orientation === Qt.Horizontal ? s.width : s.height) / n);
        return s;
    }

    // The splitters' current sizes into the tree, before it changes and before a snapshot.
    function syncSizes() {
        const walk = (s, path) => {
            const sizes = [];
            for (let i = 0; i < s.count; ++i) {
                const c = s.itemAt(i);
                sizes.push(Math.round(s.orientation === Qt.Horizontal ? c.width : c.height));
                if (c.isSplit)
                    walk(c, path.concat([i]));
            }
            controller.setSizes(JSON.stringify(path), JSON.stringify(sizes));
        };
        if (rootSplit)
            walk(rootSplit, []);
    }

    // ---- tabs

    function makeTab(id, props) {
        const tab = tabComponent.createObject(area, Object.assign({
            "tabId": id,
            "text": shortTitle(props.title || "")
        }, props));
        tabList = tabList.concat([tab]);
        return tab;
    }

    function addTab(props, where, at) {
        where = where || {};
        let pos = at === undefined ? -1 : at;
        let id = 0;
        if (where.window !== undefined && windows[where.window])
            id = controller.addWindowTab(where.window, pos);
        if (id === 0) {
            // A pane gone by now means the active one, at the end.
            const pane = where.pane !== undefined && paneViews[where.pane] ? where.pane : activePane;
            if (pane !== where.pane)
                pos = -1;
            id = controller.addTab(pane, pos);
        }
        const tab = makeTab(id, props);
        sync();
        viewOf(tab).focusCurrent();
        return tab;
    }

    function select(view, index) {
        if (view.windowId > 0)
            controller.setWindowCurrent(view.windowId, index);
        else
            controller.setCurrent(view.paneId, index);
        sync();
        view.focusCurrent();
    }

    function setActive(pane) {
        if (pane === activePane || !paneViews[pane])
            return;
        controller.setActive(pane);
        sync();
    }

    function closeTab(tab, keepPane, sure) {
        if (!tab || tabList.indexOf(tab) < 0)
            return;
        let q = tab.closeQuestion ? tab.closeQuestion() : "";
        // User's decision 2026-11-12: a live session asks, under window.confirm_close_tab.
        if (!q && !sure && tab.embedded && tab.live && shell.controller.windowBool("confirm_close_tab", true))
            q = qsTr("This session is still running. Close the tab?");
        if (q)
            ask(q, () => finishClose(tab, keepPane));
        else
            finishClose(tab, keepPane);
    }

    function finishClose(tab, keepPane) {
        if (tabList.indexOf(tab) < 0)
            return;
        // The keyboard was in the closing tab's pane / window (or nowhere): the tab shown there
        // next takes it. Focus elsewhere (another pane, the host list) stays where it is.
        const l = tabLoc[tab.tabId];
        const pv = viewOf(tab);
        const w = pv ? pv.Window.window : null;
        const refocus = pv !== null && (pv.activeFocus || w === null || w.activeFocusItem === null);
        tab.closingByUser = true;
        tab.closing();
        syncSizes();
        controller.removeTab(tab.tabId, !!keepPane);
        tabList = tabList.filter(t => t !== tab);
        sync();
        tab.destroy(); // after its views (destroyed in sync)
        if (!refocus)
            return;
        if (l.window !== undefined && windows[l.window])
            windows[l.window].view.focusCurrent();
        else if (l.pane !== undefined && paneViews[l.pane])
            paneViews[l.pane].focusCurrent();
        else if (l.window === undefined && paneViews[activePane])
            paneViews[activePane].focusCurrent();
    }

    function closePane(sure) {
        const closing = paneViews[activePane].tabIds.map(tabById).filter(t => t);
        const live = closing.filter(t => t.embedded && t.live).length;
        // One chord must not end running sessions unasked (window.confirm_close_tab).
        if (live > 0 && !sure && shell.controller.windowBool("confirm_close_tab", true)) {
            ask(qsTr("%1 session(s) in this pane are still running. Close the pane?").arg(live), () => closePane(true));
            return;
        }
        for (const t of closing) {
            t.closingByUser = true;
            t.closing();
        }
        syncSizes();
        controller.closePane(activePane);
        tabList = tabList.filter(t => closing.indexOf(t) < 0);
        sync();
        closing.forEach(t => t.destroy());
    }

    function split(vertical) {
        const v = paneViews[activePane];
        syncSizes();
        controller.split(vertical, Math.round(vertical ? v.height : v.width));
        sync();
        paneViews[activePane].focusCurrent();
    }

    function cycleTab(step) {
        const v = paneViews[activePane];
        const n = v.tabIds.length;
        if (n < 2)
            return;
        controller.setCurrent(activePane, ((v.current + step) % n + n) % n);
        sync();
        v.focusCurrent();
    }

    // Alt+1..8 = that tab, Alt+9 = the last one.
    function gotoTab(n) {
        const v = paneViews[activePane];
        const count = v.tabIds.length;
        const i = n === 9 ? count - 1 : n - 1;
        if (count === 0 || i >= count)
            return;
        controller.setCurrent(activePane, i);
        sync();
        v.focusCurrent();
    }

    function renameTab(tab) {
        viewOf(tab).rename(tab);
    }

    function applyRename(tab, text) {
        const t = text.trim();
        if (t !== "" && tabList.indexOf(tab) >= 0) {
            tab.text = t;
            tab.title = t; // restart / duplicate rebuild the tab from it
        }
    }

    function duplicateTab(tab) {
        if (!tab || !tab.embedded) {
            message(qsTr("Duplicate is only available for embedded terminal tabs."));
            return null;
        }
        if (tab.inner.trim() === "") {
            message(qsTr("This tab has no stored command to duplicate."));
            return null;
        }
        const l = tabLoc[tab.tabId];
        return addTab(copyOf(tab), l.window !== undefined ? {
            "window": l.window
        } : {
            "pane": l.pane
        });
    }

    function copyOf(tab) {
        return {
            "title": tab.title || "Session",
            "inner": tab.inner,
            "conn": tab.conn,
            "extraEnv": tab.extraEnv,
            "content": tab.content
        };
    }

    // Session → Reconnect, tab menu → Reconnect (reconnect an embedded tab).
    function reconnectTab(tab, sure) {
        if (!tab || !tab.embedded) {
            message(qsTr("Reconnect is only available for embedded terminal tabs."));
            return null;
        }
        // Reconnecting ends the running session first.
        if (!sure && tab.live && shell.controller.windowBool("confirm_close_tab", true)) {
            ask(qsTr("This session is still running. Reconnect anyway (the running session ends)?"), () => reconnectTab(tab, true));
            return null;
        }
        return reopen(tab, false);
    }

    function restartTab(tab) {
        return tab && tabList.indexOf(tab) >= 0 ? reopen(tab, true) : null;
    }

    function reopen(tab, keepPane) {
        if (tab.inner.trim() === "")
            return null;
        const l = tabLoc[tab.tabId];
        const props = copyOf(tab);
        finishClose(tab, keepPane);
        const fresh = addTab(props, l.window !== undefined ? {
            "window": l.window
        } : {
            "pane": l.pane
        }, l.index);
        tabReopened(tab, fresh);
        return fresh;
    }

    function detachTab(tab) {
        const v = viewOf(tab);
        if (!v)
            return;
        pendingWindowSize = Qt.size(Math.max(480, v.width), Math.max(320, v.height));
        syncSizes();
        controller.detachTab(tab.tabId);
        sync();
        viewOf(tab).focusCurrent();
    }

    function attachTab(tab) {
        if (!tab)
            return;
        controller.attachTab(tab.tabId);
        sync();
        shell.raise();
        shell.requestActivate();
        viewOf(tab).focusCurrent();
    }

    function closeWindow(id) {
        const w = windows[id];
        const live = w.view.tabIds.map(tabById).filter(t => t && t.embedded && t.live).length;
        if (live > 0 && shell.controller.windowBool("confirm_close", true)) {
            w.closeBox.text = qsTr("%1 session(s) in this window are still running. Close them?").arg(live);
            w.closeBox.open();
        } else {
            closeWindowTabs(id);
        }
    }

    function closeWindowTabs(id) {
        const w = windows[id];
        if (w)
            w.view.tabIds.map(tabById).forEach(t => closeTab(t, false, true));
    }

    // ---- dragging a tab (TabDrag.qml; DockOverlay shows tabDrag)

    function itemRect(item, to) {
        const p = item.mapToItem(to || area, 0, 0);
        return Qt.rect(p.x, p.y, item.width, item.height);
    }

    function edgeRect(dock) {
        return tabDragger.edgeRect(dock);
    }

    function beginTabDrag(tab, view, handler) {
        return tabDragger.beginTabDrag(tab, view, handler);
    }

    function updateTabDrag(p, host) {
        return tabDragger.updateTabDrag(p, host);
    }

    function cancelTabDrag() {
        return tabDragger.cancelTabDrag();
    }

    function endHandlerDrag() {
        return tabDragger.endHandlerDrag();
    }

    function finishTabDrag() {
        return tabDragger.finishTabDrag();
    }

    function startSystemDrag() {
        return tabDragger.startSystemDrag();
    }

    function tabDragEntered(drag, host) {
        return tabDragger.tabDragEntered(drag, host);
    }

    function tabDragExited(host) {
        return tabDragger.tabDragExited(host);
    }

    function tabDropped(drop, host) {
        return tabDragger.tabDropped(drop, host);
    }

    function endSystemDrag(action, at) {
        return tabDragger.endSystemDrag(action, at);
    }

    // ---- opening (the shell's hooks)

    // `error` is shown instead; embedded → a tab with `props` (conn…), else
    // the external terminal.
    function open(inner, title, mode, env, error, props) {
        if (error) {
            message(error);
            return;
        }
        if (inner.trim() === "") {
            message(qsTr("No command for this entry."));
            return;
        }
        const hasEnv = env && Object.keys(env).length > 0;
        if (controller.isExternal(mode || "")) {
            const err = controller.spawnExternal(inner, hasEnv ? JSON.stringify(env) : "");
            if (err !== "")
                message(err);
            return;
        }
        addTab(Object.assign({
            "title": title,
            "inner": inner.trim(),
            "extraEnv": hasEnv ? env : null
        }, props));
    }

    function openInner(inner, title, mode, extraEnvJson) {
        open(inner, title, mode, extraEnvJson ? JSON.parse(extraEnvJson) : null, "", {});
    }

    // "WxH" in device pixels of the display this window is on: the start size of an RDP session.
    function screenSize() {
        const ratio = Screen.devicePixelRatio || 1;
        return Screen.width > 0 ? Math.round(Screen.width * ratio) + "x" + Math.round(Screen.height * ratio) : "";
    }

    // A saved connection or favorite (open at an index).
    function openConnection(entryJson) {
        const p = JSON.parse(controller.connectionPlan(entryJson, screenSize()));
        if (p.files) {
            if (shell.openFilesTab)
                shell.openFilesTab(entryJson);
            return;
        }
        if (p.locked && unlockVault) {
            unlockVault(() => area.openConnection(entryJson));
            return;
        }
        if (p.stop !== undefined) {
            message(p.stop);
            return;
        }
        if (p.inner === undefined)
            return; // not an entry
        if (p.recent)
            shell.controller.persist();
        if (p.notice)
            message(p.notice);
        open(p.inner, p.title, p.mode, p.env, p.error, {
            "conn": p.conn ? JSON.parse(entryJson) : null,
            "detectDistro": p.distro
        });
    }

    // The toolbar's quick connect (quick connect): "" once a tab opened, else the error.
    function quickConnect(text) {
        const p = JSON.parse(controller.quickPlan(text));
        if (p.entry) {
            shell.controller.persist(); // the plan put the name first in the history
            openConnection(JSON.stringify(p.entry));
            return "";
        }
        if (p.error)
            return p.error;
        open(p.inner, p.title, p.mode, null, "", {
            "conn": p.conn
        });
        shell.controller.persist();
        return "";
    }

    function newLocalShell(shellPath) {
        const s = JSON.parse(controller.localShell(shellPath || ""));
        openInner(s.inner, s.title, s.mode, "");
    }

    // Ctrl+Shift+T in a detached window — the default shell, embedded, in that window.
    function newShellIn(windowId) {
        const s = JSON.parse(controller.localShell(""));
        addTab({
            "title": s.title,
            "inner": s.inner
        }, {
            "window": windowId
        });
    }

    // ---- menus (tab bar context menu, detached window tab menu)

    function tabMenuEntries(tab, detached) {
        const emb = tab.embedded;
        const hasConn = tab.conn !== null && typeof tab.conn === "object";
        const log = tab.logActive ? {
            "text": qsTr("Stop session log"),
            "icon": "process-stop"
        } : {
            "text": qsTr("Log output to file…"),
            "icon": "document-save"
        };
        log.fn = () => tab.logRequested();
        const rec = tab.recordingActive ? {
            "text": qsTr("Stop recording"),
            "icon": "media-playback-stop"
        } : {
            "text": qsTr("Record (asciinema)…"),
            "icon": "media-record"
        };
        rec.fn = () => tab.recordingRequested();
        const rename = {
            "text": qsTr("Rename tab…"),
            "icon": "edit-rename",
            "fn": () => renameTab(tab)
        };
        const close = {
            "text": qsTr("Close tab"),
            "icon": "tab-close",
            "fn": () => closeTab(tab)
        };
        const dup = {
            "text": qsTr("Duplicate tab"),
            "icon": "tab-duplicate",
            "enabled": emb,
            "fn": () => duplicateTab(tab)
        };
        const reconnect = {
            "text": qsTr("Reconnect"),
            "icon": "view-refresh",
            "enabled": emb
        };
        const addToHosts = {
            "text": qsTr("Add to Hosts…"),
            "icon": "list-add",
            "enabled": shell.saveToHosts !== null,
            "fn": () => shell.saveToHosts(tab.conn)
        };
        if (detached) {
            reconnect.fn = () => restartTab(tab);
            const out = [{
                    "text": qsTr("Attach to main window"),
                    "icon": "window-restore",
                    "fn": () => attachTab(tab)
                }, rename];
            if (emb)
                out.push(dup, reconnect);
            if (hasConn)
                out.push(addToHosts);
            if (emb)
                out.push("-", log, rec);
            return out.concat(["-", close]);
        }
        reconnect.fn = () => reconnectTab(tab);
        const out = [rename, dup, reconnect];
        if (hasConn)
            out.push({
                "text": qsTr("Test connection…"),
                "icon": "network-connect",
                "enabled": testConnection !== null,
                "fn": () => testConnection(JSON.stringify(tab.conn))
            }, addToHosts);
        out.push("-");
        if (emb && hasConn)
            out.push({
                "text": qsTr("Exclude from MultiExec"),
                "icon": "system-run",
                "checkable": true,
                "checked": tab.multiexecExcluded,
                "fn": () => tab.multiexecExcluded = !tab.multiexecExcluded
            });
        if (emb)
            out.push(log, rec);
        return out.concat(["-", {
                "text": qsTr("Detach to window"),
                "icon": "window-new",
                "fn": () => detachTab(tab)
            }, {
                "text": qsTr("Split right"),
                "icon": "view-split-left-right",
                "fn": () => split(false)
            }, {
                "text": qsTr("Split down"),
                "icon": "view-split-top-bottom",
                "fn": () => split(true)
            }, {
                "text": qsTr("Close pane"),
                "icon": "view-close",
                "fn": () => closePane()
            }, "-", close]);
    }

    // The "+" button's press-and-hold menu (plus menu).
    function shellMenuEntries() {
        const out = [{
                "text": qsTr("Local shell"),
                "icon": "utilities-terminal",
                "fn": () => newLocalShell("")
            }];
        // Linux: /etc/shells when there is a choice; Windows: cmd, PowerShell and WSL always, the
        // ones not installed here greyed out.
        const shells = JSON.parse(controller.localShells());
        if (shells.length > 1 || shells.some(s => !s.available)) {
            out.push("-");
            for (const sh of shells)
                out.push({
                    "text": sh.label,
                    "enabled": sh.available,
                    "fn": () => newLocalShell(sh.id)
                });
        }
        return out;
    }

    // ---- start and exit

    function restore() {
        const list = JSON.parse(controller.restore());
        if (list.length === 0)
            return;
        restoreFrom(list, 0, -1);
    }

    // Session restore: a sealed password with the vault locked asks for the master
    // password (a Cancel opens that tab without the password and the next one asks again); the
    // tabs after it wait for the answer. `skip`: the record whose question was just cancelled.
    function restoreFrom(list, from, skip) {
        for (let i = from; i < list.length; ++i) {
            const r = list[i];
            let props = {
                "title": r.title,
                "inner": r.inner
            };
            if (r.inner === undefined) {
                const p = JSON.parse(controller.reopenPlan(JSON.stringify(r.entry)));
                if (p.locked && unlockVault && skip !== i) {
                    unlockVault(() => restoreFrom(list, i, -1), () => restoreFrom(list, i, i));
                    return;
                }
                props = p.inner === undefined ? null : {
                    "title": r.title,
                    "inner": p.inner,
                    "conn": r.entry,
                    "extraEnv": Object.keys(p.env).length > 0 ? p.env : null
                };
            }
            if (props)
                makeTab(r.tab, props);
            else
                controller.removeTab(r.tab, !r.detached); // a detached window's pane stays in place
        }
        sync();
    }

    function shutdown() {
        syncSizes();
        const records = {};
        for (const t of tabList) {
            if (t.embedded)
                records[t.tabId] = {
                    "title": t.title,
                    "conn": t.conn,
                    "inner": t.inner
                };
        }
        controller.saveSnapshot(JSON.stringify(records));
        for (const t of tabList) {
            t.closingByUser = true;
            t.closing();
        }
        for (const id in windows)
            windows[id].hide();
    }

    objectName: "panesArea"

    PanesController {
        id: controller
    }

    Item {
        id: splitHolder

        anchors.fill: parent
    }

    DockOverlay {
        anchors.fill: splitHolder
        z: 10
        area: area
    }

    TabDrag {
        id: tabDragger

        anchors.fill: parent
        z: 11
        area: area
    }

    // Zero-size and visible: where the pane views wait while the splitters are rebuilt.
    Item {
        id: parkHolder

        width: 0
        height: 0
    }

    // Pane views live here (QObject parent) while the splitters around them come and go.
    Item {
        id: paneHolder

        visible: false
    }

    component SplitNode: QQC2.SplitView {
        id: node

        handle: SplitGrip {
            horizontal: node.orientation === Qt.Horizontal
        }
        readonly property bool isSplit: true
        property real prefSize: -1
        property bool fill: false

        QQC2.SplitView.fillWidth: fill
        QQC2.SplitView.fillHeight: fill
        QQC2.SplitView.preferredWidth: prefSize
        QQC2.SplitView.preferredHeight: prefSize
    }

    Component {
        id: splitComponent

        SplitNode {}
    }

    Component {
        id: paneComponent

        PaneView {}
    }

    Component {
        id: tabComponent

        PaneTab {}
    }

    Component {
        id: windowComponent

        DetachedWindow {}
    }

    // Until the terminal is set: the tab's title and command.
    Component {
        id: placeholderContent

        FocusScope {
            id: placeholder

            property var tab

            ColumnLayout {
                anchors.centerIn: parent
                width: Math.min(implicitWidth, parent.width - 2 * Kirigami.Units.largeSpacing)

                Kirigami.Heading {
                    Layout.fillWidth: true
                    level: 3
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideRight
                    text: placeholder.tab ? placeholder.tab.title : ""
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WrapAnywhere
                    textFormat: Text.PlainText
                    font: Kirigami.Theme.fixedWidthFont
                    text: placeholder.tab ? placeholder.tab.inner : ""
                }
            }
        }
    }

    MessageBox {
        id: messageBox

        title: "Termigator"
        buttons: MessageDialog.Ok
    }

    MessageBox {
        id: questionBox

        property var yes: null

        title: "Termigator"
        buttons: MessageDialog.Yes | MessageDialog.No
        onButtonClicked: (button, role) => {
            const fn = yes;
            yes = null;
            if (button === MessageDialog.Yes && fn)
                fn();
        }
    }

    Connections {
        target: area.shell

        function onShuttingDown() {
            area.shutdown();
        }
    }

    // The terminal behind every embedded tab, the terminal actions and MultiExec.
    TerminalHost {
        id: terminals

        panes: area
    }

    Component.onCompleted: {
        tabContent = terminals.viewComponent;
        sync();
        shell.panes = area;
        shell.openConnection = entryJson => area.openConnection(entryJson);
        shell.quickConnectHandler = text => area.quickConnect(text);
        shell.openInner = (inner, title, mode, env) => area.openInner(inner, title, mode, env);
        shell.newLocalShell = path => area.newLocalShell(path);
        shell.liveSessions = Qt.binding(() => area.liveCount);
        // The window title names the active pane's current tab.
        shell.title = Qt.binding(() => {
            const t = area.activeTab ? area.activeTab.text.trim() : "";
            return t !== "" ? t + " — Termigator" : "Termigator";
        });
        const a = shell.shellActions;
        a.handle("file.new_tab", () => area.newLocalShell(""));
        a.handle("view.split_right", () => area.split(false));
        a.handle("view.split_down", () => area.split(true));
        a.handle("view.close_pane", () => area.closePane());
        a.handle("view.next_tab", () => area.cycleTab(1));
        a.handle("view.prev_tab", () => area.cycleTab(-1));
        for (let n = 1; n <= 8; ++n)
            a.handle("tabs.goto_" + n, () => area.gotoTab(n));
        a.handle("tabs.goto_last", () => area.gotoTab(9));
        a.handle("session.rename", () => {
            if (area.activeTab)
                area.renameTab(area.activeTab);
            else
                area.message(qsTr("No terminal tab to rename."));
        });
        a.handle("session.duplicate", () => area.duplicateTab(area.activeTab));
        a.handle("session.reconnect", () => area.reconnectTab(area.activeTab));
        a.handle("session.detach", () => {
            if (area.activeTab)
                area.detachTab(area.activeTab);
        });
        a.handle("session.close_tab", () => area.closeTab(area.activeTab));
        Qt.callLater(area.restore);
    }
}
