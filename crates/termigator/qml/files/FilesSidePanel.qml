import QtQuick
import Termigator

// The Files side tab: side-dock switching. An active SSH tab brings the Files tab to the front and
// points the panel at its connection, a local shell shows this machine's files there; anything
// else (RDP, no tabs) brings Hosts back. The panel follows the folder the active session reports with OSC 7 (`remoteCwd`). A Files
// tab is its own browser, so the side panel rests for it.
Item {
    id: side

    required property var shell
    readonly property var host: shell.filesHost
    readonly property var area: shell.panesArea
    readonly property var tab: area ? area.activeTab : null
    property var panel: null
    property var session: null
    // The side dock's minimum width (ShellWindow.qml): the panel's toolbar must fit on one row.
    readonly property real minimumPanelWidth: panel ? panel.minimumPanelWidth : 0

    // The Files panel is the heavy part of this tab: create it on the first Files tab / SSH
    // session, not at startup (the benchmark showed ~45 ms and ~5 MB of window startup).
    function ensurePanel() {
        if (panel)
            return;
        if (!session)
            session = host.newSession("files_two_pane_side");
        panel = panelComponent.createObject(side, {
            "session": session,
            "files": session.files,
            "transfers": session.transfers
        });
        panel.anchors.fill = side;
    }

    // Points the panel at the active tab's connection and shows the matching side tab.
    function update() {
        const t = tab;
        const conn = host.connOf(t);
        if (conn !== null) {
            ensurePanel();
            shell.sideTabBar.currentIndex = 1;
            const json = JSON.stringify(conn);
            session.files.setTarget(json, t.title || helper.entryLabel(json));
            const cwd = t.session ? t.session.remoteCwd : "";
            if (cwd)
                panel.followDirectory(cwd);
        } else if (isLocalShell(t)) {
            // A local shell: the panel shows this machine's files, following the shell's folder.
            ensurePanel();
            session.files.setTarget("", "");
            panel.localMode = true;
            followLocal();
            shell.sideTabBar.currentIndex = 1; // the shell's files in front, as for SSH
        } else {
            if (session)
                session.files.setTarget("", "");
            shell.sideTabBar.currentIndex = 0;
        }
        if (panel && conn !== null)
            panel.localMode = false;
    }

    // An embedded tab that runs a plain command, not a connection.
    function isLocalShell(t) {
        return !!t && t.embedded && t.conn === null && !!t.session;
    }

    // The shell's folder, read every second: only a new folder counts as a cd (it ends a pause
    // the user made by going elsewhere in Files), and the Follow button decides.
    function followLocal() {
        const t = tab;
        if (!panel || !panel.local || !isLocalShell(t))
            return;
        const dir = t.session.localCwd();
        if (dir === "")
            return;
        panel.noteTerminalDir(dir);
        if (panel.followButton.checked && dir !== panel.local.currentDir())
            panel.local.setDir(dir);
    }

    // The shell's folder is read, not pushed: every second while a local shell is in front.
    Timer {
        interval: 1000
        running: side.panel !== null && side.panel.localMode
        repeat: true
        onTriggered: side.followLocal()
    }

    onTabChanged: update()

    Component.onCompleted: {
        if (tab)
            update();
    }

    // A click on the Files tab with no SSH session still shows the panel (and its last state).
    Connections {
        target: side.shell.sideTabBar

        function onCurrentIndexChanged() {
            if (side.shell.sideTabBar.currentIndex === 1)
                side.ensurePanel();
        }
    }

    FilesHelper {
        id: helper
    }

    // "Follow terminal folder": the active session's shell reported a new directory.
    Connections {
        target: side.tab && side.tab.session ? side.tab.session : null
        ignoreUnknownSignals: true

        function onRemoteDirectoryChanged(path) {
            if (side.panel)
                side.panel.followDirectory(path);
        }
    }

    // Made with its session as an initial property: a binding to it would be read once before it is set.
    Component {
        id: panelComponent

        FilesPanel {
            statusInFooter: true
            onLocalFollowRequested: side.followLocal()
        }
    }
}
