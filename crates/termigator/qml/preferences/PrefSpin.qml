import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Window
import org.kde.kirigami as Kirigami

// A spin box bound to one key of the dialog's draft. `special` is shown at `from` (Qt's
// specialValueText), `suffix` after every other value.
QQC2.SpinBox {
    id: spin

    required property string key
    // The settings window these controls live in, typed so qmllint checks draft / edit() / ok().
    readonly property PreferencesDialog prefsWindow: Window.window as PreferencesDialog
    property string suffix
    property string special
    property string tip
    readonly property var draft: prefsWindow ? prefsWindow.draft : ({})

    function shown(v, locale) {
        return special !== "" && v === from ? special : Number(v).toLocaleString(locale, "f", 0) + suffix;
    }

    objectName: key
    editable: true
    textFromValue: (v, locale) => shown(v, locale)
    valueFromText: (text, locale) => {
        if (special !== "" && text === special)
            return from;
        const n = Number.fromLocaleString(locale, text.replace(suffix, "").trim());
        return isNaN(n) ? value : Math.round(n);
    }
    onValueModified: prefsWindow.edit(key, value)
    // After from/to are set: a value assigned earlier would be clamped to the default 0..99.
    Component.onCompleted: value = Qt.binding(() => draft[key] ?? from)
    Accessible.name: Kirigami.FormData.label
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.visible: tip !== "" && hovered
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
}
