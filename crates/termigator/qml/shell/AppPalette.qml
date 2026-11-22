import QtQuick
import org.kde.kirigami as Kirigami

// Styles other than org.kde.desktop (Fusion, FluentWinUI3) take light or dark from the
// desktop, not from the application palette that Preferences → Colours sets and Kirigami.Theme
// follows: a light Termigator on a dark desktop got dark controls with dark Kirigami text. This
// copies the application palette into `target.palette` (a window; its controls inherit it).
//   AppPalette { target: root }
QtObject {
    id: sync

    required property var target
    readonly property SystemPalette normal: SystemPalette {
        colorGroup: SystemPalette.Active
        onPaletteChanged: sync.apply()
    }
    readonly property SystemPalette disabled: SystemPalette {
        colorGroup: SystemPalette.Disabled
    }
    readonly property var roles: ["window", "windowText", "base", "text", "alternateBase", "button", "buttonText", "light", "midlight", "dark", "mid", "shadow", "highlight", "highlightedText", "placeholderText", "accent"]

    function apply() {
        if (Kirigami.Settings.style === "org.kde.desktop")
            return;
        for (const role of roles) {
            target.palette[role] = normal[role]; // every colour group
            target.palette.disabled[role] = disabled[role];
        }
    }

    Component.onCompleted: apply()
}
