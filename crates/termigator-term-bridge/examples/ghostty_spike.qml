import QtQuick
import Termigator.Terminal.Spike

// Spike window: a real shell on a GhosttySession, drawn by GhosttySpike.
//
//   no arguments          interactive shell (Rust attaches the pty to sessionId 1)
//   --source <file>       draw a capture instead of a pty
//   --run <command>       send a command to the shell once it is up
//   --type <text>         type the text through the key encoder, then Return
//   --out <png>           grab the window after the command has run and quit
Window {
    id: root

    readonly property var args: Qt.application.arguments
    function argument(name) {
        const i = args.indexOf(name);
        return i >= 0 && i + 1 < args.length ? args[i + 1] : "";
    }

    readonly property string sourcePath: argument("--source")
    readonly property string runCommand: argument("--run")
    readonly property string typeText: argument("--type")
    readonly property string selectArea: argument("--select")
    readonly property string pasteTextArg: argument("--paste")
    readonly property string outPath: argument("--out")
    readonly property bool shellMode: sourcePath.length === 0

    width: 1000
    height: 620
    visible: true
    color: "black"
    title: "Ghostty spike — " + (shellMode ? "shell" : sourcePath)

    GhosttySession {
        id: session

        sessionId: 1
        historySize: -1
    }

    GhosttySpike {
        id: term

        anchors.fill: parent
        focus: true
        session: session
        source: root.sourcePath

        Component.onCompleted: term.forceActiveFocus()
    }

    Connections {
        target: session

        function onFinishedChanged() {
            if (root.outPath.length === 0)
                Qt.quit();
        }
    }

    Timer {
        interval: 700
        running: root.runCommand.length > 0
        onTriggered: term.sendText(root.runCommand + "\n")
    }

    // Type through the key encoder (not sendText): one key event per character, then Return.
    Timer {
        interval: 900
        running: root.typeText.length > 0
        onTriggered: {
            for (let i = 0; i < root.typeText.length; ++i) {
                const ch = root.typeText[i];
                const upper = ch.toUpperCase();
                const shifted = ch !== upper;
                const key = (upper.length === 1 ? upper.charCodeAt(0) : 0);
                term.sendKey(key, shifted ? 0x02000000 : 0, ch);
            }
            term.sendKey(0x01000004, 0, "\r");
        }
    }

    Timer {
        interval: 1200
        running: root.pasteTextArg.length > 0
        onTriggered: term.pasteText(root.pasteTextArg, false)
    }

    // `--select c1,r1,c2,r2`: select a region for screenshots (mouse is not scriptable).
    Timer {
        interval: 1600
        running: root.selectArea.length > 0
        onTriggered: {
            const p = root.selectArea.split(",").map(Number);
            if (p.length === 4)
                term.select(p[0], p[1], p[2], p[3], false);
        }
    }

    Timer {
        interval: root.shellMode ? 2500 : 600
        running: root.outPath.length > 0
        onTriggered: term.grabToImage(result => {
            const ok = result.saveToFile(root.outPath);
            console.log("ghostty spike: wrote", root.outPath, ok);
            Qt.exit(ok ? 0 : 1);
        })
    }
}
