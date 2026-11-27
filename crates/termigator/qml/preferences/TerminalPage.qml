import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami

PrefPage {
    id: page

    // The settings window these controls live in, typed so qmllint checks draft / edit() / ok().
    readonly property PrefWindow prefsWindow: Window.window as PrefWindow
    readonly property bool windows: Qt.platform.os === "windows"
    // Detected once the page has its window (the dialog), not on every draft change.
    readonly property var shells: windows && prefsWindow ? JSON.parse(prefsWindow.controller.localShells()) : []

    function windowsShellChoices(current) {
        const out = [{
                "text": qsTr("Automatic (WSL, else Command Prompt)"),
                "value": ""
            }];
        for (const s of shells)
            out.push({
                "text": s.available ? s.label : qsTr("%1 (not installed)").arg(s.label),
                "value": s.id
            });
        if (current !== "" && current !== "auto" && !out.some(o => o.value === current))
            out.push({
                "text": current,
                "value": current
            });
        return out;
    }

    title: qsTr("Terminal")

    RowLayout {
        Kirigami.FormData.label: qsTr("Scrollback:")
        Kirigami.FormData.buddyFor: history

        PrefSpin {
            id: history

            key: "embedded_pyte_history"
            from: 1000
            to: 2000000
            stepSize: 10000
            suffix: qsTr(" lines")
            enabled: !unlimited.checked
            Layout.fillWidth: true
            Kirigami.FormData.label: qsTr("Scrollback:")
        }
        PrefCheck {
            id: unlimited

            key: "scrollback_unlimited"
            text: qsTr("Unlimited")
            tip: qsTr("Keep the whole output in memory, with no line limit; a very chatty session can use a lot of RAM.")
        }
    }
    PrefCheck {
        key: "bracketed_paste"
        text: qsTr("Bracketed paste — the shell sees a paste as one block")
        tip: qsTr("Turn off only for old systems that show ^[[200~ around pasted text.")
    }
    PrefCheck {
        key: "tab_attention_marks"
        text: qsTr("Mark background tabs on new output or a bell")
    }
    PrefCheck {
        key: "bell_sound"
        text: qsTr("Play a sound on the terminal bell")
        tip: page.windows ? qsTr("The system's bell sound (Sound settings → Default Beep). At most a few times a second.") : qsTr("The system's bell sound (KDE: Notifications → Plasma Workspace → Beep). At most a few times a second.")
    }
    PrefCheck {
        key: "track_remote_folder"
        text: qsTr("Track the folder of SSH sessions (for the Files panel)")
        tip: qsTr("Starts the remote shell through a small bootstrap that reports its folder (bash, zsh, fish, sh), so the Files panel follows cd. Nothing is typed into the terminal. Not used for entries with their own remote command. The MOTD and the Last login line are printed by the bootstrap.")
    }
    Kirigami.Separator {
        Kirigami.FormData.isSection: true
        Kirigami.FormData.label: qsTr("New sessions")
    }
    // Windows has no external terminal: sessions always open embedded there.
    PrefCombo {
        key: "default_terminal"
        visible: !page.windows
        model: [
            {
                "text": qsTr("Embedded in Termigator"),
                "value": "embedded"
            },
            {
                "text": qsTr("External terminal window"),
                "value": "external"
            }
        ]
        Kirigami.FormData.label: qsTr("Open sessions:")
    }
    PrefField {
        key: "shell"
        visible: !page.windows
        placeholderText: qsTr("your login shell ($SHELL)")
        Kirigami.FormData.label: qsTr("Local shell:")
    }
    // Windows: the detected shells (local_shell.rs); a value typed into config.yaml by hand stays
    // selectable as itself.
    PrefCombo {
        key: "shell"
        visible: page.windows
        model: page.windowsShellChoices(current)
        // A new list (a hand-typed value appeared or went) resets the index: select again.
        onModelChanged: Qt.callLater(sync)
        Kirigami.FormData.label: qsTr("Local shell:")
    }
    PrefField {
        key: "template"
        visible: !page.windows
        placeholderText: "konsole -e bash -lc {cmd_quoted}"
        tip: qsTr("{cmd_quoted} is replaced by the session command")
        Kirigami.FormData.label: qsTr("External terminal:")
    }
}
