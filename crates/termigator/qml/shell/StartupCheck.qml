import QtQuick

// Benchmark-only check: quits once the window has shown its first frame (the host list's visible
// rows exist by then).
Item {
    id: root

    required property var shell

    Connections {
        target: root.shell

        function onFrameSwapped() {
            Qt.exit(0);
        }
    }

    // No frame (a platform that never renders): fail instead of hanging the benchmark.
    Timer {
        interval: 30000
        running: true
        onTriggered: Qt.exit(3)
    }
}
