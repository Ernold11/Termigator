import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Add / Edit RDP connection: General, Display,
// Redirection, Gateway, Advanced.
EditorDialog {
    id: dlg

    // EditorBackend.prepare("rdp", …)
    required property var ctx
    property bool ready: false

    function load() {
        const f = ctx.form;
        label.text = f.label;
        host.text = f.host;
        userEdit.text = f.user;
        passwordEdit.text = f.password;
        domainEdit.text = f.domain;
        port.value = f.port;
        sshGateway.reload(ctx.gateways, f.sshGateway);
        group.items = ["", ...ctx.groups];
        group.setText(f.group);
        terminal.setValue(f.terminal);
        resolution.setText(f.resolution);
        fullscreen.checked = f.fullscreen;
        multimon.checked = f.multimon;
        dynamicRes.checked = f.dynamicRes;
        clipboard.checked = f.clipboard;
        sound.checked = f.sound;
        microphone.checked = f.microphone;
        homeDrive.checked = f.homeDrive;
        folders.text = f.folders;
        gateway.text = f.gateway;
        gwUser.text = f.gwUser;
        gwDomain.text = f.gwDomain;
        extra.text = f.extra;
        picker.reload(ctx.credentials, f.credential);
        picker.remember();
        ready = true;
        label.forceActiveFocus();
    }

    function collect() {
        return {
            "label": label.text,
            "host": host.text,
            "user": userEdit.text,
            "password": passwordEdit.text,
            "forgetPassword": forget.visible && forget.checked,
            "credential": picker.name,
            "domain": domainEdit.text,
            "port": port.value,
            "sshGateway": sshGateway.currentValue,
            "group": group.text,
            "terminal": terminal.currentValue,
            "resolution": resolution.text,
            "fullscreen": fullscreen.checked,
            "multimon": multimon.checked,
            "dynamicRes": dynamicRes.checked,
            "clipboard": clipboard.checked,
            "sound": sound.checked,
            "microphone": microphone.checked,
            "homeDrive": homeDrive.checked,
            "folders": folders.text,
            "gateway": gateway.text,
            "gwUser": gwUser.text,
            "gwDomain": gwDomain.text,
            "extra": extra.text
        };
    }

    title: ctx.title
    sizeName: "RdpConnectionDialog"
    testable: true
    minWidth: 480
    Component.onCompleted: load()

    CredentialSync {
        id: picker

        combo: credential
        user: userEdit
        password: passwordEdit
        domain: domainEdit
    }

    QQC2.TabBar {
        id: tabs

        Layout.fillWidth: true

        QQC2.TabButton {
            text: qsTr("General")
        }
        QQC2.TabButton {
            text: qsTr("Display")
        }
        QQC2.TabButton {
            text: qsTr("Redirection")
        }
        QQC2.TabButton {
            text: qsTr("Gateway")
        }
        QQC2.TabButton {
            text: qsTr("Advanced")
        }
    }

    StackLayout {
        Layout.fillWidth: true
        Layout.fillHeight: true
        currentIndex: tabs.currentIndex

        Kirigami.FormLayout {
            EdField {
                id: label

                Kirigami.FormData.label: qsTr("Name:")
                placeholderText: dlg.ctx.isNew ? qsTr("e.g. Windows jump box") : ""
            }
            EdField {
                id: host

                Kirigami.FormData.label: qsTr("Host:")
                placeholderText: dlg.ctx.isNew ? "192.168.1.20" : ""
            }
            DataCombo {
                id: credential

                Kirigami.FormData.label: qsTr("Credential:")
                tip: qsTr("Tools → Credentials… — one login shared by many entries.")
                onCurrentIndexChanged: if (dlg.ready)
                    picker.sync()
            }
            EdField {
                id: userEdit

                Kirigami.FormData.label: qsTr("User:")
                placeholderText: dlg.ctx.isNew ? "Administrator" : ""
            }
            EdField {
                id: passwordEdit

                Kirigami.FormData.label: qsTr("Password:")
                echoMode: TextInput.Password
                placeholderText: picker.shared ? qsTr("from credential “%1”").arg(picker.name) : dlg.ctx.form.passwordSealed ? qsTr("saved (encrypted) — leave empty to keep") : ""
                tip: qsTr("Saved in session.yaml — in plain text unless a master password is set\n(Tools → Master password…), then encrypted.")
            }
            // Say it where the password is typed, not only in a tooltip.
            Kirigami.InlineMessage {
                objectName: "plainPassword"
                Layout.fillWidth: true
                visible: passwordEdit.text !== "" && !dlg.ctx.form.vaultOn && !picker.shared
                type: Kirigami.MessageType.Information
                text: qsTr("Saved without encryption. Set a master password (Tools → Master password…) to encrypt saved passwords.")
            }
            EdCheck {
                id: forget

                visible: dlg.ctx.form.passwordSealed && !picker.shared
                text: qsTr("Remove the saved (encrypted) password")
            }
            EdField {
                id: domainEdit

                Kirigami.FormData.label: qsTr("Domain:")
            }
            PortSpin {
                id: port

                Kirigami.FormData.label: qsTr("Port:")
                value: 3389
            }
            GatewayCombo {
                id: sshGateway

                Kirigami.FormData.label: qsTr("SSH gateway:")
            }
            EdCombo {
                id: group

                Kirigami.FormData.label: qsTr("Folder:")
                tip: qsTr("Folder/group for the connection list. Leave empty for top level.")
            }
            TerminalCombo {
                id: terminal

                Kirigami.FormData.label: qsTr("Terminal:")
            }
        }

        Kirigami.FormLayout {
            EdCombo {
                id: resolution

                Kirigami.FormData.label: qsTr("Resolution:")
                items: ["", "1280x720", "1366x768", "1600x900", "1920x1080", "2560x1440"]
                enabled: !fullscreen.checked
                tip: qsTr("WIDTHxHEIGHT, e.g. 1920x1080. Leave empty to use the size set in Settings → Connections (by default the screen the window is on).")
            }
            EdCheck {
                id: fullscreen

                text: qsTr("Full screen")
            }
            EdCheck {
                id: multimon

                text: qsTr("Use all monitors")
            }
            EdCheck {
                id: dynamicRes

                text: qsTr("Resize the desktop with the window")
                tip: qsTr("+dynamic-resolution — the remote desktop follows the window size (not in full screen).")
            }
        }

        Kirigami.FormLayout {
            EdCheck {
                id: clipboard

                text: qsTr("Clipboard")
            }
            EdCheck {
                id: sound

                text: qsTr("Sound")
            }
            EdCheck {
                id: microphone

                text: qsTr("Microphone")
            }
            EdCheck {
                id: homeDrive

                text: qsTr("Home folder as a drive")
            }
            EdArea {
                id: folders

                Kirigami.FormData.label: qsTr("Shared folders:")
                lines: 4
                placeholderText: qsTr("One local folder per line, shared as a drive, e.g.\n~/Downloads")
            }
        }

        Kirigami.FormLayout {
            EdField {
                id: gateway

                Kirigami.FormData.label: qsTr("RD Gateway:")
                placeholderText: qsTr("gateway.example.com[:443] — empty = direct")
            }
            EdField {
                id: gwUser

                Kirigami.FormData.label: qsTr("Gateway user:")
                placeholderText: qsTr("(empty = same login as the desktop)")
            }
            EdField {
                id: gwDomain

                Kirigami.FormData.label: qsTr("Gateway domain:")
            }
        }

        Kirigami.FormLayout {
            EdField {
                id: extra

                Kirigami.FormData.label: qsTr("Extra arguments:")
                // The certificate flag differs by version — FreeRDP 3: /cert:ignore, 2.x: /cert-ignore.
                placeholderText: qsTr("e.g. +clipboard /sound /cert:ignore (FreeRDP 2: /cert-ignore)")
                tip: qsTr("Extra xfreerdp arguments, space-separated.")
            }
        }
    }
}
