// Check: the application's own QML names (Termigator.Terminal: QMLTermWidget + ExternalSession)
// driven by Rust on a real pty. This is the same registration path the app uses; only the
// surrounding window is minimal.
import QtQuick
import Termigator.Terminal

Window {
    id: root

    readonly property var args: Qt.application.arguments
    function argument(name) {
        const i = args.indexOf(name);
        return i >= 0 && i + 1 < args.length ? args[i + 1] : "";
    }

    readonly property string runCommand: argument("--run")
    readonly property string findText: argument("--find")
    readonly property string outPath: argument("--out")
    readonly property string scheme: argument("--scheme")

    width: 1000
    height: 620
    visible: true
    color: "black"
    title: "Ghostty app-path check"

    ExternalSession {
        id: ext

        sessionId: 1
        historySize: 150000
        monitorActivity: true
    }

    QMLTermWidget {
        id: term

        anchors.fill: parent
        session: ext
        font.family: "JetBrainsMono Nerd Font"
        font.pointSize: 11
        colorScheme: root.scheme
        blinkingCursor: true
        bracketedPasteDisabled: false
        confirmMultilinePaste: true
        copyOnSelect: false
        pasteOnRightClick: false
        focus: true

        Component.onCompleted: forceActiveFocus()
        onSearchResult: found => console.log("app check: search", found)
        onLinkActivated: url => console.log("app check: link", url)
        onConfigureRequest: position => console.log("app check: menu", position.x, position.y)
    }

    Connections {
        target: ext

        function onFinishedChanged() {
            if (root.outPath.length === 0)
                Qt.quit();
        }
    }

    Timer {
        interval: 700
        running: root.runCommand.length > 0
        onTriggered: ext.sendText(root.runCommand + "\n")
    }

    Timer {
        interval: 1400
        running: root.findText.length > 0
        onTriggered: term.findNext(root.findText, false, false, true, false)
    }

    Timer {
        interval: 2500
        running: root.outPath.length > 0
        onTriggered: term.grabToImage(result => {
            const ok = result.saveToFile(root.outPath);
            console.log("app check: wrote", root.outPath, ok);
            Qt.exit(ok ? 0 : 1);
        })
    }
}
