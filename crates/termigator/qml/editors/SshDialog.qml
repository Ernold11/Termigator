import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import QtCore
import org.kde.kirigami as Kirigami

// Add / Edit SSH connection: General, Forwarding, Session.
EditorDialog {
    id: dlg

    // EditorBackend.prepare("ssh", …): {title, form, groups, credentials, …}
    required property var ctx
    property bool ready: false
    readonly property alias tabBar: tabs
    // The main window (VaultHost for the jump host key); `var`, as a QWindow has no vaultHost.
    readonly property var shellWindow: transientParent

    function load() {
        const f = ctx.form;
        label.text = f.label;
        host.text = f.host;
        userEdit.text = f.user;
        passwordEdit.text = f.password;
        identity.text = f.identityFile;
        port.value = f.port;
        extra.text = f.extra;
        terminal.setValue(f.terminal);
        group.items = ["", ...ctx.groups];
        group.setText(f.group);
        tags.text = f.tags;
        environment.categories = ctx.environments || [];
        environment.reload(f.environment || "");
        useMosh.checked = f.useMosh;
        autoReconnect.checked = f.autoReconnect;
        proxyJump.text = f.proxyJump;
        jumpKey.text = f.jumpIdentityFile || "";
        forwardAgent.checked = f.forwardAgent;
        x11.setValue(f.x11);
        localFwd.text = f.localFwd;
        remoteFwd.text = f.remoteFwd;
        dynamic.text = f.dynamic;
        remoteCmd.text = f.remoteCmd;
        strictHost.setValue(f.strictHost);
        filesOnly.checked = f.filesOnly;
        compression.checked = f.compression;
        requestTty.checked = f.requestTty;
        keepalive.setValue(f.keepalive);
        locale.text = f.locale;
        fontFamily.setText(f.fontFamily);
        fontSize.value = f.fontSize;
        colorScheme.text = f.colorScheme;
        startup.text = f.startup;
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
            "identityFile": identity.text,
            "port": port.value,
            "extra": extra.text,
            "terminal": terminal.currentValue,
            "group": group.text,
            "tags": tags.text,
            "environment": environment.currentValue,
            "useMosh": useMosh.checked,
            "autoReconnect": autoReconnect.checked,
            "proxyJump": proxyJump.text,
            "jumpIdentityFile": jumpKey.text.trim(),
            "forwardAgent": forwardAgent.checked,
            "x11": x11.currentValue,
            "localFwd": localFwd.text,
            "remoteFwd": remoteFwd.text,
            "dynamic": dynamic.text,
            "remoteCmd": remoteCmd.text,
            "strictHost": strictHost.currentValue,
            "filesOnly": filesOnly.checked,
            "compression": compression.checked,
            "requestTty": requestTty.checked,
            "keepalive": keepalive.currentValue,
            "locale": locale.text,
            "fontFamily": fontFamily.text,
            "fontSize": fontSize.value,
            "colorScheme": colorScheme.text,
            "startup": startup.text
        };
    }

    title: ctx.title
    sizeName: "ConnectionDialog"
    testable: true
    minWidth: 520
    Component.onCompleted: load()

    CredentialSync {
        id: picker

        combo: credential
        user: userEdit
        password: passwordEdit
    }

    FileDialog {
        id: keyPicker

        title: qsTr("SSH private key")
        currentFolder: StandardPaths.writableLocation(StandardPaths.HomeLocation)
        // Windows: file:///C:/k -> C:/k (not /C:/k).
        onAccepted: identity.text = decodeURIComponent(String(selectedFile).replace(/^file:\/\//, "")).replace(/^\/([A-Za-z]:)/, "$1")
    }

    QQC2.TabBar {
        id: tabs

        Layout.fillWidth: true

        QQC2.TabButton {
            text: qsTr("General")
        }
        QQC2.TabButton {
            text: qsTr("Forwarding")
        }
        QQC2.TabButton {
            text: qsTr("Session")
        }
    }

    StackLayout {
        Layout.fillWidth: true
        Layout.fillHeight: true
        currentIndex: tabs.currentIndex

        // A tab scrolls when the window is lower than its form; the form keeps its own
        // height at the top instead of being stretched to the tallest tab.
        QQC2.ScrollView {
            // The form's wide-mode width plus the scroll bar, also as the minimum: the labels
            // stay beside the fields however the window was sized before.
            implicitWidth: generalForm.implicitWidth + QQC2.ScrollBar.vertical.width
            Layout.minimumWidth: implicitWidth
            implicitHeight: generalForm.implicitHeight
            contentWidth: availableWidth
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

            Kirigami.FormLayout {
                id: generalForm

                width: parent.width

                EdField {
                    id: label

                    Kirigami.FormData.label: qsTr("Name:")
                    placeholderText: dlg.ctx.isNew ? qsTr("e.g. Production server") : ""
                }
                EdField {
                    id: host

                    Kirigami.FormData.label: qsTr("Host:")
                    placeholderText: dlg.ctx.isNew ? "192.168.1.10" : ""
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
                    placeholderText: dlg.ctx.isNew ? "admin" : ""
                }
                EdField {
                    id: passwordEdit

                    Kirigami.FormData.label: qsTr("Password:")
                    echoMode: TextInput.Password
                    placeholderText: picker.shared ? qsTr("from credential “%1”").arg(picker.name) : dlg.ctx.form.passwordSealed ? qsTr("saved (encrypted) — leave empty to keep") : qsTr("optional")
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
                // An encrypted password is never shown, so removing it needs its own switch.
                EdCheck {
                    id: forget

                    visible: dlg.ctx.form.passwordSealed && !picker.shared
                    text: qsTr("Remove the saved (encrypted) password")
                }
                RowLayout {
                    Kirigami.FormData.label: qsTr("SSH key:")
                    Kirigami.FormData.buddyFor: identity
                    Layout.fillWidth: true

                    EdField {
                        id: identity

                        placeholderText: qsTr("optional — private key file (ssh -i)")
                    }
                    QQC2.Button {
                        text: qsTr("Browse…")
                        icon.name: "document-open"
                        onClicked: keyPicker.open()
                    }
                }
                // The file looks like a public key or a PuTTY .ppk: warn, but the connection still goes ahead.
                Kirigami.InlineMessage {
                    objectName: "keyProblem"
                    Layout.fillWidth: true
                    visible: text !== ""
                    type: Kirigami.MessageType.Warning
                    text: dlg.backend ? dlg.backend.keyProblem(identity.text) : ""
                }
                PortSpin {
                    id: port

                    Kirigami.FormData.label: qsTr("Port:")
                }
                EdCombo {
                    id: group

                    Kirigami.FormData.label: qsTr("Folder:")
                    tip: qsTr("Folder/group for the connection list. Leave empty for top level.")
                }
                EdField {
                    id: tags

                    Kirigami.FormData.label: qsTr("Tags:")
                    placeholderText: qsTr("comma-separated, e.g. prod, db, warsaw")
                    tip: qsTr("Free-form tags — the sidebar filter searches them too.")
                }
                EnvironmentCombo {
                    id: environment

                    objectName: "environment"
                    Kirigami.FormData.label: qsTr("Label:")
                    tip: qsTr("Coloured in the host list, on the tab and around the terminal, so a session on the wrong server stands out. \"Inherit from folder\" takes the folder's (folder or entry menu → Label). Categories and colours: Settings → Labels.")
                }
                EdField {
                    id: extra

                    Kirigami.FormData.label: qsTr("Extra arguments:")
                }
                TerminalCombo {
                    id: terminal

                    Kirigami.FormData.label: qsTr("Terminal:")
                }
                EdCheck {
                    id: useMosh

                    text: qsTr("Use mosh (mobile shell) instead of ssh")
                    tip: qsTr("Survives roaming and flaky links. Requires mosh on both ends;\nno saved-password support, and -L/-R/-D forwardings do not stay alive.")
                }
                EdCheck {
                    id: autoReconnect

                    text: qsTr("Auto-reconnect when the session ends")
                    tip: qsTr("Embedded terminal only: when the ssh process exits, reopen the session\nautomatically (short backoff, gives up after a few attempts).")
                }
            }
        }

        // A tab scrolls when the window is lower than its form; the form keeps its own
        // height at the top instead of being stretched to the tallest tab.
        QQC2.ScrollView {
            // The form's wide-mode width plus the scroll bar, also as the minimum: the labels
            // stay beside the fields however the window was sized before.
            implicitWidth: forwardingForm.implicitWidth + QQC2.ScrollBar.vertical.width
            Layout.minimumWidth: implicitWidth
            implicitHeight: forwardingForm.implicitHeight
            contentWidth: availableWidth
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

            Kirigami.FormLayout {
                id: forwardingForm

                width: parent.width

                EdField {
                    id: proxyJump

                    Kirigami.FormData.label: qsTr("Jump host:")
                    placeholderText: qsTr("bastion or user@bastion — several hops comma-separated")
                    tip: qsTr("ssh -J (ProxyJump)")
                }
                // The jump hosts' own key (the target keeps the SSH key of the General tab).
                KeyPathField {
                    id: jumpKey

                    objectName: "jumpKey"
                    Kirigami.FormData.label: qsTr("Jump host key:")
                    Kirigami.FormData.labelAlignment: Qt.AlignTop
                    enabled: proxyJump.text.trim() !== ""
                    backend: dlg.backend
                    keys: dlg.shellWindow && dlg.shellWindow.vaultHost ? dlg.shellWindow.vaultHost : null
                    title: qsTr("Jump host key")
                    placeholderText: qsTr("optional — the jump hosts' own key")
                    tip: qsTr("A private key for the jump host(s) only, can differ from the connection's SSH key. Given to ssh through a private config file, not on the command line.")
                    passphraseText: qsTr("This key has a passphrase. An SSH tab asks for it in the terminal; the Files panel, tunnels and RDP/VNC gateways run without one and need the key in the SSH agent.")
                }
                EdCheck {
                    id: forwardAgent

                    text: qsTr("Forward SSH agent")
                    tip: qsTr("Expose your local ssh-agent to the remote session (ssh -A, ForwardAgent=yes).")
                }
                DataCombo {
                    id: x11

                    Kirigami.FormData.label: qsTr("X11 forwarding:")
                    model: [
                        {
                            "text": qsTr("Off"),
                            "value": ""
                        },
                        {
                            "text": qsTr("Untrusted (-X)"),
                            "value": "x"
                        },
                        {
                            "text": qsTr("Trusted (-Y)"),
                            "value": "y"
                        }
                    ]
                    tip: qsTr("Forward X11 for graphical apps over SSH (needs a local X server / XWayland).\nTrusted skips the X SECURITY extension — some apps need it, but the server then\nhas full access to your display.")
                }
                EdArea {
                    id: localFwd

                    Kirigami.FormData.label: qsTr("Local forwards:")
                    lines: 5
                    placeholderText: qsTr("One local forward per line (ssh -L), e.g.\n8080:127.0.0.1:80\n8443:internal:443")
                    tip: qsTr("ssh -L")
                }
                EdArea {
                    id: remoteFwd

                    Kirigami.FormData.label: qsTr("Remote forwards:")
                    lines: 4
                    placeholderText: qsTr("One remote forward per line (ssh -R), e.g.\n9090:127.0.0.1:9090")
                    tip: qsTr("ssh -R")
                }
                EdField {
                    id: dynamic

                    Kirigami.FormData.label: qsTr("SOCKS proxy:")
                    placeholderText: qsTr("SOCKS port (127.0.0.1:port), e.g. 1080 — empty = off")
                    tip: qsTr("ssh -D: dynamic SOCKS on localhost. Use a port number or bind like 127.0.0.1:1080.")
                }
                EdField {
                    id: remoteCmd

                    Kirigami.FormData.label: qsTr("Remote command:")
                    placeholderText: qsTr("Optional remote command (non-interactive), e.g. journalctl -f -u nginx")
                }
                DataCombo {
                    id: strictHost

                    Kirigami.FormData.label: qsTr("Host key checking:")
                    model: [
                        {
                            "text": qsTr("OpenSSH default (ask)"),
                            "value": ""
                        },
                        {
                            "text": "accept-new",
                            "value": "accept-new"
                        },
                        {
                            "text": "yes",
                            "value": "yes"
                        },
                        {
                            "text": "no",
                            "value": "no"
                        },
                        {
                            "text": "ask",
                            "value": "ask"
                        },
                        {
                            "text": "off",
                            "value": "off"
                        }
                    ]
                    tip: qsTr("Maps to ssh -o StrictHostKeyChecking=… (CI / lab automation).")
                }
            }
        }

        // A tab scrolls when the window is lower than its form; the form keeps its own
        // height at the top instead of being stretched to the tallest tab.
        QQC2.ScrollView {
            // The form's wide-mode width plus the scroll bar, also as the minimum: the labels
            // stay beside the fields however the window was sized before.
            implicitWidth: sessionForm.implicitWidth + QQC2.ScrollBar.vertical.width
            Layout.minimumWidth: implicitWidth
            implicitHeight: sessionForm.implicitHeight
            contentWidth: availableWidth
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

            Kirigami.FormLayout {
                id: sessionForm

                width: parent.width

                EdCheck {
                    id: filesOnly

                    text: qsTr("Open as a file browser only (SFTP) — no terminal")
                    tip: qsTr("Double-click opens the host's files in a tab of their own (an SFTP-style file session).")
                }
                EdCheck {
                    id: compression

                    text: qsTr("Compress traffic")
                    tip: qsTr("Helps on slow links, costs CPU on fast ones (ssh Compression=yes).")
                }
                EdCheck {
                    id: requestTty

                    text: qsTr("Always request a terminal")
                    tip: qsTr("ssh -t — for servers / jump setups that start without a TTY. A remote command gets one anyway.")
                }
                DataCombo {
                    id: keepalive

                    Kirigami.FormData.label: qsTr("Keepalive:")
                    model: [
                        {
                            "text": qsTr("(as in Settings)"),
                            "value": ""
                        },
                        {
                            "text": qsTr("On"),
                            "value": "on"
                        },
                        {
                            "text": qsTr("Off"),
                            "value": "off"
                        }
                    ]
                    tip: qsTr("ServerAlive keepalive — off for servers that drop idle-probing clients.")
                }
                EdField {
                    id: locale

                    Kirigami.FormData.label: qsTr("Remote locale:")
                    placeholderText: qsTr("e.g. en_US.UTF-8 — empty = this machine's")
                    tip: qsTr("Sent as LANG / LC_ALL (ssh SendEnv; the server must AcceptEnv LANG LC_*).\nSets the remote language and charset of programs, not a conversion: the\nterminal itself always speaks UTF-8.")
                }
                EdCombo {
                    id: fontFamily

                    Kirigami.FormData.label: qsTr("Terminal font:")
                    // The families load on the first open: the style measures every item, and with
                    // hundreds of fonts that made the SSH editor take about a second to appear.
                    popup.onAboutToShow: {
                        if (items.length === 0) {
                            const typed = text;
                            items = Qt.fontFamilies();
                            setText(typed);
                        }
                    }
                }
                QQC2.SpinBox {
                    id: fontSize

                    Kirigami.FormData.label: qsTr("Font size:")
                    from: 0
                    to: 72
                    editable: true
                    textFromValue: value => value === 0 ? qsTr("(default)") : String(value)
                    valueFromText: text => Number(text) || 0
                    validator: RegularExpressionValidator {
                        regularExpression: /^(\d{1,2}|\(default\))?$/
                    }
                    Layout.preferredWidth: 160
                    Layout.maximumWidth: 160
                }
                EdField {
                    id: colorScheme

                    Kirigami.FormData.label: qsTr("Color scheme:")
                    placeholderText: qsTr("(as in Settings) e.g. Breeze, DarkPastels — red for prod?")
                }
                EdArea {
                    id: startup

                    Kirigami.FormData.label: qsTr("Startup commands:")
                    lines: 4
                    placeholderText: qsTr("Typed after login, one command per line, e.g.\nsudo -i\ncd /srv/app")
                    tip: qsTr("Sent once the session is quiet after login — never into a password /\npassphrase / host-key prompt. Unlike Remote command, the shell stays interactive.")
                }
            }
        }
    }
}
