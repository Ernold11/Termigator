import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Tools → Credentials…: named logins shared by many
// entries. Edits a working copy of `creds` (VaultBackend.credentialsJson); after accepted()
// `credentials()` and `renames()` give the result. `_orig` is the name a credential had when the
// dialog opened (rename tracking). An encrypted password is never put in the field.
VaultDialog {
    id: dlg

    property var creds: []
    property var items: []
    property bool loading: false
    readonly property alias list: list
    readonly property alias names: names
    readonly property alias nameField: nameField
    readonly property alias userField: userField
    readonly property alias passwordField: passwordField
    readonly property alias forgetBox: forgetBox
    readonly property alias domainField: domainField
    readonly property alias usedLabel: used
    readonly property alias addButton: addButton
    readonly property alias deleteButton: deleteButton

    function credentials() {
        return items.map(c => {
            const out = Object.assign({}, c);
            delete out._orig;
            return out;
        });
    }

    // old name -> new name, for the entries that point at a renamed credential.
    function renames() {
        const out = {};
        for (const c of items) {
            if (c._orig && c._orig !== c.name)
                out[c._orig] = c.name;
        }
        return out;
    }

    function syncEnabled() {
        const has = list.currentIndex >= 0;
        for (const w of [nameField, userField, passwordField, forgetBox, domainField, deleteButton])
            w.enabled = has;
    }

    function load(row) {
        syncEnabled();
        if (row < 0 || row >= items.length) {
            used.text = "";
            return;
        }
        loading = true;
        const c = items[row];
        nameField.text = c.name;
        userField.text = c.user || "";
        domainField.text = c.domain || "";
        const pw = c.password;
        const sealed = pw !== undefined && host.backend.isSealed(String(pw));
        passwordField.text = sealed ? "" : String(pw || "");
        passwordField.placeholderText = sealed ? qsTr("saved (encrypted) — leave empty to keep") : "";
        forgetBox.checked = false;
        forgetBox.visible = sealed;
        const n = host.backend.credentialUsage(c._orig);
        used.text = qsTr("Used by %1 entr%2.").arg(n).arg(n === 1 ? "y" : "ies");
        loading = false;
    }

    function store() {
        const row = list.currentIndex;
        if (loading || row < 0 || row >= items.length)
            return;
        const c = items[row];
        c.name = nameField.text.trim();
        c.user = userField.text.trim();
        const dom = domainField.text.trim();
        if (dom)
            c.domain = dom;
        else
            delete c.domain;
        const typed = passwordField.text;
        const hasSealed = c.password !== undefined && host.backend.isSealed(String(c.password));
        if (typed)
            c.password = typed;
        else if (!hasSealed || forgetBox.checked)
            delete c.password; // else the encrypted one stays
        names.setProperty(row, "name", c.name || qsTr("(unnamed)"));
    }

    function add() {
        const taken = items.map(c => c.name);
        let n = 1;
        while (taken.indexOf("credential" + n) >= 0)
            ++n;
        items.push({
            "name": "credential" + n,
            "user": "",
            "_orig": ""
        });
        names.append({
            "name": "credential" + n
        });
        list.currentIndex = items.length - 1;
        nameField.forceActiveFocus();
        nameField.selectAll();
    }

    function remove() {
        const row = list.currentIndex;
        if (row < 0 || row >= items.length)
            return;
        const drop = () => {
            items.splice(row, 1);
            names.remove(row);
            syncEnabled();
            load(list.currentIndex);
        };
        const n = host.backend.credentialUsage(items[row]._orig);
        if (n)
            host.ask(qsTr("Delete credential"), qsTr("%1 entr%2 this credential. They keep the user name but lose the shared password. Delete it?").arg(n).arg(n === 1 ? "y uses" : "ies use"), dlg, drop);
        else
            drop();
    }

    title: qsTr("Credentials")
    sizeName: "CredentialsDialog"
    modality: Qt.ApplicationModal
    minWidth: 620
    minimumHeight: 340
    validator: () => {
        const taken = items.map(c => c.name);
        if (taken.some(n => !n))
            host.message(qsTr("Credentials"), qsTr("Every credential needs a name."), dlg);
        else if (new Set(taken).size !== taken.length)
            host.message(qsTr("Credentials"), qsTr("Credential names must be unique."), dlg);
        else
            return true;
        return false;
    }
    Component.onCompleted: {
        items = creds.map(c => Object.assign({
            "_orig": c.name
        }, c));
        for (const c of items)
            names.append({
                "name": c.name
            });
        if (items.length > 0)
            list.currentIndex = 0;
        syncEnabled();
        load(list.currentIndex);
    }

    QQC2.Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: qsTr("One login for many entries: pick it in an SSH / RDP entry's “Credential” field. Passwords are encrypted when a master password is set.")
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.fillHeight: true
        spacing: Kirigami.Units.largeSpacing

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.preferredWidth: 1

            QQC2.ScrollView {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ListView {
                    id: list

                    Kirigami.WheelHandler {
                        target: list
                    }

                    clip: true
                    currentIndex: -1
                    keyNavigationEnabled: true
                    activeFocusOnTab: true
                    model: ListModel {
                        id: names
                    }
                    delegate: QQC2.ItemDelegate {
                        required property int index
                        required property string name

                        width: ListView.view.width
                        text: name
                        highlighted: ListView.isCurrentItem
                        onClicked: list.currentIndex = index
                    }
                    onCurrentIndexChanged: dlg.load(currentIndex)
                    Accessible.name: qsTr("Credentials")

                    Kirigami.PlaceholderMessage {
                        anchors.centerIn: parent
                        width: parent.width - Kirigami.Units.gridUnit * 2
                        visible: list.count === 0
                        icon.name: "dialog-password"
                        text: qsTr("No named credentials yet.")
                        helpfulAction: Kirigami.Action {
                            text: qsTr("Add credential")
                            onTriggered: dlg.add()
                        }
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true

                QQC2.Button {
                    id: addButton

                    text: qsTr("Add")
                    onClicked: dlg.add()
                }
                QQC2.Button {
                    id: deleteButton

                    text: qsTr("Delete")
                    onClicked: dlg.remove()
                }
            }
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.preferredWidth: 2

            QQC2.TextField {
                id: nameField

                Kirigami.FormData.label: qsTr("Name:")
                Layout.fillWidth: true
                onTextEdited: dlg.store()
            }
            QQC2.TextField {
                id: userField

                Kirigami.FormData.label: qsTr("User:")
                Layout.fillWidth: true
                onTextEdited: dlg.store()
            }
            QQC2.TextField {
                id: passwordField

                Kirigami.FormData.label: qsTr("Password:")
                Layout.fillWidth: true
                echoMode: TextInput.Password
                onTextEdited: dlg.store()
            }
            QQC2.CheckBox {
                id: forgetBox

                text: qsTr("Remove the saved password")
                onToggled: dlg.store()
            }
            QQC2.TextField {
                id: domainField

                Kirigami.FormData.label: qsTr("Domain:")
                Layout.fillWidth: true
                placeholderText: qsTr("RDP only, optional")
                placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
                onTextEdited: dlg.store()
            }
            QQC2.Label {
                id: used

                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font: Kirigami.Theme.smallFont
                color: Qt.alpha(Kirigami.Theme.textColor, 0.75)
            }
        }
    }
}
