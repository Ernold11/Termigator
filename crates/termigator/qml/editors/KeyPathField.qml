import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import QtCore
import org.kde.kirigami as Kirigami

// A private key file of its own (a tunnel's, the jump host's): the path, Browse…, "From SSH
// keys…" (the list of Tools → SSH keys…), a warning (not found, public key, .ppk, too open) and,
// for a key with a passphrase that is not in the agent, `passphraseText` with "Add to agent…"
// (ssh-add in a terminal tab, so Termigator never sees the passphrase).
// `backend` answers keyFileProblem / keyNeedsPassphrase / keyPath (EditorBackend, ToolsBackend);
// `keys` is the VaultHost, or null (no list, no agent button). A null `backend` checks nothing.
ColumnLayout {
    id: root

    property var backend: null
    property var keys: null
    property string title: qsTr("SSH key")
    property string placeholderText: ""
    property string tip: ""
    property string passphraseText: ""
    property alias text: field.text
    readonly property alias field: field
    // Expanded paths of the listed keys that are in the agent.
    property var inAgent: []
    readonly property string keyPath: backend ? backend.keyPath(field.text) : ""
    readonly property bool locked: backend !== null && backend.keyNeedsPassphrase(field.text) && inAgent.indexOf(keyPath) < 0
    property var lastChoice: null

    // The key list of the SSH keys window (ssh-keygen -l, ssh-add -l on a worker thread).
    function listKeys(done) {
        if (!keys)
            return;
        keys.start(t => keys.backend.startListKeys(t), r => {
            inAgent = r.keys.filter(k => k.inAgent === "✔").map(k => k.path);
            if (done)
                done(r.keys);
        });
    }

    function pickKey() {
        listKeys(list => {
            if (list.length === 0) {
                keys.message(title, qsTr("No key pairs found in ~/.ssh — use Browse… or generate one in Tools → SSH keys…."), Window.window);
                return;
            }
            const pick = lastChoice = keys.openChoice(title, qsTr("Private key:"), list.map(k => k.comment !== "" ? "%1 (%2, %3)".arg(k.name).arg(k.type).arg(k.comment) : "%1 (%2)".arg(k.name).arg(k.type)), Window.window);
            pick.accepted.connect(() => field.text = list[pick.index].path);
        });
    }

    function addToAgent() {
        if (keys && keyPath !== "")
            keys.run(JSON.parse(keys.backend.addToAgentCommand(keyPath)));
    }

    Layout.fillWidth: true
    spacing: Kirigami.Units.smallSpacing

    // Back from the terminal tab where the passphrase was typed: is the key in the agent now?
    Connections {
        target: root.Window.window

        function onActiveChanged() {
            if (root.Window.window.active && root.visible && root.field.text.trim() !== "")
                root.listKeys(null);
        }
    }

    FileDialog {
        id: keyFile

        title: root.title
        currentFolder: StandardPaths.writableLocation(StandardPaths.HomeLocation)
        // Windows: file:///C:/k -> C:/k (not /C:/k).
        onAccepted: field.text = decodeURIComponent(String(selectedFile).replace(/^file:\/\//, "")).replace(/^\/([A-Za-z]:)/, "$1")
    }

    RowLayout {
        Layout.fillWidth: true

        EdField {
            id: field

            // Not the placeholder's width: the row must not widen the form past the window
            // (a FormLayout then puts every label above its field).
            Layout.preferredWidth: Kirigami.Units.gridUnit * 8
            placeholderText: root.placeholderText
            tip: root.tip
        }
        QQC2.Button {
            text: qsTr("Browse…")
            icon.name: "document-open"
            onClicked: keyFile.open()
        }
        QQC2.Button {
            visible: root.keys !== null
            text: qsTr("From SSH keys…")
            icon.name: "view-certificate"
            onClicked: root.pickKey()
        }
    }
    Kirigami.InlineMessage {
        objectName: "keyProblem"
        Layout.fillWidth: true
        Layout.preferredWidth: Kirigami.Units.gridUnit * 10 // wraps; its one-line width would widen the form
        visible: text !== ""
        type: Kirigami.MessageType.Warning
        text: root.backend ? root.backend.keyFileProblem(field.text) : ""
    }
    Kirigami.InlineMessage {
        objectName: "keyPassphrase"
        Layout.fillWidth: true
        Layout.preferredWidth: Kirigami.Units.gridUnit * 10
        visible: root.locked && root.passphraseText !== ""
        type: Kirigami.MessageType.Information
        text: root.passphraseText
        actions: [
            Kirigami.Action {
                visible: root.keys !== null
                text: qsTr("Add to agent…")
                icon.name: "list-add"
                onTriggered: root.addToAgent()
            }
        ]
    }
}
