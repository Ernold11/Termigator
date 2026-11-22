import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Tools → Master password… set, change or remove the
// vault. Every action rewrites the config at once through controller.persist() — no OK/Cancel
// staging, the passwords on disk must never be half re-encrypted.
VaultDialog {
    id: dlg

    property bool busy: false
    readonly property alias status: statusLabel
    readonly property alias done: doneMessage
    readonly property alias setButton: setPasswordButton
    readonly property alias changeButton: changePasswordButton
    readonly property alias removeButton: removePasswordButton

    function refresh() {
        const s = JSON.parse(host.backend.statusJson());
        statusLabel.text = s.text;
        setPasswordButton.enabled = s.canSet && !busy;
        changePasswordButton.enabled = s.canChange && !busy;
        removePasswordButton.enabled = s.canRemove && !busy;
    }

    // The result of an action: saved, shown; or the reason it changed nothing.
    function finished(r) {
        busy = false;
        if (!r.ok) {
            host.message(qsTr("Termigator"), r.warning, dlg);
            return;
        }
        host.shell.controller.persist();
        refresh();
        doneMessage.text = r.message;
        doneMessage.visible = true;
    }

    function setMaster() {
        host.askPassword(qsTr("Set master password"), qsTr("New master password:"), true, dlg, master => {
            busy = true;
            refresh();
            host.start(t => host.backend.startSet(t, master), finished);
        });
    }

    function changeMaster() {
        host.backend.forgetKey(); // always re-confirm the current master before re-keying
        host.ensureUnlocked(qsTr("Enter the current master password."), () => {
            host.askPassword(qsTr("Change master password"), qsTr("New master password:"), true, dlg, master => {
                busy = true;
                refresh();
                host.start(t => host.backend.startChange(t, master), finished);
            });
        }, refresh, dlg);
    }

    function removeMaster() {
        host.ask(qsTr("Remove master password"), qsTr("Saved passwords will be decrypted and stored in plain text in session.yaml again.\nContinue?"), dlg, () => {
            host.backend.forgetKey();
            host.ensureUnlocked(qsTr("Enter the current master password."), () => {
                busy = true;
                refresh();
                host.start(t => host.backend.startRemove(t), finished);
            }, refresh, dlg);
        });
    }

    title: qsTr("Master password")
    modality: Qt.ApplicationModal
    minWidth: 480
    standardButtons: QQC2.DialogButtonBox.Close
    Component.onCompleted: {
        refresh();
        setPasswordButton.forceActiveFocus();
    }

    QQC2.Label {
        id: statusLabel

        Layout.fillWidth: true
        wrapMode: Text.Wrap
    }

    Kirigami.InlineMessage {
        id: doneMessage

        Layout.fillWidth: true
        visible: false
        type: Kirigami.MessageType.Positive
    }

    QQC2.Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        font: Kirigami.Theme.smallFont
        color: Qt.alpha(Kirigami.Theme.textColor, 0.75)
        text: qsTr("Saved SSH/RDP passwords are encrypted with a key derived from the master password (scrypt + AES-GCM). The master password itself is not stored — if you forget it, the saved passwords are lost (the entries stay).")
    }

    RowLayout {
        Layout.fillWidth: true

        QQC2.Button {
            id: setPasswordButton

            text: qsTr("Set master password…")
            onClicked: dlg.setMaster()
        }
        QQC2.Button {
            id: changePasswordButton

            text: qsTr("Change…")
            onClicked: dlg.changeMaster()
        }
        QQC2.Button {
            id: removePasswordButton

            text: qsTr("Remove…")
            onClicked: dlg.removeMaster()
        }
        Item {
            Layout.fillWidth: true
        }
    }
}
