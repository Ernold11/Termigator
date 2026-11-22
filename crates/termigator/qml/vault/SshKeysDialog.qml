import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Tools → SSH keys…: the key pairs in ~/.ssh, the agent, generate
// / copy / add to the agent / install on a host. One window, not modal; the commands run in a
// terminal tab (host.run) so passphrases and passwords are typed there, and the list refreshes when
// the window is activated again.
VaultDialog {
    id: dlg

    property var keys: []
    property string messageText: ""
    property int messageType: Kirigami.MessageType.Information
    readonly property alias table: table
    readonly property alias rows: rows
    readonly property alias message: banner
    readonly property alias generateButton: generate
    readonly property alias copyButton: copy
    readonly property alias agentButton: agent
    readonly property alias installButton: install
    readonly property var columns: [qsTr("Key"), qsTr("Type"), qsTr("Fingerprint"), qsTr("Comment"), qsTr("In agent")]
    readonly property var current: table.currentIndex >= 0 && table.currentIndex < keys.length ? keys[table.currentIndex] : null

    function refresh() {
        host.start(t => host.backend.startListKeys(t), r => {
            const previous = current ? current.path : "";
            keys = r.keys;
            rows.clear();
            for (const k of r.keys)
                rows.append({
                    "name": k.name,
                    "type": k.type,
                    "fingerprint": k.fingerprint,
                    "comment": k.comment,
                    "inAgent": k.inAgent,
                    "path": k.path
                });
            const at = r.keys.findIndex(k => k.path === previous);
            table.currentIndex = at;
            messageText = r.agent ? "" : qsTr("No SSH agent is reachable — keys can still be used, but each connection asks for the passphrase.");
            messageType = Kirigami.MessageType.Information;
        });
    }

    function generateKey() {
        const dlg2 = host.openGenerate(dlg);
        dlg2.accepted.connect(() => host.run(dlg2.command));
    }

    function copyPublicKey() {
        const k = current;
        if (!k)
            return;
        const r = JSON.parse(host.backend.readPublicKey(k.pub));
        if (r.error) {
            host.message(qsTr("SSH keys"), r.error, dlg);
            return;
        }
        host.copyText(r.text);
        messageText = qsTr("Public key %1 copied — paste it into authorized_keys or a Git host's SSH key page.").arg(k.pub.split("/").pop());
        messageType = Kirigami.MessageType.Positive;
    }

    function addToAgent() {
        const k = current;
        if (k)
            host.run(JSON.parse(host.backend.addToAgentCommand(k.path)));
    }

    function installOnHost() {
        const k = current;
        const entries = JSON.parse(host.backend.sshEntriesJson());
        if (!k)
            return;
        if (entries.length === 0) {
            host.message(qsTr("SSH keys"), qsTr("There are no SSH entries to install the key on."), dlg);
            return;
        }
        const pick = host.openChoice(qsTr("Install key on host"), qsTr("Append %1 to authorized_keys on:").arg(k.pub.split("/").pop()), entries.map(e => e.label), dlg);
        pick.accepted.connect(() => {
            const entry = entries[pick.index];
            host.passwordEnv(JSON.stringify(entry.entry), env => {
                const r = JSON.parse(host.backend.copyIdCommand(k.pub, JSON.stringify(entry.entry), env !== null));
                if (r.error)
                    host.message(qsTr("SSH keys"), r.error, dlg);
                else
                    host.run(r, env);
            }, dlg);
        });
    }

    title: qsTr("SSH keys")
    sizeName: "SshKeysDialog"
    keepAlive: true
    standardButtons: 0
    width: 820
    height: 340
    minWidth: 600
    onActiveChanged: {
        if (active)
            refresh();
    }
    Component.onCompleted: refresh()

    Kirigami.InlineMessage {
        id: banner

        Layout.fillWidth: true
        visible: dlg.messageText !== ""
        text: dlg.messageText
        type: dlg.messageType
    }

    // Header and rows share the column widths below.
    QQC2.ScrollView {
        Layout.fillWidth: true
        Layout.fillHeight: true

        ListView {
            id: table

            Kirigami.WheelHandler {
                target: table
            }

            clip: true
            currentIndex: -1
            keyNavigationEnabled: true
            activeFocusOnTab: true
            model: ListModel {
                id: rows
            }
            Accessible.name: qsTr("SSH keys")
            headerPositioning: ListView.OverlayHeader
            header: Rectangle {
                z: 2
                width: table.width
                height: headerRow.implicitHeight + 4
                color: Kirigami.Theme.backgroundColor

                RowLayout {
                    id: headerRow

                    anchors.fill: parent
                    anchors.leftMargin: Kirigami.Units.smallSpacing
                    spacing: Kirigami.Units.largeSpacing

                    QQC2.Label {
                        Layout.preferredWidth: 130
                        text: dlg.columns[0]
                        font.bold: true
                    }
                    QQC2.Label {
                        Layout.preferredWidth: 90
                        text: dlg.columns[1]
                        font.bold: true
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: dlg.columns[2]
                        font.bold: true
                    }
                    QQC2.Label {
                        Layout.preferredWidth: 150
                        text: dlg.columns[3]
                        font.bold: true
                    }
                    QQC2.Label {
                        Layout.preferredWidth: 60
                        text: dlg.columns[4]
                        font.bold: true
                        horizontalAlignment: Text.AlignHCenter
                    }
                }
            }
            delegate: QQC2.ItemDelegate {
                required property int index
                required property string name
                required property string type
                required property string fingerprint
                required property string comment
                required property string inAgent
                required property string path

                width: ListView.view.width
                highlighted: ListView.isCurrentItem
                onClicked: table.currentIndex = index
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: path
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                Accessible.name: qsTr("%1, %2, %3").arg(name).arg(type).arg(fingerprint)

                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing

                    QQC2.Label {
                        Layout.preferredWidth: 130
                        text: name
                        elide: Text.ElideRight
                    }
                    QQC2.Label {
                        Layout.preferredWidth: 90
                        text: type
                        elide: Text.ElideRight
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: fingerprint
                        elide: Text.ElideRight
                    }
                    QQC2.Label {
                        Layout.preferredWidth: 150
                        text: comment
                        elide: Text.ElideRight
                    }
                    QQC2.Label {
                        Layout.preferredWidth: 60
                        text: inAgent
                        horizontalAlignment: Text.AlignHCenter
                    }
                }
            }

            Kirigami.PlaceholderMessage {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.gridUnit * 2
                visible: table.count === 0
                icon.name: "dialog-password"
                text: qsTr("No SSH keys in %1 yet.").arg(dlg.host.backend.sshDir)
                helpfulAction: Kirigami.Action {
                    text: qsTr("Generate a key…")
                    onTriggered: dlg.generateKey()
                }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true

        QQC2.Button {
            id: generate

            text: qsTr("Generate…")
            onClicked: dlg.generateKey()
        }
        QQC2.Button {
            id: copy

            text: qsTr("Copy public key")
            enabled: dlg.current !== null
            onClicked: dlg.copyPublicKey()
        }
        QQC2.Button {
            id: agent

            text: qsTr("Add to agent")
            enabled: dlg.current !== null
            onClicked: dlg.addToAgent()
        }
        QQC2.Button {
            id: install

            text: qsTr("Install on host…")
            enabled: dlg.current !== null
            onClicked: dlg.installOnHost()
        }
        Item {
            Layout.fillWidth: true
        }
        QQC2.Button {
            text: qsTr("Close")
            icon.name: "window-close"
            onClicked: dlg.reject()
        }
    }
}
