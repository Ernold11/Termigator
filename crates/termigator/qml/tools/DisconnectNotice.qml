import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// The non-modal popup for a session that ended on its own. One per tab, and never modal — a dropped
// session must not block the terminals that are still alive. It probes the endpoint in the background and says whether the host still
// answers, which tells "the link died" from "the remote side closed the shell".
//
// `reconnect` (fn or null) is the Reconnect button; `mute` (fn) runs when the box is ticked and the
// notice closes. `conn` is the entry as JSON.
ToolDialog {
    id: dlg

    required property var host
    property string conn: ""
    property string label: ""
    property real lived: 0
    property var reconnect: null
    property var mute: null
    property string token: ""
    property string tail: ""
    readonly property alias muteBox: muteCheck
    readonly property string lead: lived < 10 ? qsTr("<b>%1</b> ended after %2s — it likely never connected.").arg(escaped(label)).arg(lived.toFixed(0)) : qsTr("<b>%1</b> disconnected after %2.").arg(escaped(label)).arg(humanize(lived))
    // Under ~10 s the session almost never "dropped" — it failed to come up (bad key, refused auth),
    // and saying so points the user at the terminal text still on screen.
    readonly property string message: lead + "<br><span style='color:" + hint + ";'>" + (tail !== "" ? tail : qsTr("Checking the host…")) + "</span>"
    readonly property string hint: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.textColor, Kirigami.Theme.backgroundColor, 0.35)

    function escaped(text) {
        return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    }

    function humanize(seconds) {
        if (seconds < 90)
            return seconds.toFixed(0) + "s";
        if (seconds < 5400)
            return (seconds / 60).toFixed(0) + " min";
        return (seconds / 3600).toFixed(1) + " h";
    }

    function doReconnect() {
        const fn = reconnect;
        close();
        if (fn)
            fn();
    }

    title: qsTr("Termigator — session ended")
    okCancel: false
    modal: false
    minWidth: 440
    flags: Qt.Tool

    onClosing: {
        if (muteCheck.checked && mute)
            mute();
    }

    Component.onCompleted: {
        token = String(Math.random());
        host.backend.startNoticeProbe(token, conn);
    }

    Connections {
        target: dlg.host.backend

        function onTestFinished(answered, reportJson) {
            if (answered !== dlg.token)
                return;
            const report = JSON.parse(reportJson);
            dlg.tail = report.ok ? qsTr("%1 still answers — the remote side closed the session.").arg(report.endpoint) : report.summary;
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            source: "network-disconnect"
            implicitWidth: 32
            implicitHeight: 32
            Layout.alignment: Qt.AlignTop
        }
        QQC2.Label {
            Layout.fillWidth: true
            text: dlg.message
            textFormat: Text.RichText
            wrapMode: Text.Wrap
        }
    }

    QQC2.CheckBox {
        id: muteCheck

        text: qsTr("Don't notify me when a session ends")
        QQC2.ToolTip.visible: hovered
        QQC2.ToolTip.text: qsTr("Can be re-enabled in Settings → Configure Termigator… → Application.")
        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
    }

    RowLayout {
        Layout.fillWidth: true

        QQC2.Button {
            id: reconnectButton

            visible: dlg.reconnect !== null
            text: qsTr("Reconnect")
            icon.name: "view-refresh"
            highlighted: true
            onClicked: dlg.doReconnect()
        }
        QQC2.Button {
            text: qsTr("Test connection…")
            icon.name: "network-connect"
            onClicked: dlg.host.testConnection(dlg.conn, dlg.label, dlg)
        }
        Item {
            Layout.fillWidth: true
        }
        QQC2.Button {
            text: qsTr("Close")
            icon.name: "dialog-close"
            onClicked: dlg.close()
        }
    }
}
