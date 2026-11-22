pragma ComponentBehavior: Bound
import QtQuick
import org.kde.kirigami as Kirigami

// ---- dragging a tab (PaneView's DragHandler, then a system drag between windows;
// DockOverlay shows tabDrag). Part of PanesArea (`area`), which forwards its tabDrag property
// and the drag functions here; fills it, above DockOverlay.
Item {
    id: dragger

    required property var area
    // {tab, view, handler, system, host, target, pane, pos, dropped, left} while a tab is
    // dragged; positions are in `host` (the area, or a detached window's PaneView).
    property var tabDrag: null

    function inRect(r, p) {
        return p.x >= r.x && p.x <= r.x + r.width && p.y >= r.y && p.y <= r.y + r.height;
    }

    // The part of `r` a tab docked at `dock` would take.
    function dockRect(r, dock) {
        const w = r.width / 2, h = r.height / 2;
        switch (dock) {
        case "left":
            return Qt.rect(r.x, r.y, w, r.height);
        case "right":
            return Qt.rect(r.x + w, r.y, w, r.height);
        case "top":
            return Qt.rect(r.x, r.y, r.width, h);
        case "bottom":
            return Qt.rect(r.x, r.y + h, r.width, h);
        }
        return r;
    }

    // The arrow at an outer edge of the whole area.
    function edgeRect(dock) {
        const g = area.dockGuide, m = Kirigami.Units.smallSpacing;
        const width = area.width, height = area.height;
        switch (dock) {
        case "left":
            return Qt.rect(m, (height - g) / 2, g, g);
        case "right":
            return Qt.rect(width - m - g, (height - g) / 2, g, g);
        case "top":
            return Qt.rect((width - g) / 2, m, g, g);
        }
        return Qt.rect((width - g) / 2, height - m - g, g, g);
    }

    // `handler`: the DragHandler that started it, reset after a system drag (that eats its release).
    function beginTabDrag(tab, view, handler) {
        tabDrag = {
            "tab": tab,
            "view": view,
            "handler": handler || null,
            "system": false,
            "host": null,
            "target": null,
            "pane": null,
            "pos": null,
            "dropped": false,
            "left": 0
        };
        // A terminal with the focus swallows Esc (ShortcutOverride), and Esc cancels the drag.
        forceActiveFocus();
    }

    // The focus goes back to the tab's terminal once a drag ended without moving it.
    function restoreFocus(d) {
        if (d && area.tabList.indexOf(d.tab) >= 0)
            area.viewOf(d.tab).focusCurrent();
    }

    // Center at gap `p.x` of `v`'s tab bar (final index: inside its own list the tab's old slot goes).
    function barTarget(v, p) {
        const gap = v.tabGapAt(p.x);
        const from = area.tabLoc[tabDrag.tab.tabId];
        const own = from && (from.window !== undefined ? from.window === v.windowId : from.pane === v.paneId);
        const bar = v.tabBarRect();
        return {
            "view": v,
            "dock": "center",
            "at": own && gap > from.index ? gap - 1 : gap,
            "line": Qt.rect(v.tabGapX(gap) - 1, bar.y, 2, bar.height)
        };
    }

    // In the area: the edge arrows, then the tab bar of the pane under `p`, then its dock zones
    // (controller.hitZone, the same cross DockOverlay draws).
    function paneTarget(p) {
        let target = null;
        let pane = null;
        for (const dock of ["left", "right", "top", "bottom"]) {
            if (inRect(edgeRect(dock), p))
                target = {
                    "edge": true,
                    "dock": dock,
                    "rect": dockRect(Qt.rect(0, 0, area.width, area.height), dock)
                };
        }
        const v = Object.values(area.paneViews).find(v => inRect(area.itemRect(v), p)) || null;
        if (v)
            pane = area.itemRect(v);
        if (!target && v && v.tabIds.length > 0 && inRect(v.tabBarRect(), p)) {
            target = barTarget(v, p);
        } else if (!target && v) {
            const dock = area.controller.hitZone(pane.width, pane.height, p.x - pane.x, p.y - pane.y, area.dockGuide);
            // Its own pane's centre changes nothing.
            if (dock !== "" && !(dock === "center" && v === tabDrag.view))
                target = {
                    "view": v,
                    "dock": dock,
                    "at": -1,
                    "rect": dockRect(pane, dock)
                };
        }
        return {
            "target": target,
            "pane": pane
        };
    }

    // In a detached window (no splits): its tab bar, else the end of its list (not its own tab's).
    function windowTarget(v, p) {
        let target = null;
        if (inRect(v.tabBarRect(), p))
            target = barTarget(v, p);
        else if (v !== tabDrag.view)
            target = {
                "view": v,
                "dock": "center",
                "at": -1,
                "rect": Qt.rect(0, 0, v.width, v.height)
            };
        return {
            "target": target,
            "pane": null
        };
    }

    // `p` in `host` coordinates (default the area).
    function updateTabDrag(p, host) {
        if (!tabDrag)
            return;
        host = host || area;
        tabDrag = Object.assign({}, tabDrag, host === area ? paneTarget(p) : windowTarget(host, p), {
            "host": host,
            "pos": p
        });
    }

    function cancelTabDrag() {
        const d = tabDrag;
        tabDrag = null;
        restoreFocus(d);
    }

    // The DragHandler let go: a drop inside its window (a system drag ends in endSystemDrag).
    function endHandlerDrag() {
        if (tabDrag && !tabDrag.system)
            finishTabDrag();
    }

    // No zone, or a cancelled drag: no change.
    function finishTabDrag() {
        const d = tabDrag;
        tabDrag = null;
        if (!d || !d.target || area.tabList.indexOf(d.tab) < 0) {
            restoreFocus(d);
            return;
        }
        const t = d.target;
        const across = t.dock === "left" || t.dock === "right";
        area.syncSizes();
        if (t.edge)
            area.controller.dockAtEdge(d.tab.tabId, t.dock, Math.round(across ? area.width : area.height));
        else if (t.view.windowId > 0 && area.windows[t.view.windowId] && area.windows[t.view.windowId].view === t.view)
            area.controller.moveTab(d.tab.tabId, true, t.view.windowId, "center", t.at, 0);
        else if (t.view.windowId === 0 && area.paneViews[t.view.paneId] === t.view)
            area.controller.moveTab(d.tab.tabId, false, t.view.paneId, t.dock, t.at, Math.round(across ? t.view.width : t.view.height));
        area.sync();
        const l = area.tabLoc[d.tab.tabId];
        if (l.window !== undefined)
            area.windows[l.window].requestActivate();
        else if (d.system)
            area.shell.requestActivate();
        area.viewOf(d.tab).focusCurrent();
    }

    // The pointer left the source window: QDrag from here on (Wayland has no global window
    // positions). Blocks in QDrag::exec until the drop; dragProxy's onDragFinished follows.
    function startSystemDrag() {
        if (!tabDrag || tabDrag.system)
            return;
        tabDrag = Object.assign({}, tabDrag, {
            "system": true,
            "host": null,
            "target": null,
            "pane": null,
            "pos": null
        });
        const data = {};
        data[area.tabMime] = area.controller.pid() + ":" + tabDrag.tab.tabId;
        dragProxy.Drag.mimeData = data;
        dragProxy.Drag.active = true;
    }

    // A DropArea (the area's or a detached window's, `host`) met the drag: only our own tab.
    function tabDragEntered(drag, host) {
        const ok = tabDrag !== null && tabDrag.system && drag.getDataAsString(area.tabMime) === area.controller.pid() + ":" + tabDrag.tab.tabId;
        if (ok)
            updateTabDrag(Qt.point(drag.x, drag.y), host);
        return ok;
    }

    function tabDragExited(host) {
        if (tabDrag && tabDrag.host === host)
            tabDrag = Object.assign({}, tabDrag, {
                "host": null,
                "target": null,
                "pane": null,
                "pos": null,
                "left": Date.now()
            });
    }

    // Accepted even with no zone under it: a drop on an Termigator window never detaches.
    function tabDropped(drop, host) {
        if (!tabDrag || !tabDrag.system)
            return;
        updateTabDrag(Qt.point(drop.x, drop.y), host);
        tabDrag = Object.assign({}, tabDrag, {
            "dropped": true
        });
        drop.accept(Qt.MoveAction);
    }

    // After QDrag::exec (`at`: when it returned). Not taken by any Termigator window: detached,
    // unless it is the only tab of a detached window. Qt reports Esc like that too; Esc over an
    // Termigator window comes right after its DropArea saw the drag leave, so that one is a cancel.
    function endSystemDrag(action, at) {
        const d = tabDrag;
        if (!d || !d.system)
            return;
        if (d.handler) {
            d.handler.enabled = false;
            d.handler.enabled = true;
        }
        if (d.dropped) {
            finishTabDrag();
            return;
        }
        tabDrag = null;
        if (action !== Qt.IgnoreAction || at - d.left < 100 || area.tabList.indexOf(d.tab) < 0) {
            restoreFocus(d);
            return;
        }
        const l = area.tabLoc[d.tab.tabId];
        if (l.window === undefined || area.windows[l.window].view.tabIds.length > 1)
            area.detachTab(d.tab);
    }

    DropArea {
        anchors.fill: parent
        keys: [dragger.area.tabMime]
        onEntered: drag => drag.accepted = dragger.tabDragEntered(drag, dragger.area)
        onPositionChanged: drag => dragger.updateTabDrag(Qt.point(drag.x, drag.y), dragger.area)
        onExited: dragger.tabDragExited(dragger.area)
        onDropped: drop => dragger.tabDropped(drop, dragger.area)
    }

    // The system drag's source; here, so a drop that destroys the tab's delegate or window does
    // not take it along.
    Item {
        id: dragProxy

        Drag.dragType: Drag.Automatic
        Drag.supportedActions: Qt.MoveAction
        Drag.onDragFinished: dropAction => Qt.callLater(dragger.endSystemDrag, dropAction, Date.now())
    }

    Shortcut {
        sequence: "Escape"
        enabled: dragger.tabDrag !== null
        onActivated: dragger.cancelTabDrag()
    }
}
