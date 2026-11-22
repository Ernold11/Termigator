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
// Host list, terminal and detached-window keys (contexts hostList / terminal / detachedWindow)
// are not bound here: the host list, terminal and panes bind them in their own FocusScopes.
Item {
    id: registry

    required property string tableJson
    readonly property var table: JSON.parse(tableJson)
    property var actions: ({})
    property bool built: false
    property var menus: ({})

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

    function buildMenu(path, parent, found) {
        const menu = menuComponent.createObject(parent);
        for (const a of table.actions) {
            if (a.menu !== path)
                continue;
            if (a.sep && menu.count > 0)
                menu.addItem(separatorComponent.createObject(null));
            if (a.submenu) {
                const sub = buildMenu(path + "/" + plain(a.text), parent, found);
                sub.title = a.text;
                sub.icon.name = a.icon;
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
