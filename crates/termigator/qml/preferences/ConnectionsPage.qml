import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

PrefPage {
    title: qsTr("Connections")

    PrefCheck {
        id: keepalive

        key: "ssh_keepalive"
        text: qsTr("Detect a silently dead SSH connection")
    }
    PrefSpin {
        key: "ssh_keepalive_seconds"
        from: 3
        to: 300
        suffix: qsTr(" s")
        enabled: keepalive.checked
        Layout.maximumWidth: 160
        tip: qsTr("How long a silent connection (cable pulled, laptop suspended, a firewall\nthat eats the flow) can go unanswered before the tab shows “Session stopped”.")
        Kirigami.FormData.label: qsTr("Give up after:")
    }
    PrefSpin {
        key: "probe_timeout"
        from: 1
        to: 60
        suffix: qsTr(" s")
        Layout.maximumWidth: 160
        tip: qsTr("How long Test connection waits for DNS, ping and the TCP connect.")
        Kirigami.FormData.label: qsTr("Test connection waits:")
    }
    Kirigami.Separator {
        Kirigami.FormData.isSection: true
        Kirigami.FormData.label: qsTr("Programs")
    }
    PrefCombo {
        key: "rdp_resolution"
        freeText: true
        model: ["screen", "fullscreen", "1920x1080", "2560x1440", "1600x900", "1366x768", "1280x720"]
        placeholder: qsTr("client default")
        Kirigami.FormData.label: qsTr("RDP window size:")
        QQC2.ToolTip.text: qsTr("The size a remote desktop starts with: the screen, full screen, or WxH such as 1920x1080. A host's own size (its RDP editor) wins.")
        QQC2.ToolTip.visible: hovered
        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
    }
    PrefField {
        key: "rdp_client"
        visible: Qt.platform.os !== "windows" // Windows always opens RDP with mstsc
        placeholderText: qsTr("automatic (xfreerdp3, xfreerdp, wlfreerdp)")
        Kirigami.FormData.label: qsTr("RDP client:")
    }
    PrefField {
        key: "vnc_client"
        placeholderText: Qt.platform.os === "windows" ? qsTr("automatic (TigerVNC's vncviewer.exe on PATH)") : qsTr("automatic (vncviewer, xtigervncviewer, gvncviewer)")
        Kirigami.FormData.label: qsTr("VNC viewer:")
    }
    PrefField {
        key: "editor"
        placeholderText: Qt.platform.os === "windows" ? qsTr("the default application — or e.g. notepad, code --wait") : qsTr("the desktop's default application — or e.g. kate, code --wait")
        tip: qsTr("Files panel → Edit: opens the downloaded copy; every save is uploaded back.")
        Kirigami.FormData.label: qsTr("Editor for remote files:")
    }
}
