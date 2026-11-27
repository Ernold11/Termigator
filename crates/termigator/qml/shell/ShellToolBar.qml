pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts

// One compact row — New ▾, Quick connect, splits, MultiExec, tunnels, Files, and ☰ with the menus
// while the menu bar is hidden (View → Show menubar, Ctrl+Shift+M; Alt on Windows), as in KDE applications.
// Everything on it is a menu action too, so View → Show toolbar can hide it.
QQC2.ToolBar {
    id: bar

    required property var shellActions
    required property var controller
    property var quickConnectHandler: null
    property bool menuButtonShown: false
    // What the bar shows, so ☰ leaves it out (New ▾ stands for File → New tab / New connection).
    readonly property var shownActions: ({
            "file.new_tab": true,
            "file.new_connection": true,
            "file.quick_connect": true,
            "view.split_right": true,
            "view.split_down": true,
            "session.multiexec": true,
            "tools.tunnels": true,
            "session.files": true
        })

    signal quickConnectFailed(string message)

    component ActionButton: QQC2.ToolButton {
        required property string actionId

        objectName: "tb_" + actionId

        action: bar.shellActions.actions[actionId] ?? null
        display: QQC2.AbstractButton.IconOnly
        Accessible.name: action ? bar.shellActions.plain(action.text) : ""
        QQC2.ToolTip.visible: hovered && action !== null
        QQC2.ToolTip.text: action ? (action.tip || bar.shellActions.plain(action.text)) : ""
        QQC2.ToolTip.delay: 700
    }

    RowLayout {
        anchors.fill: parent
        spacing: 2

        QQC2.ToolButton {
            id: newButton

            objectName: "tbNew"

            readonly property var newTab: bar.shellActions.actions["file.new_tab"] ?? null

            text: qsTr("New")
            icon.name: "tab-new"
            display: QQC2.AbstractButton.TextBesideIcon
            enabled: newTab !== null && newTab.enabled
            onClicked: newTab.trigger()
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: qsTr("New local shell (Ctrl+Shift+T) — arrow: new connection")
            QQC2.ToolTip.delay: 700
        }

        // The arrow half of the split button.
        QQC2.ToolButton {
            objectName: "tbNewMenu"
            icon.name: "arrow-down"
            display: QQC2.AbstractButton.IconOnly
            Accessible.name: qsTr("New connection")
            onClicked: {
                if (!bar.newMenu)
                    bar.newMenu = bar.shellActions.buildMenu("File/New connection", bar);
                bar.newMenu.popup(newButton, 0, newButton.height);
            }
        }

        QuickConnectField {
            controller: bar.controller
            handler: bar.quickConnectHandler
            Layout.minimumWidth: 240
            Layout.maximumWidth: 380
            Layout.preferredWidth: 300
            onFailed: message => bar.quickConnectFailed(message)
        }

        QQC2.ToolSeparator {}
        ActionButton {
            actionId: "view.split_right"
        }
        ActionButton {
            actionId: "view.split_down"
        }
        QQC2.ToolSeparator {}
        ActionButton {
            actionId: "session.multiexec"
        }
        ActionButton {
            actionId: "tools.tunnels"
        }
        ActionButton {
            actionId: "session.files"
        }

        Item {
            Layout.fillWidth: true
        }

        // A new copy on every click: submenus an area fills (Snippets) are current.
        QQC2.ToolButton {
            id: menuButton

            objectName: "tbMenu"
            visible: bar.menuButtonShown
            icon.name: "application-menu"
            display: QQC2.AbstractButton.IconOnly
            Accessible.name: qsTr("Menu")
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: Qt.platform.os === "windows" ? qsTr("Menu (Alt or Ctrl+Shift+M shows the menu bar)") : qsTr("Menu (Ctrl+Shift+M shows the menu bar)")
            QQC2.ToolTip.delay: 700
            onClicked: {
                if (bar.mainMenu)
                    bar.mainMenu.destroy();
                bar.mainMenu = bar.shellActions.buildHamburger(bar, bar.shownActions);
                // Right under the button, right edges aligned, as KHamburgerMenu does.
                const m = bar.mainMenu;
                m.popup(menuButton, menuButton.width - m.implicitWidth, menuButton.height);
            }
        }
    }

    property QQC2.Menu newMenu: null
    property QQC2.Menu mainMenu: null
}
