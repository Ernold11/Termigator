import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import Termigator
import QtQuick.Window

// Tabs in a window of their own. Its tab menu comes from PanesArea.tabMenuEntries; the main
// window's shortcuts are not live here, only the keys of the detachedWindow context. Closing it asks about
// running sessions, then closes its tabs; PanesArea destroys it once the last tab is gone.
QQC2.ApplicationWindow {
    id: win

    required property var area
    required property int windowId
    readonly property alias view: paneView
    readonly property alias closeBox: closeBox

    function keys(id) {
        const a = area.shell.shellActions.table.actions.find(x => x.id === id);
        return a ? a.keys : [];
    }

    title: paneView.currentTab ? paneView.currentTab.text + " — Termigator" : "Termigator"
    // Not transient for the main window: a transient child keeps that window "active", so its
    // window shortcuts (Ctrl+Shift+T/W/M, F11) would clash with the ones below.
    transientParent: null
    minimumWidth: 280
    minimumHeight: 160
    // See-through terminals like the main window's (ShellWindow.terminalBackground).
    color: area.shell.seeThrough ? "transparent" : palette.window
    header: MultiExecBanner {
        shell: win.area.shell
    }

    PaneView {
        id: paneView

        anchors.fill: parent
        focus: true
        area: win.area
        windowId: win.windowId
    }

    // A dragged tab joins this window's list (no splits here).
    DockOverlay {
        anchors.fill: parent
        z: 10
        area: win.area
        host: paneView
    }

    DropArea {
        anchors.fill: parent
        z: 11
        keys: [win.area.tabMime]
        onEntered: drag => drag.accepted = win.area.tabDragEntered(drag, paneView)
        onPositionChanged: drag => win.area.updateTabDrag(Qt.point(drag.x, drag.y), paneView)
        onExited: win.area.tabDragExited(paneView)
        onDropped: drop => win.area.tabDropped(drop, paneView)
    }

    Shortcut {
        sequence: "Escape"
        enabled: win.area.tabDrag !== null
        onActivated: win.area.cancelTabDrag()
    }

    Shortcut {
        sequences: win.keys("detached.close_tab")
        onActivated: win.area.closeTab(paneView.currentTab)
    }
    Shortcut {
        sequences: win.keys("detached.attach")
        onActivated: win.area.attachTab(paneView.currentTab)
    }
    Shortcut {
        sequences: win.keys("detached.new_tab")
        onActivated: win.area.newShellIn(win.windowId)
    }
    Shortcut {
        sequences: win.keys("detached.multiexec")
        onActivated: win.area.shell.shellActions.actions["session.multiexec"].trigger()
    }
    Shortcut {
        sequences: win.keys("detached.fullscreen")
        onActivated: win.visibility === Window.FullScreen ? win.showNormal() : win.showFullScreen()
    }

    onClosing: close => {
        close.accepted = false;
        win.area.closeWindow(win.windowId);
    }

    MessageBox {
        id: closeBox

        title: "Termigator"
        buttons: MessageDialog.Yes | MessageDialog.No
        onButtonClicked: (button, role) => {
            if (button === MessageDialog.Yes)
                win.area.closeWindowTabs(win.windowId);
        }
    }

    AppPalette {
        target: win
    }
}
