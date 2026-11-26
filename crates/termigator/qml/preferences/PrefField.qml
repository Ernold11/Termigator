import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami

// A line edit bound to one key of the dialog's draft; Enter is OK, like a QDialog's default button.
QQC2.TextField {
    id: field

    required property string key
    // The settings window these controls live in, typed so qmllint checks draft / edit() / ok().
    readonly property PreferencesDialog prefsWindow: Window.window as PreferencesDialog
    property string tip
    readonly property var draft: prefsWindow ? prefsWindow.draft : ({})

    objectName: key
    placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
    // The style sizes a box to its longest model entry / placeholder; a form must not depend on
    // that (hundreds of fonts, a long hint), so the preferred width is fixed and the box grows
    // with the page.
    implicitWidth: Kirigami.Units.gridUnit * 14
    Layout.fillWidth: true
    text: draft[key] ?? ""
    onTextEdited: prefsWindow.edit(key, text)
    onAccepted: prefsWindow.ok()
    Accessible.name: Kirigami.FormData.label
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.visible: tip !== "" && hovered
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
}
