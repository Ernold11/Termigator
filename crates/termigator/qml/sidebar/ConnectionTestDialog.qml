import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Test connection: DNS, ping, TCP port and
// server greeting for one entry, no login. The probe runs on a thread (HostListModel); closing
// the dialog mid-probe just drops the result.
QQC2.Dialog {
    id: dlg

    required property var hosts
    property string entryJson: ""
    property int token: -1
    property var report: null

    function test(json) {
        entryJson = json;
        title = hosts.probeTitle(json);
        open();
        start();
    }

    function start() {
        report = null;
        token = hosts.testConnection(entryJson);
    }

    component StepRow: RowLayout {
        property alias check: checkLabel.text
        property alias detail: detailLabel.text
        property alias time: timeLabel.text
        property font font: Kirigami.Theme.defaultFont

        Layout.fillWidth: true
        spacing: 12

        QQC2.Label {
            id: checkLabel

            font: parent.font
            Layout.preferredWidth: 130
            elide: Text.ElideRight
        }
        QQC2.Label {
            id: detailLabel

            font: parent.font
            Layout.fillWidth: true
            Layout.preferredWidth: 300
            elide: Text.ElideRight
            QQC2.ToolTip.visible: detailHover.hovered && truncated
            QQC2.ToolTip.text: text

            HoverHandler {
                id: detailHover
            }
        }
        QQC2.Label {
            id: timeLabel

            font: parent.font
            Layout.preferredWidth: 60
        }
    }

    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    modal: true
    width: Math.max(520, implicitWidth)
    onClosed: token = -1

    Connections {
        target: dlg.hosts

        function onProbeFinished(token, reportJson) {
            if (token === dlg.token)
                dlg.report = JSON.parse(reportJson);
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 8

        QQC2.TextArea {
            Layout.fillWidth: true
            readOnly: true
            wrapMode: Text.Wrap
            background: null
            padding: 0
            text: dlg.report ? dlg.report.headline : qsTr("Probing… (no login is attempted)")
        }

        // The "Check / Result / Time" table.
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 150
            spacing: 4

            StepRow {
                check: qsTr("Check")
                detail: qsTr("Result")
                time: qsTr("Time")
                font.bold: true
            }

            Repeater {
                model: dlg.report ? dlg.report.steps : []

                delegate: StepRow {
                    required property var modelData

                    check: modelData.check
                    detail: modelData.detail
                    time: modelData.time
                }
            }

            Item {
                Layout.fillHeight: true
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: dlg.report !== null && dlg.report.note !== ""
            text: dlg.report ? dlg.report.note : ""
            wrapMode: Text.Wrap
            color: Kirigami.Theme.disabledTextColor
        }
    }

    footer: QQC2.DialogButtonBox {
        QQC2.Button {
            text: qsTr("Test again")
            icon.name: "view-refresh"
            enabled: dlg.report !== null
            QQC2.DialogButtonBox.buttonRole: QQC2.DialogButtonBox.ActionRole
            onClicked: dlg.start()
        }
        QQC2.Button {
            text: qsTr("Close")
            QQC2.DialogButtonBox.buttonRole: QQC2.DialogButtonBox.RejectRole
        }
    }
}
