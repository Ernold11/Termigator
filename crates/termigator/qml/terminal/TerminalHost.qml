import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator
import Termigator.Terminal

// The terminal side of the panes PanesArea instantiates one, makes its `viewComponent` the tabs' content and
// calls nothing else: the rest is the other way round.
//
//   viewComponent        the tab's view (TerminalView.qml), made per place the tab is shown
//   ensureSession(tab)   the tab's TabSession (made on first use, kept in `tab.session`: the
//                        process, the pty, the log; views come and go, the session does not)
//   viewFor(tab)         the tab's TerminalView where it is shown, or null
//   toggleLog / toggleRecording / broadcast / setMultiexec   Session menu, tab menu
//   copyText(text)       the clipboard
//   askSave(title, path, filters, fn)   a save dialog; fn(localPath) once accepted
//
// It registers the handlers of Edit → Copy / Paste / Find, View → Zoom, Session → Log / Record /
// MultiExec / Broadcast on the shell's actions, and keeps `shell.multiexecTargets` and the
// bridge's MultiExec hub in step with the tabs. For the disconnect notice `panes.disconnectNotice`
// is called with (tab, label, livedSeconds) when a connection tab ended on its own and
// `window.notify_on_disconnect` is on; Files follows the shell's directory in `tab.session.remoteCwd`.
Item {
    id: thost

    required property var panes
    readonly property var shell: panes.shell
    readonly property alias viewComponent: viewComponent
    readonly property bool dark: Kirigami.Theme.backgroundColor.hslLightness < 0.5
    property int lastId: 0
    // {log, attempt} for the session made next: a restart or reconnect in the same slot .
    property var carry: null
    property bool multiexecOn: false
    property var saveHandler: null

    visible: false

    function takeId() {
        lastId += 1;
        return lastId;
    }

    // The colour schemes the terminal knows (compiled in + Konsole's directories).
    function schemes() {
        return probe.availableColorSchemes;
    }

    function ensureSession(tab) {
        if (tab.session)
            return tab.session;
        return sessionComponent.createObject(thost, {
            "tab": tab,
            "host": thost
        });
    }

    function viewFor(tab) {
        const pv = tab ? panes.viewOf(tab) : null;
        return pv && pv.views[tab.tabId] ? pv.views[tab.tabId] : null;
    }

    // The tab the terminal commands act on: the active pane's current tab when it is an embedded terminal.
    function currentTerminal() {
        const t = panes.activeTab;
        return t && t.embedded && t.session ? t : null;
    }

    function isConnection(tab) {
        return tab.conn !== null && typeof tab.conn === "object";
    }

    // file: URLs and local paths, right for Windows drive paths too (file:///C:/dir <-> C:/dir).
    function localPath(url) {
        const p = decodeURIComponent(url.toString().replace(/^file:\/\//, ""));
        return /^\/[A-Za-z]:/.test(p) ? p.slice(1) : p;
    }

    function fileUrl(p) {
        const s = /^[A-Za-z]:/.test(p) ? "/" + p.replace(/\\/g, "/") : p;
        return "file://" + s.split("/").map(x => /^[A-Za-z]:$/.test(x) ? x : encodeURIComponent(x)).join("/");
    }

    function askSave(title, path, filters, fn) {
        saveHandler = fn;
        saveDialog.title = title;
        saveDialog.nameFilters = filters;
        saveDialog.currentFile = fileUrl(path);
        saveDialog.open();
    }

    function copyText(text) {
        clipboard.text = text;
        clipboard.selectAll();
        clipboard.copy();
    }

    // ---- the terminal commands (Edit, View)

    function withView(fn) {
        const t = currentTerminal();
        const v = viewFor(t);
        if (v)
            fn(v);
    }

    function find() {
        const t = currentTerminal();
        if (!t) {
            panes.message(qsTr("Find is only available in embedded terminal tabs."));
            return;
        }
        withView(v => v.toggleFind());
    }

    // ---- session log, recording (Session menu, tab menu)

    function toggleLog(tab) {
        const t = tab || currentTerminal();
        if (!t || !t.embedded || !t.session) {
            panes.message(qsTr("Session logging is only available for embedded QTermWidget tabs."));
            return;
        }
        const s = t.session;
        if (s.logOn) {
            s.core.stopLog();
            shell.notify(qsTr("Session log stopped."));
            return;
        }
        askSave(qsTr("Session log file"), s.core.homeFile("termigator-session.log"), [qsTr("Log files (*.log)"), qsTr("All files (*)")], path => {
            const err = s.core.startLog(path);
            if (err !== "")
                panes.message(err);
            else
                shell.notify(qsTr("Logging this tab to %1").arg(path));
        });
    }

    function toggleRecording(tab) {
        const t = tab || currentTerminal();
        if (!t || !t.embedded || !t.session) {
            panes.message(qsTr("Recording is only available for embedded terminal tabs."));
            return;
        }
        const s = t.session;
        if (s.recordPath !== "") {
            const path = s.recordPath;
            s.stopRecording();
            shell.notify(qsTr("Recording saved: %1 — play it with “asciinema play”.").arg(path));
            return;
        }
        const title = t.title || "session";
        askSave(qsTr("Record session (asciinema)"), s.core.recordingDefaultPath(title), [qsTr("asciinema recordings (*.cast)"), qsTr("All files (*)")], path => {
            const err = s.startRecording(path, title);
            if (err !== "")
                panes.message(qsTr("Could not start the recording:\n%1").arg(err));
            else
                shell.notify(qsTr("Recording this tab to %1").arg(path));
        });
    }

    // ---- MultiExec and the broadcast line

    // Live embedded remote sessions not excluded by the user: a local shell stays out (a stray
    // "reboot" must not hit this machine).
    function multiexecTargets() {
        return panes.tabList.filter(t => t.embedded && t.session && !t.session.finished && isConnection(t) && !t.multiexecExcluded);
    }

    function refreshMultiexec() {
        const ts = multiexecOn ? multiexecTargets() : [];
        MultiExec.targets = ts.map(t => t.session.ext.sessionId);
        for (const t of panes.tabList)
            t.multiexecTarget = ts.indexOf(t) >= 0;
        shell.multiexecNames = ts.map(targetLabel).join(", ");
        shell.multiexecTargets = multiexecOn ? ts.length : -1;
    }

    // A session that starts while MultiExec is on stays out until the user includes it
    // (tab menu), so a newly opened host never receives keys meant for the others.
    function sessionBegan(tab) {
        if (multiexecOn && isConnection(tab))
            tab.multiexecExcluded = true;
        refreshMultiexec();
    }

    // "title (user@host)" for the lists that say where input goes; two tabs to one host differ by title.
    function targetLabel(t) {
        const c = isConnection(t) ? t.conn : {};
        const at = c.host ? " (" + (c.user ? c.user + "@" : "") + c.host + ")" : "";
        return (t.title || "?") + at;
    }

    // Up to 12 "  • …" lines.
    function targetListing(targets) {
        const names = targets.map(targetLabel);
        return names.slice(0, 12).map(n => "  • " + n).join("\n") + (names.length > 12 ? "\n  …" : "");
    }

    function setMultiexec(on) {
        multiexecOn = on;
        refreshMultiexec();
    }

    // The same sessions as MultiExec: a tab excluded from one is excluded from both .
    function broadcastTargets() {
        return multiexecTargets();
    }

    function broadcast() {
        broadcastField.text = "";
        broadcastDialog.open();
    }

    function broadcastAsk(text) {
        if (text.trim() === "")
            return;
        const targets = broadcastTargets();
        if (targets.length === 0) {
            panes.message(qsTr("No live remote sessions are open."));
            return;
        }
        const listing = targetListing(targets);
        confirmBox.line = text;
        confirmBox.targets = targets;
        confirmBox.text = qsTr("Send this line to %1 session(s)?\n\n%2\n\n%3").arg(targets.length).arg(text).arg(listing);
        confirmBox.open();
    }

    function broadcastSend(targets, text) {
        const line = text.replace(/\n+$/, "") + "\n";
        for (const t of targets) {
            if (t.session && !t.session.finished)
                t.session.ext.sendText(line);
        }
    }

    // ---- parts

    Component {
        id: viewComponent

        TerminalView {
            host: thost
        }
    }

    Component {
        id: sessionComponent

        TabSession {}
    }

    // Only to list the colour schemes before the first view exists.
    QMLTermWidget {
        id: probe

        width: 1
        height: 1
        visible: false
    }

    TextEdit {
        id: clipboard

        visible: false
    }

    FileDialog {
        id: saveDialog

        fileMode: FileDialog.SaveFile
        onAccepted: {
            const fn = thost.saveHandler;
            thost.saveHandler = null;
            if (fn)
                fn(thost.localPath(selectedFile));
        }
    }

    QQC2.Dialog {
        id: broadcastDialog

        title: qsTr("Broadcast to remote sessions")
        modal: true
        parent: QQC2.Overlay.overlay
        anchors.centerIn: parent
        standardButtons: QQC2.Dialog.Ok | QQC2.Dialog.Cancel
        onOpened: broadcastField.forceActiveFocus()
        onAccepted: thost.broadcastAsk(broadcastField.text)

        ColumnLayout {
            QQC2.Label {
                text: qsTr("Text to send to every live remote session (newline is appended):")
            }
            QQC2.TextField {
                id: broadcastField

                Layout.fillWidth: true
                Layout.minimumWidth: 420
                Accessible.name: qsTr("Text to send")
                onAccepted: broadcastDialog.accept()
            }
        }
    }

    MessageBox {
        id: confirmBox

        property string line
        property var targets: []

        title: qsTr("Broadcast")
        buttons: MessageDialog.Yes | MessageDialog.No
        onButtonClicked: (button, role) => {
            if (button === MessageDialog.Yes)
                thost.broadcastSend(targets, line);
        }
    }

    Component.onCompleted: {
        const a = shell.shellActions;
        a.handle("edit.copy", () => withView(v => v.copy()));
        a.handle("edit.paste", () => withView(v => v.paste()));
        a.handle("edit.find", () => thost.find());
        a.handle("view.zoom_in", () => withView(v => v.zoomIn()));
        a.handle("view.zoom_out", () => withView(v => v.zoomOut()));
        a.handle("view.zoom_reset", () => withView(v => v.resetZoom()));
        a.handle("session.log", () => thost.toggleLog(null), () => panes.activeTab !== null && panes.activeTab.logActive);
        a.handle("session.record", () => thost.toggleRecording(null), () => panes.activeTab !== null && panes.activeTab.recordingActive);
        a.handle("session.multiexec", () => thost.setMultiexec(!thost.multiexecOn), () => thost.multiexecOn);
        a.handle("session.broadcast", () => thost.broadcast());
    }
}
