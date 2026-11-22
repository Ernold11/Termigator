pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import Termigator

// "Test connection" from an editor: DNS, ping, TCP
// port and server greeting of the entry as it stands in the form, probed off the GUI thread. No
// login is attempted. Start it with open(); the answer comes back through backend.testFinished.
Window {
    id: dlg

    required property var backend
    // The entry as JSON (EditorBackend.build) and the name shown in the title.
    required property string conn
    required property string heading
    property string token: ""
    property bool running: false
    property string headline: ""
    property string note: ""
    property var steps: []

    function start() {
        running = true;
        steps = [];
        note = "";
        headline = qsTr("Probing… (no login is attempted)");
        token = String(Math.random());
        backend.startTest(token, conn);
    }

    function mark(ok) {
        return ok === true ? "✔" : ok === false ? "✘" : "•";
    }

    title: qsTr("Test connection — %1").arg(heading)
    width: Math.max(520, layout.implicitWidth + 2 * Kirigami.Units.largeSpacing * 2)
    height: layout.implicitHeight + 2 * Kirigami.Units.largeSpacing * 2
    minimumWidth: 520
    flags: Qt.Dialog
    modality: Qt.ApplicationModal
    color: Kirigami.Theme.backgroundColor
    onVisibleChanged: {
        if (!visible)
            Qt.callLater(dlg.destroy);
    }
    Component.onCompleted: start()

    Connections {
        target: dlg.backend

        function onTestFinished(answered, reportJson) {
            if (answered !== dlg.token)
                return;
            const report = JSON.parse(reportJson);
            dlg.running = false;
            dlg.headline = (report.ok ? "✔" : "✘") + "  " + report.summary;
            dlg.note = report.via_note;
            dlg.steps = report.steps;
        }
    }

    ColumnLayout {
        id: layout

        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing
        Keys.onEscapePressed: dlg.close()

        QQC2.Label {
            Layout.fillWidth: true
            text: dlg.headline
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
        }

        // Check | Result | Time
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: Kirigami.Units.gridUnit * 8
            spacing: 0

            RowLayout {
                Layout.fillWidth: true

                Kirigami.Heading {
                    level: 5
                    text: qsTr("Check")
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                }
                Kirigami.Heading {
                    level: 5
                    text: qsTr("Result")
                    Layout.fillWidth: true
                }
                Kirigami.Heading {
                    level: 5
                    text: qsTr("Time")
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 4
                }
            }
            Kirigami.Separator {
                Layout.fillWidth: true
            }
            Repeater {
                model: dlg.steps

                RowLayout {
                    id: step

                    required property var modelData

                    Layout.fillWidth: true

                    QQC2.Label {
                        text: dlg.mark(step.modelData.ok) + "  " + step.modelData.name
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                    }
                    QQC2.Label {
                        text: step.modelData.detail
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        QQC2.ToolTip.visible: hoverArea.hovered
                        QQC2.ToolTip.text: step.modelData.detail

                        HoverHandler {
                            id: hoverArea
                        }
                    }
                    QQC2.Label {
                        text: step.modelData.ms === null ? "" : qsTr("%1 ms").arg(Math.round(step.modelData.ms))
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 4
                    }
                }
            }
            Item {
                Layout.fillHeight: true
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: dlg.note !== ""
            text: dlg.note
            wrapMode: Text.Wrap
            color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.textColor, Kirigami.Theme.backgroundColor, 0.35)
        }

        RowLayout {
            Layout.fillWidth: true

            QQC2.Button {
                text: qsTr("Test again")
                icon.name: "view-refresh"
                enabled: !dlg.running
                onClicked: dlg.start()
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

    AppPalette {
        target: dlg
    }
}
