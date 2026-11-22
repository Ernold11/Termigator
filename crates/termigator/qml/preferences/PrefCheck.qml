import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Window
import org.kde.kirigami as Kirigami

// A check box bound to one key of the dialog's draft.
QQC2.CheckBox {
    id: box

    required property string key
    property string tip
    readonly property var draft: Window.window ? Window.window.draft : ({})

    objectName: key
    checked: draft[key] === true
    onToggled: Window.window.edit(key, checked)
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.visible: tip !== "" && hovered
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
}
