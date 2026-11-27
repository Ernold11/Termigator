import QtQuick
import QtQuick.Controls as QQC2

// The action table (AppController.actionsJson, core/actions.rs) as QQC2 Actions, menus and
// shortcuts: the single source for the menu bar, the toolbar and the window's keys (ADR-11).
//
// API for the areas:
//   actions[id]               the QQC2.Action (bindings to it update once `actions` is filled)
//   handle(id, fn, checkedFn) fn(checked) runs on trigger. Until an area registers its handler the
//                             action is disabled and its keys are not live; a DisplayOnly entry
//                             (Copy, Paste, Find, zoom) becomes a live window shortcut too.
//                             checkedFn: the checked state, re-bound after every trigger.
//   menus[path]               the menu-bar Menu of a submenu, e.g. "Edit/Snippets" (fill it yourself)
//   buildMenu(path, parent)   a new Menu with the entries of `path` (the toolbar's New ▾)
//   buildHamburger(parent, shown)  the toolbar's ☰ while the menu bar is hidden, laid out like KDE's
//                             KHamburgerMenu: a few frequent actions, Session / Tools / Settings / Help,
//                             then More with File / Edit / View; actions in `shown` (ids already on
//                             the toolbar) and the ones on top are left out of the submenus
//   fillers[path]             fn(menu): fills a submenu its area owns (Edit/Snippets) in a menu made
//                             by buildMenu / buildHamburger; the menu bar's own copy stays the area's to fill
// Host list, terminal and detached-window keys (contexts hostList / terminal / detachedWindow)
// are not bound here: the host list, terminal and panes bind them in their own FocusScopes.
Item {
    id: registry

    required property string tableJson
    readonly property var table: JSON.parse(tableJson)
    property var actions: ({})
    property bool built: false
    property var menus: ({})
    property var fillers: ({})

    function plain(text) {
        return text.replace(/&&/g, "\u0000").replace(/&/g, "").replace(/\u0000/g, "&");
    }

    function handle(id, fn, checkedFn) {
        ensureActions();
        const act = actions[id];
        act.handler = fn;
        act.checkedFn = checkedFn || null;
        if (checkedFn)
            act.checked = Qt.binding(checkedFn);
    }

    // Adds the top-level menus to `bar`.
    function populate(bar) {
        ensureActions();
        const found = {};
        for (const title of table.menus) {
            const menu = buildMenu(plain(title), bar, found);
            menu.title = title;
            bar.addMenu(menu);
        }
        menus = found;
    }

    function buildMenu(path, parent, found, skip) {
        const menu = menuComponent.createObject(parent);
        for (const a of table.actions) {
            if (a.menu !== path || (skip && skip[a.id]))
                continue;
            if (a.sep && menu.count > 0)
                menu.addItem(separatorComponent.createObject(null));
            if (a.submenu) {
                const subPath = path + "/" + plain(a.text);
                const sub = buildMenu(subPath, parent, found, skip);
                sub.title = a.text;
                sub.icon.name = a.icon;
                if (!found && fillers[subPath])
                    fillers[subPath](sub);
                menu.addMenu(sub);
                const item = menu.itemAt(menu.count - 1);
                item.enabled = Qt.binding(() => sub.count > 0);
                if (found)
                    found[path + "/" + plain(a.text)] = sub;
            } else {
                menu.addAction(actions[a.id]);
            }
        }
        return menu;
    }

    // The ☰ menu's own entries, above its submenus (ids; "-" is a separator).
    readonly property var hamburgerTop: ["session.duplicate", "session.reconnect", "session.log", "-", "view.fullscreen", "view.menubar", "view.zoom_in", "view.zoom_reset", "view.zoom_out"]
    readonly property var hamburgerMenus: ["Session", "Tools", "Settings", "Help"]

    // Destroying the returned Menu destroys its submenus too.
    function buildHamburger(parent, shown) {
        const top = menuComponent.createObject(parent);
        const skip = Object.assign({}, shown || {});
        for (const id of hamburgerTop) {
            if (id === "-") {
                top.addItem(separatorComponent.createObject(null));
            } else {
                top.addAction(actions[id]);
                skip[id] = true;
            }
        }
        const titled = path => table.menus.find(t => plain(t) === path) || path;
        const addSub = (into, path) => {
            const sub = buildMenu(path, top, null, skip);
            sub.title = titled(path);
            sub.icon.name = (table.menuIcons || {})[titled(path)] || "";
            into.addMenu(sub);
            into.itemAt(into.count - 1).enabled = Qt.binding(() => sub.count > 0);
        };
        top.addItem(separatorComponent.createObject(null));
        for (const path of hamburgerMenus)
            addSub(top, path);
        top.addItem(separatorComponent.createObject(null));
        const more = menuComponent.createObject(top);
        more.title = qsTr("More");
        more.icon.name = "overflow-menu";
        for (const t of table.menus) {
            if (hamburgerMenus.indexOf(plain(t)) < 0)
                addSub(more, plain(t));
        }
        top.addMenu(more);
        asWindows(top);
        return top;
    }

    // Each its own popup window, as KDE's menus are: submenus open beside it even past the
    // window's edge instead of folding back over it.
    function asWindows(menu) {
        menu.popupType = QQC2.Popup.Window;
        for (let i = 0; i < menu.count; ++i) {
            const sub = menu.menuAt(i);
            if (sub)
                asWindows(sub);
        }
    }

    // Whichever Component.onCompleted runs first (this one or the window's) creates the actions.
    function ensureActions() {
        if (built)
            return;
        built = true;
        const made = {};
        for (const a of table.actions) {
            if (a.submenu || (a.context !== "window" && a.context !== "displayOnly"))
                continue;
            const inMenu = a.menu !== "";
            const act = actionComponent.createObject(registry, {
                actionId: a.id,
                text: a.text,
                checkable: a.checkable,
                tip: a.tip,
                // A menu shows the primary key; the alternatives are extra Shortcuts.
                shortcut: inMenu && a.keys.length > 0 ? a.keys[0] : ""
            });
            act.icon.name = a.icon;
            const extra = inMenu ? a.keys.slice(1) : a.keys;
            if (extra.length > 0)
                shortcutComponent.createObject(registry, { action: act, sequences: extra });
            made[a.id] = act;
        }
        actions = made;
    }

    Component.onCompleted: ensureActions()

    Component {
        id: actionComponent

        QQC2.Action {
            property string actionId
            property string tip
            property var handler: null
            property var checkedFn: null

            enabled: handler !== null
            onTriggered: {
                handler(checked);
                if (checkedFn)
                    checked = Qt.binding(checkedFn);
            }
        }
    }

    Component {
        id: shortcutComponent

        Shortcut {
            property QQC2.Action action

            enabled: action.enabled
            onActivated: action.trigger()
        }
    }

    Component {
        id: menuComponent

        QQC2.Menu {}
    }

    Component {
        id: separatorComponent

        QQC2.MenuSeparator {}
    }
}
