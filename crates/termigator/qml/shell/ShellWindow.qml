pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQuick.Window
import Qt.labs.platform as Platform
import org.kde.kirigami as Kirigami
import Termigator

// Main window.
//
// Where the other areas plug in:
//   hostsPanel / filesPanel      side-tab contents (host sidebar, Files) — placeholders below
//   panesComponent               the terminal panes (qml/panes/PanesArea.qml)
//   shellActions.handle(id, fn)  menu, toolbar and window-shortcut actions (ShellActions.qml)
//   quickConnectHandler          set by panes/terminal: fn(text) -> error text, "" once a tab opened
//   Opening tabs — panes/terminal set these, the sidebar (and others) call them:
//   openConnection               fn(entryJson): a connection or favorite entry
//                                (_open_connection_at / _open_favorite_at)
//   openInner                    fn(inner, title, mode, extraEnvJson): a command (_open_inner;
//                                extraEnvJson "" or a JSON object of strings)
//   newLocalShell                fn(shellPath): "" = the default shell (_new_local_shell)
//   openFilesTab                 Files sets: fn(entryJson) the Files panel for an entry (_open_files_tab)
//   scpTransfer                  Files sets, sidebar calls: fn(entryJson, upload) the host menu's one-shot scp
//   panesArea                    the loaded panesComponent (PanesArea): Files follows its activeTab
//   sideTabBar / filesHost       the side dock's tab bar (Files switches Hosts / Files) and the FilesHost
//   openEditor                   editors set, sidebar calls: fn(kind, entryJson, defaultGroup, onAccepted)
//                                kind: ssh|rdp|vnc|telnet|serial|favorite; entryJson: the entry
//                                (Value::to_json) or "" for a new one; onAccepted(dataJson) after
//                                OK — the sidebar validates, stores and persists
//   saveToHosts                  sidebar sets, panes call: fn(conn) the tab menu's "Add to Hosts…"
//                                (conn: PaneTab.conn)
//   openBulkEdit                 tools set, sidebar calls: fn(count, groups, done) — "Edit N entries…"; done(changes)
//                                after OK when something was ticked
//   panes                        panes set: the PanesArea (tools hang disconnectNotice / testConnection on it)
//   unlockVault                  vault sets: fn(retry, cancelled) asks for the master password when the vault is
//                                locked; retry() once unlocked, else
//                                cancelled() (optional). PanesArea.unlockVault follows it; Files (scp)
//                                and tunnels call it before they reveal a saved password.
//   liveSessions, activeTransfers, runningTunnels   panes / Files / tools, for the close question
//   multiexecTargets             terminal: the number of MultiExec targets while it is on, else -1
//   listSummary                  sidebar: the status-bar summary of the host list
//   shuttingDown()               the window is closing for good: snapshot, stop jobs
//   notify(text)                 a success message in the status bar for 6 s
QQC2.ApplicationWindow {
    id: root

    readonly property alias controller: appController
    readonly property alias shellActions: appActions
    readonly property alias followTimer: followTimer
    readonly property alias closeBox: closeMessageBox
    readonly property alias contentArea: split
    readonly property alias trayQuitItem: trayQuit
    readonly property alias tools: tools
    readonly property alias sideTabBar: sideTabs
    readonly property alias filesHost: filesHost
    readonly property var panesArea: panesLoader.item
    // The update banner (tests read its version).
    readonly property var updates: updateBanner

    // The host list (qml/sidebar); it binds listSummary itself.
    property Component hostsPanel: Component {
        HostsPanel {
            shell: root
        }
    }
    // The Files side tab (qml/files); FilesHost sets openFilesTab / scpTransfer.
    property Component filesPanel: FilesSidePanel {
        shell: root
    }
    property Component panesComponent: PanesArea {
        shell: root
        unlockVault: root.unlockVault ? ((retry, cancelled) => root.unlockVault(retry, cancelled)) : null
    }
    property var quickConnectHandler: null
    property var openConnection: null
    property var openUrl: url => Qt.openUrlExternally(url) // the smoke check replaces it (no xdg-open)
    property var openInner: null
    property var newLocalShell: null
    property var openFilesTab: null
    property var scpTransfer: null
    property var openEditor: null
    property var openBulkEdit: null
    property var saveToHosts: null
    property var panes: null
    property var unlockVault: null
    property int liveSessions: 0
    property int activeTransfers: 0
    property int runningTunnels: 0
    property int multiexecTargets: -1
    property string multiexecNames // the targets' labels for the banners
    property string listSummary: ""

    property bool toolbarVisible: appController.windowBool("toolbar", true)
    property bool sidebarVisible: true
    // The side dock can narrow far (the Files toolbar hides icons behind its » button), and the
    // window still leaves the panes their 360 px minimum plus the splitter handle.
    readonly property real sideMinimumWidth: Math.max(140, filesLoader.item ? filesLoader.item.minimumPanelWidth : 0)
    readonly property real contentMinimumWidth: 360
    property string statusMessage: ""
    property Window shortcutsWindow: null

    signal shuttingDown

    function notify(text) {
        statusMessage = text;
        statusTimer.restart();
    }

    // A balloon from the tray icon; nothing when there is no tray.
    function trayMessage(title, text) {
        if (tray.visible && tray.supportsMessages)
            tray.showMessage(title, text, Platform.SystemTrayIcon.Warning, 5000);
    }

    function toggleVisible() {
        if (visible) {
            hide();
        } else {
            showNormal();
            raise();
            requestActivate();
        }
    }

    // File → Quit, the tray's Quit and the close button: the confirmation, then finishClose().
    // Not through close(): a window hidden in the tray gets no closing signal.
    // Preferences → Restart now (a new interface style): the same quit, then a new process that
    // reopens the tabs. A "No" to the confirmation cancels the restart too.
    property bool restarting: false
    // Preferences → Colours as this process started (PreferencesDialog notes it on first open).
    property string runningTheme: ""
    // The style / colours choice the restart question was last asked for (asked once per choice).
    property string restartAskedFor: ""
    readonly property alias restartBox: restartMessageBox

    function askRestart() {
        restartMessageBox.open();
    }

    function restart() {
        restarting = true;
        requestQuit();
    }

    function requestQuit() {
        const question = appController.closeQuestion(liveSessions, activeTransfers, runningTunnels);
        if (question === "") {
            finishClose();
        } else {
            closeMessageBox.text = question;
            closeMessageBox.open();
        }
    }

    // The geometry of the last normal (not maximized / full screen) state: what is saved, so a
    // maximized window comes back maximized over its own normal size, not as a screen-sized one.
    property rect normalGeometry: Qt.rect(0, 0, 0, 0)
    readonly property bool onWindows: Qt.platform.os === "windows"

    function noteGeometry() {
        // A screen-wide size is a maximize in progress (the size may change before visibility).
        if (visible && visibility === Window.Windowed && width < Screen.desktopAvailableWidth)
            normalGeometry = Qt.rect(x, y, width, height);
    }

    function finishClose() {
        if (restarting)
            appController.requestRestart(); // before shuttingDown(): the tab snapshot is kept
        shuttingDown();
        const g = visibility === Window.Windowed || normalGeometry.width <= 0 ? Qt.rect(x, y, width, height) : normalGeometry;
        appController.setWindowPlacement(g.x, g.y, onWindows, visibility === Window.Maximized);
        appController.saveWindowState(g.width, g.height, side.visible ? side.width : 0);
        tray.visible = false;
        Qt.callLater(Qt.quit);
    }

    title: "Termigator"
    onXChanged: noteGeometry()
    onYChanged: noteGeometry()
    onWidthChanged: noteGeometry()
    onHeightChanged: noteGeometry()
    width: appController.windowInt("width", 900)
    height: {
        const h = appController.windowInt("height", 0);
        return h > 0 ? h : appController.windowInt("min_height", 560);
    }
    minimumWidth: Math.ceil((sidebarVisible ? sideMinimumWidth : 0) + contentMinimumWidth + 8)
    minimumHeight: 360
    flags: appController.windowBool("always_on_top", false) ? (Qt.Window | Qt.WindowStaysOnTopHint) : Qt.Window

    menuBar: QQC2.MenuBar {
        id: menuBar
    }

    header: ShellToolBar {
        visible: root.toolbarVisible
        shellActions: appActions
        controller: appController
        quickConnectHandler: root.quickConnectHandler
        onQuickConnectFailed: message => root.notify(message)
    }

    footer: QQC2.ToolBar {
        position: QQC2.ToolBar.Footer

        QQC2.Label {
            id: statusLabel

            anchors.fill: parent
            text: root.statusMessage !== "" ? root.statusMessage : root.listSummary
            elide: Text.ElideRight
            verticalAlignment: Text.AlignVCenter
            Accessible.name: qsTr("List summary")
        }
    }

    AppController {
        id: appController
    }

    ShellActions {
        id: appActions

        tableJson: appController.actionsJson
    }

    QQC2.SplitView {
        id: split

        anchors.fill: parent
        orientation: Qt.Horizontal

        handle: SplitGrip {}

        // Hosts (session tree) and Files (remote browser of the active SSH tab).
        ColumnLayout {
            id: side

            visible: root.sidebarVisible
            spacing: 0
            QQC2.SplitView.minimumWidth: root.sideMinimumWidth
            QQC2.SplitView.preferredWidth: {
                const lw = appController.windowInt("list_width", 0);
                return lw > 0 ? lw : 280;
            }

            QQC2.TabBar {
                id: sideTabs

                Layout.fillWidth: true

                // The two tabs share the dock's width equally.
                QQC2.TabButton {
                    width: sideTabs.width / 2
                    text: qsTr("Hosts")
                }
                QQC2.TabButton {
                    width: sideTabs.width / 2
                    text: qsTr("Files")
                }
            }

            StackLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                currentIndex: sideTabs.currentIndex

                Loader {
                    sourceComponent: root.hostsPanel
                }
                Loader {
                    id: filesLoader

                    sourceComponent: root.filesPanel
                }
            }
        }

        ColumnLayout {
            spacing: 0
            QQC2.SplitView.fillWidth: true
            QQC2.SplitView.minimumWidth: root.contentMinimumWidth

            // A newer release on GitHub (window.check_updates).
            UpdateBanner {
                id: updateBanner

                Layout.fillWidth: true
            }

            // MultiExec banner above the terminals while it is on.
            MultiExecBanner {
                Layout.fillWidth: true
                shell: root
            }

            Loader {
                id: panesLoader

                Layout.fillWidth: true
                Layout.fillHeight: true
                sourceComponent: root.panesComponent
            }
        }
    }

    // Files tabs, the host menu's scp and the transfer count.
    FilesHost {
        id: filesHost

        shell: root
    }

    Timer {
        id: statusTimer

        interval: 6000
        onTriggered: root.statusMessage = ""
    }

    // window.follow_konsole: the window follows the external terminal;
    // decided once at start.
    Timer {
        id: followTimer

        interval: appController.followPollMs()
        running: appController.followEnabled()
        repeat: true
        onTriggered: appController.followTick(root.width)
    }

    Connections {
        function onFollowMove(x, y) {
            root.x = x;
            root.y = y;
        }

        target: appController
    }

    // shortcut: polls the control socket every 100 ms (a non-blocking try_recv); a queued
    // signal from the accept thread would need the server moved off this object.
    Timer {
        interval: 100
        running: true
        repeat: true
        onTriggered: appController.pollControl()
    }

    Connections {
        target: appController

        function onRemoteCommand(command) {
            if (command === "toggle" && root.visible && root.visibility !== Window.Minimized && root.active) {
                root.hide();
                return;
            }
            if (root.visibility === Window.Minimized)
                root.showNormal();
            else
                root.show();
            root.raise();
            root.requestActivate();
        }

        function onWarning(text) {
            warningBox.text = text;
            warningBox.open();
        }
    }

    onClosing: close => {
        close.accepted = false;
        root.requestQuit();
    }

    MessageBox {
        id: closeMessageBox

        title: "Termigator"
        buttons: MessageDialog.Yes | MessageDialog.No
        onButtonClicked: (button, role) => {
            if (button === MessageDialog.Yes)
                root.finishClose();
            else
                root.restarting = false;
        }
    }

    MessageBox {
        id: restartMessageBox

        title: "Termigator"
        text: qsTr("The new interface style takes effect after a restart. Restart Termigator now?\n\nThe open tabs are reopened: local shells start afresh, remote sessions connect again.")
        buttons: MessageDialog.Yes | MessageDialog.No
        onButtonClicked: (button, role) => {
            if (button === MessageDialog.Yes) {
                if (root.preferencesDialog)
                    root.preferencesDialog.close();
                root.restart();
            }
        }
    }

    MessageBox {
        id: warningBox

        title: "Termigator"
        icon: "warning"
        buttons: MessageDialog.Ok
    }

    MessageBox {
        id: firstRunBox

        title: "Termigator"
        text: appController.firstRunMessage
        buttons: MessageDialog.Ok
        checkBoxText: qsTr("Check for new versions on GitHub at startup (once a day)")
        checked: true
        // Any way out of the welcome (OK, Esc) stores the choice; then the first check may run.
        onButtonClicked: {
            appController.setWindowBool("check_updates", checked);
            appController.persist();
            updateTimer.start();
        }
    }

    // The update check waits a little: the window and the tabs come first.
    Timer {
        id: updateTimer

        interval: 3000
        onTriggered: updateBanner.start()
    }

    MessageBox {
        id: tipsBox

        title: qsTr("Hosts & favorites")
        buttons: MessageDialog.Ok
        text: "• Double-click a row or press Enter — each entry uses its saved terminal mode (edit the entry to change embedded vs external).\n\n"
            + "• With focus on the list, press Enter to connect to the selected row.\n\n"
            + "• Reorder: right-click → Move up / Move down, or Alt+Up / Alt+Down. Duplicate: right-click → Duplicate entry.\n\n"
            + "• Folders: nest them with / in the name (e.g. prod/web). Drag an entry onto a folder to move it there; drag to empty space for top level. Right-click a folder for New subfolder, Rename / move, Dissolve or Delete; right-click empty space → New folder. Set a Group when adding/editing an entry, or right-click → Move to folder… Click the arrow to collapse/expand; the state is remembered.\n\n"
            + "• Ctrl+Shift+H (anywhere) or Ctrl+L / Ctrl+F (in the list) focuses the search; Escape clears it.\n\n"
            + (Qt.platform.os === "windows" ? "" : "• Embedded = terminal inside Termigator (PTY). External = your launch.template (e.g. Konsole).\n\n")
            + "• Filter narrows the list as you type (SSH + favorites).\n\n"
            + "• Split panes: View → Split right (Ctrl+( ) / Split down (Ctrl+) ) split the focused pane; splits nest, so 2x2 grids and beyond just work. The current tab moves into the new pane (or the pane starts empty). Close pane: Ctrl+Shift+X; a pane whose last tab closes disappears on its own.\n\n"
            + "• Double-click a terminal tab to rename it.\n\n"
            + "• SSH DevOps: edit a connection, open the \"Forwarding\" tab for a jump host (-J), -L/-R/-D tunnels, agent/X11, optional remote command and host key checking. Right-click a host or Edit → Copy connection command copies the exact line to the clipboard.\n\n"
            + "• Quick connect (File menu, Ctrl+Shift+O): connect as user@host[:port] without saving.\n\n"
            + "• Import MobaXterm sessions (File → Import or + above the list): pick an exported .ini to merge its SSH bookmarks into the host list.\n\n"
            + "• Session log / broadcast: Session menu or terminal tab bar — log embedded output to a file, broadcast one line to all embedded tabs. All keyboard shortcuts: Help → Keyboard shortcuts."
    }

    Component {
        id: shortcutsDialog

        ShortcutsDialog {
            transientParent: root
            rows: JSON.parse(appController.shortcutRowsJson)
            onClosing: destroy()
        }
    }

    // Settings → Configure Termigator… — created on first use: the dialog is the heaviest QML
    // singleton in the window (six pages, ~75 ms / ~8 MB of startup) and most runs never open it.
    readonly property var preferencesDialog: prefsLoader.item

    function openPreferences() {
        if (!prefsLoader.item)
            prefsLoader.active = true;
        if (prefsLoader.item)
            root.preferencesDialog.open();
    }

    Component {
        id: preferencesComponent

        PreferencesDialog {
            shell: root
            onApplied: {
                const top = root.controller.windowBool("always_on_top", false);
                if (((root.flags & Qt.WindowStaysOnTopHint) !== 0) !== top) {
                    root.flags = top ? (Qt.Window | Qt.WindowStaysOnTopHint) : Qt.Window;
                    root.show(); // changing the flags hides a window
                }
                tray.visible = root.controller.windowBool("tray_icon", true) && tray.available;
                root.controller.persist();
            }
        }
    }

    Loader {
        id: prefsLoader

        active: false
        sourceComponent: preferencesComponent
    }

    // Connection editors; sets openEditor and handles File → Quick connect….
    EditorHost {
        id: editors

        shell: root
    }

    // Tunnels, snippets, bulk edit, the disconnect notice.
    ToolsHost {
        id: tools

        shell: root
    }

    // The master password, Tools → Credentials… and Tools → SSH keys…; sets unlockVault.
    readonly property alias vaultHost: vaultHost

    VaultHost {
        id: vaultHost

        shell: root
    }

    // File → Import / Export (qml/import); needs the vault host's message boxes.
    readonly property alias importHost: importHost

    ImportHost {
        id: importHost

        shell: root
    }

    // Tray icon toggling the window (window.tray_icon).
    Platform.SystemTrayIcon {
        id: tray

        visible: appController.windowBool("tray_icon", true) && available
        icon.source: appController.trayIconUrl
        icon.name: appController.trayIconName !== "" ? appController.trayIconName
                   : appController.trayIconUrl === "" ? "utilities-terminal" : ""
        tooltip: "Termigator"
        menu: Platform.Menu {
            Platform.MenuItem {
                text: qsTr("Show / Hide")
                onTriggered: root.toggleVisible()
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                id: trayQuit

                text: qsTr("Quit")
                onTriggered: root.requestQuit()
            }
        }
        onActivated: reason => {
            if (reason === Platform.SystemTrayIcon.Trigger)
                root.toggleVisible();
        }
    }

    // Smoke-test mode: SmokeCheck.qml (tests/smoke.rs), or an area's check from qml/ given as
    // --smoke-check=<area>/<File>.qml (tests/common/mod.rs).
    Loader {
        active: appController.smokeTestShot !== ""
        Component.onCompleted: {
            const arg = Qt.application.arguments.find(a => a.startsWith("--smoke-check="));
            // --warm-up (lib.rs): quit at the first frame.
            const file = arg ? "../" + arg.slice("--smoke-check=".length) : Qt.application.arguments.includes("--warm-up") ? "StartupCheck.qml" : "SmokeCheck.qml";
            setSource(Qt.resolvedUrl(file), {
                "shell": root
            });
        }
    }

    Component.onCompleted: {
        appActions.populate(menuBar);
        const a = appActions;
        a.handle("file.quit", () => root.requestQuit());
        a.handle("settings.configure", () => root.openPreferences());
        a.handle("file.open_config_folder", () => {
            const url = appController.configFolderUrl();
            if (url.toString() !== "")
                root.openUrl(url);
        });
        a.handle("view.toolbar", checked => {
            root.toolbarVisible = checked;
            appController.setWindowBool("toolbar", checked);
            appController.persist();
        }, () => root.toolbarVisible);
        a.handle("view.sidebar", () => root.sidebarVisible = !root.sidebarVisible, () => root.sidebarVisible);
        a.handle("view.fullscreen", () => {
            if (root.visibility === Window.FullScreen)
                root.showNormal();
            else
                root.showFullScreen();
        }, () => root.visibility === Window.FullScreen);
        a.handle("help.shortcuts", () => {
            root.shortcutsWindow = shortcutsDialog.createObject(root);
            root.shortcutsWindow.show();
        });
        a.handle("help.host_list_tips", () => tipsBox.open());
        a.handle("help.about", () => appController.aboutTermigator(qsTr("About Termigator")));
        a.handle("help.about_qt", () => appController.aboutQt());

        // width / height are the inside of the window: on Windows the title bar and the frame come
        // on top, so leave room for them or the title bar ends up above the screen.
        // Elsewhere the window manager fits and places the window itself.
        const frame = onWindows ? 40 : 0;
        if (onWindows) {
            width = Math.max(minimumWidth, Math.min(width, Screen.desktopAvailableWidth));
            height = Math.max(minimumHeight, Math.min(height, Screen.desktopAvailableHeight - frame));
        }
        const sx = appController.windowInt("x", -1000000);
        const sy = appController.windowInt("y", -1000000);
        const onScreen = sx > Screen.virtualX - width / 2 && sx < Screen.virtualX + Screen.desktopAvailableWidth - 100 && sy >= Screen.virtualY + frame / 2 && sy < Screen.virtualY + Screen.desktopAvailableHeight - 100;
        if (onWindows && onScreen) {
            x = sx;
            y = sy;
        } else {
            x = Screen.virtualX + (Screen.desktopAvailableWidth - width) / 2;
            y = Screen.virtualY + frame + (Screen.desktopAvailableHeight - frame - height) / 2;
        }
        normalGeometry = Qt.rect(x, y, width, height);
        if (appController.windowBool("maximized", false))
            showMaximized();
        else
            visible = true;
        if (appController.firstRunMessage !== "")
            firstRunBox.open();
        else
            updateTimer.start();
    }

    AppPalette {
        target: root
    }
}
