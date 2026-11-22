import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// "Edit N entries…": every field has a tick box and only the
// ticked ones change. OK hands changes() to `accepted` — the apply_bulk changes, which can be empty.
ToolDialog {
    id: dlg

    property int count: 0
    property var groups: []
    property var credentials: []
    property var environments: []
    readonly property alias rows: rowBoxes
    readonly property alias tagMode: tagModeCombo
    readonly property alias tagsField: tags
    readonly property alias credentialBox: credential
    readonly property alias folderBox: group
    readonly property alias jumpField: jump
    readonly property alias terminalBox: terminal
    readonly property alias environmentBox: environment
    readonly property var tagModes: [
        {
            "text": qsTr("Add tags"),
            "value": "tags_add"
        },
        {
            "text": qsTr("Remove tags"),
            "value": "tags_remove"
        },
        {
            "text": qsTr("Replace tags with"),
            "value": "tags_set"
        }
    ]

    function on(key) {
        return rowBoxes[key].checked;
    }

    function splitTags(text) {
        return text.split(",").map(t => t.trim()).filter(t => t !== "");
    }

    function changes() {
        const out = {};
        if (on("group"))
            out.group = group.editText.trim();
        if (on("tags"))
            out[tagModeCombo.currentValue] = splitTags(tags.text);
        if (on("terminal"))
            out.terminal = String(terminal.currentValue);
        if (on("environment"))
            out.environment = String(environment.currentValue);
        if (on("proxy_jump"))
            out.proxy_jump = jump.text.trim();
        if (on("credential"))
            out.credential = String(credential.currentValue || "");
        return out;
    }

    title: qsTr("Edit %1 entries").arg(count)
    sizeName: "BulkEditDialog"
    minWidth: 520

    Component.onCompleted: {
        environment.categories = environments;
        environment.reload("");
    }

    // The tick boxes by key.
    QtObject {
        id: rowBoxes

        property var group: groupTick
        property var tags: tagsTick
        property var terminal: terminalTick
        property var environment: environmentTick
        property var proxy_jump: jumpTick
        property var credential: credTick
    }

    QQC2.Label {
        Layout.fillWidth: true
        text: qsTr("Tick what to change on all %1 selected entries — the rest stays as it is.").arg(dlg.count)
        wrapMode: Text.Wrap
    }

    GridLayout {
        Layout.fillWidth: true
        columns: 2
        columnSpacing: Kirigami.Units.largeSpacing

        QQC2.CheckBox {
            id: groupTick

            text: qsTr("Folder:")
        }
        Item {
            Layout.fillWidth: true
            implicitHeight: group.implicitHeight

            EdCombo {
                id: group

                anchors.fill: parent
                enabled: groupTick.checked
                items: [""].concat(dlg.groups)
                Accessible.name: qsTr("Folder")
            }
            QQC2.Label {
                anchors.left: parent.left
                anchors.leftMargin: Kirigami.Units.largeSpacing
                anchors.verticalCenter: parent.verticalCenter
                visible: group.editText === ""
                enabled: false
                text: qsTr("(top level) — or type a new folder, a/b nests")
            }
        }

        QQC2.CheckBox {
            id: tagsTick

            text: qsTr("Tags:")
        }
        RowLayout {
            Layout.fillWidth: true

            DataCombo {
                id: tagModeCombo

                Layout.fillWidth: false
                enabled: tagsTick.checked
                model: dlg.tagModes
                Accessible.name: qsTr("Tag action")
            }
            EdField {
                id: tags

                enabled: tagsTick.checked
                placeholderText: qsTr("comma-separated, e.g. prod, db")
                Accessible.name: qsTr("Tags")
            }
        }

        QQC2.CheckBox {
            id: terminalTick

            visible: Qt.platform.os !== "windows" // no external terminal there
            text: qsTr("Terminal:")
        }
        DataCombo {
            id: terminal

            visible: terminalTick.visible
            enabled: terminalTick.checked
            model: [
                {
                    "text": qsTr("Embedded in Termigator"),
                    "value": "embedded"
                },
                {
                    "text": qsTr("External window"),
                    "value": "external"
                }
            ]
            Accessible.name: qsTr("Terminal")
        }

        QQC2.CheckBox {
            id: environmentTick

            text: qsTr("Label:")
        }
        EnvironmentCombo {
            id: environment

            enabled: environmentTick.checked
            Accessible.name: qsTr("Label")
        }

        QQC2.CheckBox {
            id: jumpTick

            text: qsTr("Jump host:")
        }
        EdField {
            id: jump

            enabled: jumpTick.checked
            placeholderText: qsTr("bastion or user@bastion:port — empty removes it (SSH entries)")
            Accessible.name: qsTr("Jump host")
        }

        QQC2.CheckBox {
            id: credTick

            text: qsTr("Credential:")
            enabled: dlg.credentials.length > 0
            QQC2.ToolTip.visible: !enabled && hovered
            QQC2.ToolTip.text: qsTr("No named credentials yet — Tools → Credentials…")
        }
        DataCombo {
            id: credential

            enabled: credTick.checked
            model: [
                {
                    "text": qsTr("(own login — detach)"),
                    "value": ""
                }
            ].concat(dlg.credentials.map(n => ({
                        "text": n,
                        "value": n
                    })))
            Accessible.name: qsTr("Credential")
        }
    }

    QQC2.Label {
        Layout.fillWidth: true
        text: qsTr("Jump host applies to SSH entries, credential to SSH and RDP; other entries keep theirs.")
        wrapMode: Text.Wrap
        color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.textColor, Kirigami.Theme.backgroundColor, 0.35)
    }
}
