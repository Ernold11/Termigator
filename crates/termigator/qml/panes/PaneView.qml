pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// A pane of the split tree, or the tab area of a detached window: the tab bar with the pane's
// "+" button, one view per tab, the empty-pane placeholder and the accent line over the
// active pane. PanesArea.sync() feeds it through setTabs(); every action goes back to PanesArea.
FocusScope {
    id: view

    required property var area
    property int paneId: -1 // -1 in a detached window
    property int windowId: 0 // the detached window, else 0
    property bool main: false
    property var tabIds: []
    property int current: -1
    property real prefSize: -1 // set by PanesArea.rebuild()
    property var views: ({})
    readonly property bool isSplit: false
    readonly property var currentTab: current >= 0 && current < tabIds.length ? area.tabById(tabIds[current]) : null
    readonly property bool accent: paneId >= 0 && area.paneCount > 1 && area.activePane === paneId
    readonly property Item coords: paneId >= 0 ? area : view // what a dragged tab's positions are relative to
    readonly property alias menu: menu
    readonly property alias renameDialog: paneRenameDialog
    readonly property alias renameField: paneRenameField

    function setTabs(ids, cur) {
        if (JSON.stringify(ids) !== JSON.stringify(tabIds))
            tabIds = ids;
        current = cur;
        syncViews();
    }

    // One view per tab, made here (a moved tab gets a new one: termigator-term-bridge's views are
    // disposable, sessions are not); only the current tab's is visible.
    function syncViews() {
        const keep = {};
        for (const id of tabIds) {
            keep[id] = true;
            const tab = area.tabById(id);
            if (!tab && !views[id])
                console.warn("termigator: pane " + paneId + " lists tab " + id + " but no such tab exists (an empty tab button)");
            if (views[id] || !tab)
                continue;
            const v = (tab.content || area.tabContent).createObject(stack, {
                "tab": tab
            });
            v.anchors.fill = stack;
            views[id] = v;
        }
        for (const id in views) {
            if (!keep[id]) {
                views[id].destroy();
                delete views[id];
            }
        }
        for (const id in views)
            views[id].visible = currentTab !== null && Number(id) === currentTab.tabId;
    }

    function focusCurrent() {
        const v = currentTab ? views[currentTab.tabId] : null;
        if (v)
            v.forceActiveFocus();
        else
            forceActiveFocus();
    }

    function rename(tab) {
        paneRenameDialog.tab = tab;
        paneRenameField.text = tab.text;
        paneRenameDialog.open();
    }

    // entries: {text, icon, fn, enabled, checkable, checked} or "-"; separators collapse like
    // QMenu's (no leading, trailing or doubled ones).
    function popupEntries(entries, at) {
        while (menu.count > 0)
            menu.takeItem(0).destroy();
        let afterSep = true;
        for (const e of entries) {
            if (e === "-") {
                if (!afterSep)
                    menu.addItem(separatorComponent.createObject(null));
                afterSep = true;
                continue;
            }
            const item = itemComponent.createObject(null, {
                "text": e.text,
                "enabled": e.enabled !== false,
                "checkable": !!e.checkable,
                "checked": !!e.checked,
                "fn": e.fn
            });
            item.icon.name = e.icon || "";
            menu.addItem(item);
            afterSep = false;
        }
        if (afterSep && menu.count > 0)
            menu.takeItem(menu.count - 1).destroy();
        if (at)
            menu.popup(at, 0, at.height);
        else
            menu.popup();
    }

    function openMenu(tab, at) {
        popupEntries(area.tabMenuEntries(tab, windowId > 0), at);
    }

    // The tab bar row (with "+") in `coords`.
    function tabBarRect() {
        return area.itemRect(barRow, coords);
    }

    // The gap (0..count) nearest to x in `coords`, from the buttons' centres.
    function tabGapAt(x) {
        const bar = barLoader.item as QQC2.TabBar;
        const n = bar ? bar.count : 0;
        for (let i = 0; i < n; ++i) {
            const b = bar.itemAt(i);
            if (x < b.mapToItem(coords, b.width / 2, 0).x)
                return i;
        }
        return n;
    }

    // The x of gap i in `coords` (the insertion line).
    function tabGapX(i) {
        const bar = barLoader.item as QQC2.TabBar;
        const n = bar ? bar.count : 0;
        if (n === 0)
            return tabBarRect().x;
        const b = bar.itemAt(Math.min(i, n - 1));
        return b.mapToItem(coords, i < n ? 0 : b.width, 0).x;
    }

    property bool fill: false // the last child of a splitter takes the room a resize adds or removes
    QQC2.SplitView.fillWidth: fill
    QQC2.SplitView.fillHeight: fill
    QQC2.SplitView.preferredWidth: prefSize
    QQC2.SplitView.preferredHeight: prefSize
    QQC2.SplitView.minimumWidth: main ? 360 : 120
    QQC2.SplitView.minimumHeight: 90

    // The active pane follows keyboard focus as well as clicks.
    onActiveFocusChanged: {
        if (activeFocus && paneId >= 0)
            area.setActive(paneId);
    }

    // Own backgrounds for the tab bar and an empty pane: the window is transparent while the
    // terminals are see-through (Preferences → Terminal).
    Rectangle {
        x: barRow.x
        y: barRow.y
        width: barRow.width
        height: barRow.height
        visible: barRow.visible
        color: Kirigami.Theme.backgroundColor
    }
    Rectangle {
        anchors.fill: parent
        visible: view.tabIds.length === 0
        color: Kirigami.Theme.backgroundColor
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // With no tabs the bar and its "+" hide; the placeholder takes the pane.
        RowLayout {
            id: barRow

            Layout.fillWidth: true
            spacing: 0
            visible: view.tabIds.length > 0

            // Created with the first tab: the style's empty TabBar reads contentModel.get(0) (TypeError).
            Loader {
                id: barLoader

                Layout.fillWidth: true
                active: view.tabIds.length > 0

                sourceComponent: QQC2.TabBar {
                    id: bar

                    clip: true // tabs that do not fit scroll under the edge, not under the "+"
                    currentIndex: view.current

                    Repeater {
                        model: view.tabIds

                        QQC2.TabButton {
                            id: button

                            objectName: "tabButton"

                            required property int index
                            required property int modelData
                            readonly property var tab: view.area.tabById(modelData)

                            // The terminal sets textColor for a logging tab; the style cannot paint it, so
                            // such a tab's title and icon are the row below.
                            readonly property bool tinted: tab !== null && tab.textColor.a > 0

                            // The style paints the text and the state icon (QTabBar); the close
                            // button sits at the right end, as with tabsClosable.
                            width: Math.max(implicitWidth, tinted ? tintedRow.implicitWidth + 2 * Kirigami.Units.gridUnit : 0) + closeButton.width + Kirigami.Units.smallSpacing
                            focusPolicy: Qt.TabFocus
                            text: tab && !tinted ? tab.text : ""
                            icon.name: tab && !tinted ? tab.iconName : ""
                            Accessible.name: tab ? tab.text : ""
                            // QTabBar has no automatic mnemonics; Alt+letter belongs to the terminal.
                            Kirigami.MnemonicData.enabled: false
                            QQC2.ToolTip.visible: hovered && tab !== null && tab.toolTip !== ""
                            QQC2.ToolTip.text: tab ? tab.toolTip : ""
                            QQC2.ToolTip.delay: 700

                            RowLayout {
                                id: tintedRow

                                anchors.centerIn: parent
                                visible: button.tinted
                                spacing: Kirigami.Units.smallSpacing

                                Kirigami.Icon {
                                    source: button.tab ? button.tab.iconName : ""
                                    visible: source !== ""
                                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                                }
                                QQC2.Label {
                                    text: button.tab ? button.tab.text : ""
                                    color: button.tab ? button.tab.textColor : "transparent"
                                }
                            }

                            // The connection's environment along the top of its tab.
                            EnvMark {
                                width: button.width
                                height: 3
                                environment: button.tab ? button.tab.environment : ""
                            }

                            // This tab receives MultiExec input.
                            Rectangle {
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                height: 3
                                visible: button.tab !== null && button.tab.multiexecTarget
                                color: Kirigami.Theme.neutralTextColor
                            }

                            QQC2.ToolButton {
                                id: closeButton

                                anchors.right: parent.right
                                anchors.rightMargin: Kirigami.Units.smallSpacing
                                anchors.verticalCenter: parent.verticalCenter
                                width: Kirigami.Units.iconSizes.small + 2 * padding
                                height: width
                                icon.name: "window-close-symbolic" // the QTabBar close button, not the "Close tab" action icon
                                display: QQC2.AbstractButton.IconOnly
                                flat: true
                                focusPolicy: Qt.NoFocus
                                padding: 1
                                Accessible.name: qsTr("Close tab")
                                // Later: closing destroys this delegate.
                                onClicked: Qt.callLater(view.area.closeTab, button.tab)
                            }

                            onClicked: view.area.select(view, index)
                            // Double-click and middle click are wired on the panes' bars only.
                            onDoubleClicked: if (view.paneId >= 0)
                                view.rename(tab)
                            Keys.onMenuPressed: view.openMenu(tab, button)
                            Keys.onPressed: event => {
                                if (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier)) {
                                    view.openMenu(tab, button);
                                    event.accepted = true;
                                }
                            }

                            // A middle click closes the tab.
                            TapHandler {
                                enabled: view.paneId >= 0
                                acceptedButtons: Qt.MiddleButton
                                onTapped: Qt.callLater(view.area.closeTab, button.tab)
                            }
                            TapHandler {
                                acceptedButtons: Qt.RightButton
                                onTapped: {
                                    view.area.select(view, button.index);
                                    view.openMenu(button.tab);
                                }
                            }

                            // Drag the tab to reorder it or dock it (PanesArea decides where).
                            // Past the platform's drag threshold only, so clicks stay clicks. Out
                            // of the window it turns into a system drag (other windows, detach).
                            DragHandler {
                                id: tabDragHandler

                                target: null
                                acceptedButtons: Qt.LeftButton
                                grabPermissions: PointerHandler.CanTakeOverFromAnything
                                onActiveChanged: {
                                    if (active)
                                        view.area.beginTabDrag(button.tab, view, tabDragHandler);
                                    else
                                        Qt.callLater(view.area.endHandlerDrag); // the drop may destroy this delegate
                                }
                                onCentroidChanged: {
                                    if (!active)
                                        return;
                                    const s = button.mapToItem(null, centroid.position);
                                    const w = button.Window.window;
                                    if (s.x < 0 || s.y < 0 || s.x >= w.width || s.y >= w.height)
                                        view.area.startSystemDrag();
                                    else
                                        view.area.updateTabDrag(button.mapToItem(view.coords, centroid.position), view.coords);
                                }
                            }
                        }
                    }
                }
            }

            // A click opens the default shell in *this* pane, press and hold picks the shell.
            QQC2.ToolButton {
                visible: view.paneId >= 0
                icon.name: "list-add"
                display: QQC2.AbstractButton.IconOnly
                focusPolicy: Qt.TabFocus
                Accessible.name: qsTr("New local shell in this pane")
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: qsTr("New local shell in this pane (press and hold: choose the shell)")
                QQC2.ToolTip.delay: 700
                Layout.leftMargin: 2
                Layout.rightMargin: 6
                onPressed: view.area.setActive(view.paneId)
                onClicked: view.area.newLocalShell("")
                onPressAndHold: view.popupEntries(view.area.shellMenuEntries(), this)
                Keys.onMenuPressed: view.popupEntries(view.area.shellMenuEntries(), this)
            }
        }

        Item {
            id: stack

            Layout.fillWidth: true
            Layout.fillHeight: true
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(implicitWidth, parent.width)
        visible: view.tabIds.length === 0

        Image {
            source: "qrc:/qt/qml/Termigator/qml/assets/termigator-icon.svg"
            sourceSize.width: 96
            sourceSize.height: 96
            fillMode: Image.PreserveAspectFit
            smooth: true
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: 96
            Layout.preferredHeight: 96
        }
        QQC2.Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            text: qsTr("No terminal open\n\nDouble-click a host, or press Ctrl+Shift+T for a local shell.")
        }
    }

    // The target of Split, Close pane and "+" while the view is split.
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 2
        z: 2
        visible: view.accent
        color: Kirigami.Theme.highlightColor
    }

    // Any press in the pane (tab bar, empty pane, a view) makes it the active one, then goes on
    // to whatever is under it — a tab bar or an empty pane never takes the focus that the active pane follows.
    MouseArea {
        anchors.fill: parent
        z: 3
        acceptedButtons: Qt.AllButtons
        onPressed: mouse => {
            if (view.paneId >= 0)
                view.area.setActive(view.paneId);
            mouse.accepted = false;
        }
    }

    QQC2.Menu {
        id: menu
    }

    Component {
        id: itemComponent

        QQC2.MenuItem {
            property var fn

            onTriggered: fn()
        }
    }

    Component {
        id: separatorComponent

        QQC2.MenuSeparator {}
    }

    QQC2.Dialog {
        id: paneRenameDialog

        property var tab: null

        title: qsTr("Rename tab")
        modal: true
        anchors.centerIn: QQC2.Overlay.overlay
        standardButtons: QQC2.Dialog.Ok | QQC2.Dialog.Cancel
        onOpened: {
            paneRenameField.selectAll();
            paneRenameField.forceActiveFocus();
        }
        onAccepted: view.area.applyRename(tab, paneRenameField.text)
        onClosed: view.focusCurrent()

        ColumnLayout {
            QQC2.Label {
                text: qsTr("Tab title:")
            }
            QQC2.TextField {
                id: paneRenameField

                Layout.minimumWidth: 280
                Accessible.name: qsTr("Tab title")
                onAccepted: paneRenameDialog.accept()
            }
        }
    }
}
