import QtQuick
import QtQuick.Window
import Termigator
import Termigator.Terminal

// One terminal tab's session. It belongs to the tab (a QObject child of its PaneTab, kept in
// `tab.session`), not to a view: the views (TerminalView.qml) come and go when the tab moves, the process, the
// scrollback and everything here stay.
//
// `core` (TerminalCore, terminal_qobj.rs) runs the process and the state machine; this file
// carries out the machine's effects on the tab, the overlay and the neighbouring tabs, and applies
// the settings. Views read: ext (the ExternalSession they draw), look and fontFamily / pointSize /
// scheme, overlayText, finished. The Files area reads remoteCwd (OSC 7).
Item {
    id: s

    visible: false

    required property var tab
    required property var host
    readonly property alias core: core
    readonly property alias ext: ext
    readonly property var area: host.panes
    // Settings as the core read them (TerminalCore.look): history, family, size, scheme and the
    // clipboard / cursor options.
    property var look: ({
            "history": 150000,
            "family": "Monospace",
            "size": 11,
            "scheme": "",
            "blinking_cursor": false,
            "bracketed_paste": true,
            "confirm_multiline": true,
            "copy_on_select": false,
            "paste_on_right_click": false,
            "paste_on_middle_click": true,
            "attention": true
        })
    property string fontFamily: look.family
    property int fontSize: look.size
    // What views draw with: the configured size and the tab's zoom (tab.zoom), so a new view (split,
    // drag, a new place in the tab bar) starts at the size the old one had.
    readonly property int pointSize: Math.max(1, fontSize + tab.zoom)
    property string scheme: look.scheme
    property bool finished: false
    property string overlayText: ""
    property string stateIcon: ""
    property string stateTip: ""
    property bool logOn: false
    property string recordPath: ""
    // The directory the shell reported with OSC 7 ("" until it does).
    property string remoteCwd: ""
    readonly property bool overlayActive: overlayText !== ""
    readonly property string connJson: tab.conn !== null && typeof tab.conn === "object" ? JSON.stringify(tab.conn) : ""

    signal remoteDirectoryChanged(string path)

    // A local shell's working folder (the Files panel follows it), "" when unknown.
    function localCwd() {
        // Linux reads the shell's folder itself; Windows maps the one the shell reported with OSC 7
        // (cmd, PowerShell, WSL: local_shell.rs windows_local_dir).
        return core.localCwd(remoteCwd);
    }

    function readLook() {
        return JSON.parse(core.look(connJson, host.schemes(), host.dark));
    }

    // Applies settings to an open tab: font / scheme / scrollback only when they changed (a
    // tab zoomed by hand keeps its zoom otherwise, a new font size starts unzoomed); the options at once.
    function reapply() {
        const n = readLook();
        ext.historySize = n.history;
        ext.monitorActivity = n.attention;
        if (n.size !== look.size)
            tab.zoom = 0;
        look = n;
    }

    // Ctrl+= / Ctrl+- / Ctrl+0 (ADR-11), one point a step, not below 6 points.
    function zoomIn() {
        tab.zoom += 1;
    }

    function zoomOut() {
        if (pointSize > 6)
            tab.zoom -= 1;
    }

    function resetZoom() {
        tab.zoom = 0;
    }

    function startRecording(path, title) {
        const err = core.startRecording(path, title);
        if (err === "")
            recordPath = path;
        return err;
    }

    function stopRecording() {
        core.stopRecording();
        recordPath = "";
    }

    function saveOutput() {
        host.askSave(qsTr("Save terminal output"), core.homeFile("termigator-terminal-output.txt"), [qsTr("Text files (*.txt)"), qsTr("All files (*)")], path => {
            const err = core.saveOutputText(path, ext.history);
            if (err !== "")
                area.message(err);
        });
    }

    // Replaces this tab by a fresh session in the same slot (the R key, auto-reconnect). The
    // running log goes on in the new tab the attempt count with it.
    function replace(log, attempt) {
        host.carry = {
            "log": log || "",
            "attempt": attempt
        };
        try {
            area.restartTab(tab);
        } finally {
            host.carry = null;
        }
    }

    // _mark_attention: bell / activity in the background mark the tab; a bell also alerts the window.
    function attention(bell) {
        if (!look.attention)
            return;
        const pv = area.viewOf(tab);
        const win = pv ? pv.Window.window : null;
        const current = pv !== null && pv.currentTab === tab;
        const active = win !== null && win.active;
        if (bell)
            core.bell(current, active);
        else
            core.activity(current, active);
    }

    function apply(e) {
        switch (e.k) {
        case "startup":
            ext.sendText(e.v);
            break;
        case "title":
            tab.text = e.v;
            break;
        case "tip":
            stateTip = e.v;
            break;
        case "icon":
            stateIcon = e.v;
            break;
        case "logmarker":
            logOn = e.v;
            break;
        case "overlay":
            overlayText = e.v;
            break;
        case "overlay_hide":
            overlayText = "";
            break;
        case "restart":
            replace(e.log, 0);
            break;
        case "reconnect":
            replace(e.log, e.attempt);
            break;
        case "close":
            area.closeTab(tab);
            break;
        case "save":
            saveOutput();
            break;
        case "alert":
        {
            const pv = area.viewOf(tab);
            if (pv && pv.Window.window)
                pv.Window.window.alert(0); // the taskbar / window-manager urgency hint
            break;
        }
        case "rearm":
            ext.monitorActivity = true; // Konsole reports activity once per arming
            break;
        case "notify":
            if (!tab.closingByUser && area.disconnectNotice)
                area.disconnectNotice(tab, e.label, e.lived);
            break;
        case "warn":
            area.message(e.v);
            break;
        }
    }

    function begin() {
        const c = host.carry;
        host.carry = null;
        look = readLook();
        ext.historySize = look.history;
        ext.monitorActivity = look.attention;
        const request = {
            "session_id": ext.sessionId,
            "inner": tab.inner,
            "env": tab.extraEnv || {},
            "conn": typeof tab.conn === "object" ? tab.conn : null,
            "title": tab.title,
            "attempt": c ? c.attempt : 0,
            "carry_log": c ? c.log : "",
            "cols": 80,
            "rows": 24
        };
        const err = core.start(JSON.stringify(request));
        if (err !== "") {
            // A critical box and no tab.
            finished = true;
            area.message(err);
            Qt.callLater(() => area.closeTab(tab));
            return;
        }
        pollTimer.start();
        host.sessionBegan(tab);
    }

    function teardown() {
        pollTimer.stop();
        deadline.stop();
        core.stopLog();
        stopRecording();
        core.close();
        finished = true;
        host.refreshMultiexec();
    }

    ExternalSession {
        id: ext
    }

    TerminalCore {
        id: core

        onEffects: json => {
            for (const e of JSON.parse(json))
                s.apply(e);
        }
        onExited: {
            s.finished = true;
            pollTimer.stop();
            host.refreshMultiexec();
        }
        onTimerArm: ms => {
            if (ms < 0) {
                deadline.stop();
            } else {
                deadline.interval = ms;
                deadline.restart();
            }
        }
        onDirectoryChanged: path => {
            s.remoteCwd = path;
            s.remoteDirectoryChanged(path);
        }
        // _on_distro_detected: only for SSH entries opened from the list.
        onDistroDetected: id => {
            if (!s.tab.detectDistro || typeof s.tab.conn !== "object" || s.tab.conn === null)
                return;
            if (core.rememberDistro(s.connJson, id)) {
                s.tab.conn = Object.assign({}, s.tab.conn, {
                    "distro": id
                });
                s.host.shell.controller.persist();
            }
        }
    }

    Connections {
        target: s.ext

        function onBell() {
            s.attention(true);
        }
        function onActivity() {
            s.attention(false);
        }
    }

    Connections {
        target: s.tab

        function onClosing() {
            s.teardown();
            // After the views (PanesArea destroys them right after this signal): they draw `ext`.
            Qt.callLater(s.destroy);
        }
        function onLogRequested() {
            s.host.toggleLog(s.tab);
        }
        function onRecordingRequested() {
            s.host.toggleRecording(s.tab);
        }
        function onMultiexecExcludedChanged() {
            s.host.refreshMultiexec();
        }
    }

    // Settings → Apply: the terminal settings of the new config.
    Connections {
        target: s.host.shell.controller

        function onConfigChanged() {
            s.reapply();
        }
    }

    // The process is watched by asking for its exit status once a second (the bridge's EOF asks too).
    Timer {
        id: pollTimer

        interval: 1000
        repeat: true
        onTriggered: core.poll()
    }

    // The state machine's next deadline (startup commands, the reconnect countdown).
    Timer {
        id: deadline

        onTriggered: core.tick()
    }

    // What the tab shows follows the session: the state icon, the recording's red dot, the log's red title.
    Binding {
        target: s.tab
        property: "iconName"
        value: s.recordPath !== "" ? "media-record" : s.stateIcon
    }
    Binding {
        target: s.tab
        property: "toolTip"
        value: s.recordPath !== "" ? qsTr("Recording to %1").arg(s.recordPath) : s.stateTip
    }
    Binding {
        target: s.tab
        property: "textColor"
        value: s.logOn ? "#d9534f" : "transparent"
    }
    Binding {
        target: s.tab
        property: "logActive"
        value: s.logOn
    }
    Binding {
        target: s.tab
        property: "recordingActive"
        value: s.recordPath !== ""
    }
    Binding {
        target: s.tab
        property: "live"
        value: !s.finished
    }

    Component.onCompleted: {
        ext.sessionId = host.takeId();
        tab.session = s;
        begin();
    }
}
