import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami

// A combo box bound to one key of the dialog's draft. `freeText`: an editable box over a list of
// suggestions (font, colour scheme), model = strings. Otherwise a fixed choice, model = [{text,
// value}] and the draft holds the value.
QQC2.ComboBox {
    id: combo

    required property string key
    // The settings window these controls live in, typed so qmllint checks draft / edit() / ok().
    readonly property PrefWindow prefsWindow: Window.window as PrefWindow
    property bool freeText: false
    property string placeholder
    readonly property var draft: prefsWindow ? prefsWindow.draft : ({})
    readonly property string current: draft[key] ?? ""

    // Not while typing: resetting the text would move the cursor.
    function sync() {
        if (freeText) {
            if (contentItem.activeFocus)
                return;
            const at = find(current, Qt.MatchExactly);
            if (currentIndex !== at)
                currentIndex = at;
            if (editText !== current)
                editText = current;
        } else {
            currentIndex = indexOfValue(current);
        }
    }

    objectName: key
    // The style sizes a box to its longest model entry / placeholder; a form must not depend on
    // that (hundreds of fonts, a long hint), so the preferred width is fixed and the box grows
    // with the page.
    implicitWidth: Kirigami.Units.gridUnit * 14
    Layout.fillWidth: true
    editable: freeText
    textRole: freeText ? "" : "text"
    valueRole: freeText ? "" : "value"
    currentIndex: -1
    onCurrentChanged: sync()
    // Qt picks item 0 when the model arrives, after this signal: look again once it has settled.
    onCountChanged: Qt.callLater(sync)
    onCurrentIndexChanged: if (freeText) Qt.callLater(sync)
    onActivated: index => prefsWindow.edit(key, freeText ? textAt(index) : valueAt(index))
    onAccepted: prefsWindow.ok()
    Accessible.name: Kirigami.FormData.label

    Connections {
        target: combo.freeText ? combo.contentItem : null

        function onTextEdited() {
            combo.prefsWindow.edit(combo.key, combo.editText);
        }
    }

    // The editable box is a plain templates TextField: it draws no placeholder of its own.
    QQC2.Label {
        parent: combo.contentItem
        anchors.fill: parent
        visible: combo.freeText && combo.editText === ""
        text: combo.placeholder
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
        color: Qt.alpha(Kirigami.Theme.textColor, 0.65)
    }

    Component.onCompleted: sync()
}
