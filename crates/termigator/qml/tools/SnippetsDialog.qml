import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// Edit → Snippets → Manage snippets… edits a working copy
// of the list; OK hands it to `accepted` (read it with snippets()).
ToolDialog {
    id: dlg

    // The config's snippets: [{name, command, enter}].
    property var initial: []
    property var items: []
    property bool loading: false
    readonly property alias list: list
    readonly property alias nameField: nameField
    readonly property alias commandField: commandField
    readonly property alias enterBox: enterBox
    readonly property bool has: list.currentIndex >= 0 && list.currentIndex < items.length

    function snippets() {
        return items;
    }

    function load(row) {
        if (row < 0 || row >= items.length)
            return;
        loading = true;
        const s = items[row];
        nameField.text = s.name;
        commandField.text = s.command;
        enterBox.checked = s.enter !== false;
        loading = false;
    }

    // Stores the snippet: the name falls back to the command's first line.
    function store() {
        const row = list.currentIndex;
        if (loading || row < 0 || row >= items.length)
            return;
        const command = commandField.text;
        const first = (command.split(/[\r\n]/)[0] || "").slice(0, 80);
        const name = nameField.text.trim() || first || "snippet";
        items[row] = {
            "name": name,
            "command": command,
            "enter": enterBox.checked
        };
        names.setProperty(row, "name", name);
    }

    function add() {
        items.push({
            "name": "New snippet",
            "command": "",
            "enter": true
        });
        names.append({
            "name": "New snippet"
        });
        itemsChanged();
        list.currentIndex = items.length - 1;
        load(items.length - 1);
        nameField.forceActiveFocus();
        nameField.selectAll();
    }

    function remove() {
        const row = list.currentIndex;
        if (row < 0 || row >= items.length)
            return;
        items.splice(row, 1);
        names.remove(row);
        itemsChanged();
        if (items.length > 0)
            load(list.currentIndex);
    }

    title: qsTr("Snippets")
    sizeName: "SnippetsDialog"
    minWidth: 620
    minHeight: 380

    Component.onCompleted: {
        items = initial.map(s => Object.assign({}, s));
        for (const s of items)
            names.append({
                "name": s.name
            });
        if (items.length > 0) {
            list.currentIndex = 0;
            load(0);
        }
    }

    ListModel {
        id: names
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.fillHeight: true
        spacing: Kirigami.Units.largeSpacing

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.preferredWidth: 1
            spacing: Kirigami.Units.smallSpacing

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                QQC2.ScrollView {
                    anchors.fill: parent

                    ListView {
                        id: list

                        Kirigami.WheelHandler {
                            target: list
                        }

                        clip: true
                        model: names
                        currentIndex: -1
                        activeFocusOnTab: true
                        keyNavigationEnabled: true
                        Accessible.name: qsTr("Snippets")
                        onCurrentIndexChanged: dlg.load(currentIndex)

                        delegate: QQC2.ItemDelegate {
                            required property string name
                            required property int index

                            width: ListView.view.width
                            text: name
                            highlighted: ListView.isCurrentItem
                            onClicked: ListView.view.currentIndex = index
                        }
                    }
                }

                Kirigami.PlaceholderMessage {
                    anchors.centerIn: parent
                    width: parent.width - Kirigami.Units.gridUnit * 2
                    visible: names.count === 0
                    icon.name: "insert-text"
                    text: qsTr("No snippets yet — commands you type often, one click away.")
                    helpfulAction: Kirigami.Action {
                        text: qsTr("Add snippet")
                        onTriggered: dlg.add()
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true

                QQC2.Button {
                    text: qsTr("Add")
                    onClicked: dlg.add()
                }
                QQC2.Button {
                    text: qsTr("Delete")
                    enabled: dlg.has
                    onClicked: dlg.remove()
                }
            }
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.preferredWidth: 2

            EdField {
                id: nameField

                Kirigami.FormData.label: qsTr("Name:")
                enabled: dlg.has
                onTextEdited: dlg.store()
            }
            EdArea {
                id: commandField

                Kirigami.FormData.label: qsTr("Command:")
                enabled: dlg.has
                lines: 6
                placeholderText: qsTr("e.g. journalctl -u {?Service} -f")
                onTextChanged: dlg.store()
            }
            EdCheck {
                id: enterBox

                text: qsTr("Press Enter after typing it")
                enabled: dlg.has
                checked: true
                onToggled: dlg.store()
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: qsTr("Placeholders: {host} {user} {label} {port} — filled per session (also with MultiExec); {?Question} — asked before sending; {{ }} — literal braces.")
                wrapMode: Text.Wrap
                color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.textColor, Kirigami.Theme.backgroundColor, 0.35)
            }
        }
    }
}
