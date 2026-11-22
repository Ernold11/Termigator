pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// Quick connect — a saved entry's name or user@host[:port]; the completer offers the
// history and the entry labels (case-insensitive, contains).
Kirigami.ActionTextField {
    id: field

    required property var controller
    // Set by panes/terminal: fn(text) -> error text, "" once a tab opened; it also
    // calls controller.rememberQuickTarget() and persist(). Disabled until then.
    property var handler: null
    property var completions: controller.quickCompletions()
    readonly property var matches: {
        const needle = text.trim().toLowerCase();
        return needle === "" ? [] : completions.filter(c => c.toLowerCase().includes(needle));
    }

    signal failed(string message)

    function choose(index) {
        text = matches[index];
        popup.close();
    }

    enabled: handler !== null
    placeholderText: qsTr("user@host[:port] or a saved name")
    placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
    Accessible.name: qsTr("Quick connect")
    QQC2.ToolTip.visible: hovered
    QQC2.ToolTip.text: qsTr("Quick connect — Enter connects (Ctrl+Shift+O opens the full dialog)")
    QQC2.ToolTip.delay: 700
    leftActions: Kirigami.Action {
        icon.name: "network-connect"
    }
    rightActions: Kirigami.Action {
        icon.name: "edit-clear"
        visible: field.text !== ""
        onTriggered: field.clear()
    }

    onTextEdited: {
        if (matches.length > 0) {
            list.currentIndex = -1;
            popup.open();
        } else {
            popup.close();
        }
    }
    onAccepted: {
        if (popup.visible && list.currentIndex >= 0) {
            choose(list.currentIndex);
            return;
        }
        popup.close();
        const err = handler(text);
        if (err)
            failed(qsTr("Quick connect: %1").arg(err));
        else
            clear();
    }
    Keys.onDownPressed: event => {
        if (popup.visible)
            list.incrementCurrentIndex();
        else
            event.accepted = false;
    }
    Keys.onUpPressed: event => {
        if (popup.visible)
            list.decrementCurrentIndex();
        else
            event.accepted = false;
    }
    Keys.onEscapePressed: event => {
        if (popup.visible)
            popup.close();
        else
            event.accepted = false;
    }

    Connections {
        target: field.controller

        function onConfigChanged() {
            field.completions = field.controller.quickCompletions();
        }
    }

    QQC2.Popup {
        id: popup

        y: field.height
        width: field.width
        height: Math.min(list.contentHeight, Kirigami.Units.gridUnit * 12) + topPadding + bottomPadding
        padding: 1
        closePolicy: QQC2.Popup.CloseOnEscape | QQC2.Popup.CloseOnPressOutsideParent

        ListView {
            id: list

            Kirigami.WheelHandler {
                target: list
            }

            anchors.fill: parent
            clip: true
            model: field.matches
            delegate: QQC2.ItemDelegate {
                required property int index
                required property string modelData

                width: ListView.view.width
                text: modelData
                highlighted: ListView.isCurrentItem
                onClicked: field.choose(index)
            }
        }
    }
}
