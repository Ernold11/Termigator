pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Quick connect — a saved entry's name or user@host[:port]. Empty, a click, the ▾ button or Down
// shows the history (newest first, `termigator_core::quick_history`); typing filters the history
// and the entry labels (case-insensitive, contains). Up/Down pick, Enter connects to the picked
// row, Escape closes, Shift+Delete or a row's ✕ forgets a history entry, "Clear history" all.
Kirigami.ActionTextField {
    id: field

    required property var controller
    // Set by panes/terminal: fn(text) -> error text, "" once a tab opened; the plan puts the
    // target first in the history and the handler persists it. Disabled until then.
    property var handler: null
    property var completions: controller.quickCompletions()
    property var history: controller.quickHistory()
    // The popup's rows: {text, removable} (removable = in the history).
    readonly property var matches: {
        const needle = text.trim().toLowerCase();
        const list = needle === "" ? history : completions.filter(c => c.toLowerCase().includes(needle));
        return list.map(t => ({
                    "text": t,
                    "removable": history.indexOf(t) >= 0
                }));
    }
    readonly property alias popup: historyPopup
    readonly property alias list: historyList

    signal failed(string message)

    function refresh() {
        completions = controller.quickCompletions();
        history = controller.quickHistory();
    }

    // Shows the rows for the current text; closes when there are none.
    function showList() {
        if (matches.length === 0) {
            historyPopup.close();
            return;
        }
        historyList.currentIndex = -1;
        historyPopup.open();
    }

    function connect() {
        historyPopup.close();
        const err = handler(text);
        if (err)
            failed(qsTr("Quick connect: %1").arg(err));
        else
            clear();
    }

    function choose(index) {
        text = matches[index].text;
        connect();
    }

    function forget(index) {
        const row = matches[index];
        if (!row || !row.removable)
            return;
        controller.forgetQuickTarget(row.text);
        controller.persist();
        refresh(); // also when the save failed
        historyList.currentIndex = Math.min(index, matches.length - 1);
        if (matches.length === 0)
            historyPopup.close();
    }

    function clearHistory() {
        controller.clearQuickHistory();
        controller.persist();
        refresh();
        if (matches.length === 0)
            historyPopup.close();
    }

    objectName: "quickConnect"
    enabled: handler !== null
    placeholderText: qsTr("user@host[:port] or a saved name")
    placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
    Accessible.name: qsTr("Quick connect")
    QQC2.ToolTip.visible: hovered && !historyPopup.visible
    QQC2.ToolTip.text: qsTr("Quick connect — Enter connects, Down shows recent targets (Ctrl+Shift+O opens the full dialog)")
    QQC2.ToolTip.delay: 700
    leftActions: Kirigami.Action {
        icon.name: "network-connect"
    }
    rightActions: [
        Kirigami.Action {
            icon.name: "edit-clear"
            visible: field.text !== ""
            onTriggered: field.clear()
        },
        Kirigami.Action {
            icon.name: "go-down"
            text: qsTr("Recent targets")
            tooltip: qsTr("Recent quick-connect targets")
            enabled: field.matches.length > 0
            onTriggered: {
                field.forceActiveFocus();
                if (historyPopup.visible)
                    historyPopup.close();
                else
                    field.showList();
            }
        }
    ]

    onPressed: {
        if (!historyPopup.visible)
            showList();
    }
    onTextEdited: showList()
    onAccepted: {
        if (historyPopup.visible && historyList.currentIndex >= 0)
            choose(historyList.currentIndex);
        else
            connect();
    }
    Keys.onDownPressed: event => {
        if (historyPopup.visible)
            historyList.incrementCurrentIndex();
        else if (matches.length > 0) {
            showList();
            historyList.currentIndex = 0;
        } else
            event.accepted = false;
    }
    Keys.onUpPressed: event => {
        if (historyPopup.visible)
            historyList.decrementCurrentIndex();
        else
            event.accepted = false;
    }
    Keys.onEscapePressed: event => {
        if (historyPopup.visible)
            historyPopup.close();
        else
            event.accepted = false;
    }
    Keys.onDeletePressed: event => {
        if (historyPopup.visible && historyList.currentIndex >= 0 && (event.modifiers & Qt.ShiftModifier))
            forget(historyList.currentIndex);
        else
            event.accepted = false;
    }

    Connections {
        target: field.controller

        function onConfigChanged() {
            field.refresh();
        }
    }

    QQC2.Popup {
        id: historyPopup

        y: field.height
        width: field.width
        height: Math.min(historyList.contentHeight, Kirigami.Units.gridUnit * 14) + topPadding + bottomPadding
        padding: 1
        closePolicy: QQC2.Popup.CloseOnEscape | QQC2.Popup.CloseOnPressOutsideParent
        // No fade: the keys above go by `visible`, which a running exit transition keeps true.
        enter: Transition {}
        exit: Transition {}

        ListView {
            id: historyList

            Kirigami.WheelHandler {
                target: historyList
            }

            anchors.fill: parent
            clip: true
            keyNavigationWraps: true
            model: field.matches
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Recent targets")
            delegate: QQC2.ItemDelegate {
                id: row

                required property int index
                required property var modelData

                width: ListView.view.width
                text: modelData.text
                highlighted: ListView.isCurrentItem
                focusPolicy: Qt.NoFocus
                Accessible.name: modelData.text
                onClicked: field.choose(index)
                contentItem: RowLayout {
                    spacing: Kirigami.Units.smallSpacing

                    Kirigami.Icon {
                        source: row.modelData.removable ? "document-open-recent" : "network-server"
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    }
                    QQC2.Label {
                        text: row.modelData.text
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                    QQC2.ToolButton {
                        objectName: "forget"
                        visible: row.modelData.removable
                        icon.name: "edit-delete-remove"
                        display: QQC2.AbstractButton.IconOnly
                        text: qsTr("Remove from history")
                        Accessible.name: qsTr("Remove %1 from history").arg(row.modelData.text)
                        focusPolicy: Qt.NoFocus
                        QQC2.ToolTip.text: qsTr("Remove from history (Shift+Delete)")
                        QQC2.ToolTip.visible: hovered
                        QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small + Kirigami.Units.smallSpacing * 2
                        onClicked: field.forget(row.index)
                    }
                }
            }
            footer: QQC2.ItemDelegate {
                objectName: "clearHistory"
                visible: field.text.trim() === "" && field.history.length > 0
                height: visible ? implicitHeight : 0
                width: ListView.view.width
                icon.name: "edit-clear-history"
                text: qsTr("Clear history")
                focusPolicy: Qt.NoFocus
                onClicked: field.clearHistory()
            }
        }
    }
}
