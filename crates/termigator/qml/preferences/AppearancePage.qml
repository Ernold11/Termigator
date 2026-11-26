import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami

PrefPage {
    id: page

    title: qsTr("Appearance")

    // Windows has no Konsole profile: an empty font / size / scheme means Termigator's own defaults.
    // The settings window these controls live in, typed so qmllint checks draft / edit() / ok().
    readonly property PreferencesDialog prefsWindow: Window.window as PreferencesDialog
    readonly property bool windows: Qt.platform.os === "windows"

    // Fixed-pitch families and colour scheme names, listed by the dialog.
    required property var fonts
    required property var schemes
    // Installed Qt Quick Controls styles and the one this process started with.
    required property var styles
    required property string runningStyle
    required property string runningTheme
    // The PreferencesDialog (Restart now applies and closes it).
    required property var dialog
    readonly property string chosenStyle: prefsWindow?.draft?.quick_style ?? ""
    // An installed style other than the running one (a configured one that is not offered any
    // more falls back to org.kde.desktop at start: nothing to restart for).
    readonly property bool styleChanged: page.styles.indexOf(page.chosenStyle) >= 0 && page.chosenStyle.toLowerCase() !== page.runningStyle.toLowerCase()
    // Other styles show Qt's own icons, which keep the old icon theme until a restart.
    // Apply / OK then asks whether to restart now (ShellWindow.askRestart).
    readonly property bool needsRestart: styleChanged || themeNeedsRestart
    readonly property bool themeNeedsRestart: page.runningStyle !== "org.kde.desktop" && page.runningTheme !== "" && (page.dialog.draft?.ui_theme ?? page.runningTheme) !== page.runningTheme

    Kirigami.Separator {
        Kirigami.FormData.isSection: true
        Kirigami.FormData.label: qsTr("Terminal")
    }
    PrefCombo {
        key: "font_family"
        freeText: true
        placeholder: page.windows ? qsTr("Automatic (Cascadia Code, Consolas, …)") : qsTr("Konsole's default profile")
        model: page.fonts
        Kirigami.FormData.label: qsTr("Font:")
    }
    PrefSpin {
        key: "font_size"
        from: 0
        to: 72
        suffix: qsTr(" pt")
        special: page.windows ? qsTr("Default (11 pt)") : qsTr("Konsole's default")
        Layout.maximumWidth: 160
        Kirigami.FormData.label: qsTr("Font size:")
    }
    PrefCombo {
        key: "color_scheme"
        freeText: true
        placeholder: page.windows ? qsTr("Breeze (default)") : qsTr("Konsole's default profile")
        model: [""].concat(page.schemes)
        Kirigami.FormData.label: qsTr("Color scheme:")
    }
    PrefCheck {
        key: "blinking_cursor"
        text: qsTr("Blinking cursor")
    }
    PrefHint {
        text: qsTr("An entry's own font or colour scheme (its Session tab) still wins.")
    }
    Kirigami.Separator {
        Kirigami.FormData.isSection: true
        Kirigami.FormData.label: qsTr("Window")
    }
    PrefCombo {
        key: "quick_style"
        // The configured style stays listed even if it is not installed (it then falls back).
        model: (page.styles.indexOf(page.chosenStyle) < 0 && page.chosenStyle !== "" ? [page.chosenStyle] : []).concat(page.styles).map(s => ({
            text: s === "org.kde.desktop" ? qsTr("org.kde.desktop (Breeze, default)") : s,
            value: s
        }))
        Kirigami.FormData.label: qsTr("Interface style:")
    }
    PrefCombo {
        key: "ui_theme"
        model: [
            {
                "text": qsTr("Automatic (follow the desktop)"),
                "value": "auto"
            },
            {
                "text": qsTr("Light"),
                "value": "light"
            },
            {
                "text": qsTr("Dark"),
                "value": "dark"
            }
        ]
        Kirigami.FormData.label: qsTr("Colours:")
    }
    PrefHint {
        objectName: "style_restart_hint"
        visible: page.needsRestart
        text: qsTr("This takes effect after Termigator is restarted; Apply or OK offers to restart now. The open tabs are reopened (local shells start afresh, remote sessions connect again).")
    }
    PrefCheck {
        key: "host_list_detailed"
        text: qsTr("Two-line host list (user@host under each name)")
    }
    PrefHint {
        text: qsTr("Light and Dark apply at once (with another interface style than org.kde.desktop the icons follow after a restart). Switching back to Automatic takes effect after Termigator is restarted; it leaves a KDE Plasma session as it is, on other desktops Termigator uses its own Breeze look.")
        visible: !page.windows
    }
    PrefHint {
        visible: page.windows
        text: qsTr("Light and Dark apply at once (with another interface style than org.kde.desktop the icons follow after a restart); Automatic follows Windows' app mode.")
    }
}
