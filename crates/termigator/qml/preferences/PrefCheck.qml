import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Window
import org.kde.kirigami as Kirigami

// A check box bound to one key of the dialog's draft.
QQC2.CheckBox {
    id: box

    required property string key
    // The settings window these controls live in, typed so qmllint checks draft / edit() / ok().
    readonly property PreferencesDialog prefsWindow: Window.window as PreferencesDialog
    property string tip
    readonly property var draft: prefsWindow ? prefsWindow.draft : ({})

    objectName: key
    checked: draft[key] === true
    onToggled: prefsWindow.edit(key, checked)
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.visible: tip !== "" && hovered
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
}
