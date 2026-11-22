import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// The find bar under a terminal (QTermWidget's search bar: toggleShowSearchBar). Typing finds as
// you type, Enter / Shift+Enter go to the next / previous match, Esc closes it. The terminal's
// findNext() selects and scrolls to the match.
Rectangle {
    id: bar

    required property var term
    readonly property alias field: findField
    property bool notFound: false

    signal closed

    function open() {
        visible = true;
        findField.forceActiveFocus();
        findField.selectAll();
    }

    function close() {
        visible = false;
        notFound = false;
        closed();
    }

    function toggle() {
        if (visible)
            close();
        else
            open();
    }

    // Typing keeps a match that still fits; the buttons and keys move on.
    function search(forward, current) {
        if (findField.text === "") {
            notFound = false;
            term.findNext("", false, false, true, true); // clears the selection
            return;
        }
        term.findNext(findField.text, matchCase.checked, regex.checked, forward, current);
    }

    visible: false
    color: Kirigami.Theme.backgroundColor
    implicitHeight: row.implicitHeight + 2 * Kirigami.Units.smallSpacing

    Connections {
        target: bar.term

        function onSearchResult(found) {
            bar.notFound = !found && findField.text !== "";
        }
    }

    Rectangle {
        anchors.top: parent.top
        width: parent.width
        height: 1
        color: Kirigami.Theme.disabledTextColor
        opacity: 0.4
    }

    RowLayout {
        id: row

        anchors.fill: parent
        anchors.margins: Kirigami.Units.smallSpacing
        spacing: Kirigami.Units.smallSpacing

        QQC2.ToolButton {
            icon.name: "window-close"
            display: QQC2.AbstractButton.IconOnly
            flat: true
            focusPolicy: Qt.TabFocus
            Accessible.name: qsTr("Close the find bar")
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: qsTr("Close")
            onClicked: bar.close()
        }
        QQC2.Label {
            text: qsTr("Find:")
        }
        QQC2.TextField {
            id: findField

            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            placeholderText: qsTr("Search the terminal output")
            placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
            Accessible.name: qsTr("Find in the terminal")
            color: bar.notFound ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
            onTextChanged: bar.search(true, true)
            onAccepted: bar.search(true, false)
            Keys.onPressed: event => {
                if (event.key === Qt.Key_Escape) {
                    bar.close();
                    event.accepted = true;
                } else if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && (event.modifiers & Qt.ShiftModifier)) {
                    bar.search(false, false);
                    event.accepted = true;
                }
            }
        }
        QQC2.ToolButton {
            icon.name: "go-down-search"
            display: QQC2.AbstractButton.IconOnly
            flat: true
            focusPolicy: Qt.TabFocus
            Accessible.name: qsTr("Find next")
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: qsTr("Find next")
            onClicked: bar.search(true, false)
        }
        QQC2.ToolButton {
            icon.name: "go-up-search"
            display: QQC2.AbstractButton.IconOnly
            flat: true
            focusPolicy: Qt.TabFocus
            Accessible.name: qsTr("Find previous")
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.text: qsTr("Find previous")
            onClicked: bar.search(false, false)
        }
        QQC2.CheckBox {
            id: matchCase

            text: qsTr("Match case")
            focusPolicy: Qt.TabFocus
            onToggled: bar.search(true, true)
        }
        QQC2.CheckBox {
            id: regex

            text: qsTr("Regular expression")
            focusPolicy: Qt.TabFocus
            onToggled: bar.search(true, true)
        }
        QQC2.Label {
            visible: bar.notFound
            text: qsTr("Not found")
            color: Kirigami.Theme.negativeTextColor
        }
        Item {
            Layout.fillWidth: true
        }
    }
}
