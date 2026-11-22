import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami

// Settings → Environments: the names and colours of the entry/folder categories. The
// stable id is never shown; entries store it, so renaming a category keeps every mark.
Item {
    id: page

    property var draft: Window.window ? Window.window.draft : ({})
    readonly property var environments: draft.environments || []
    property int colorIndex: -1
    readonly property alias rowList: rows
    readonly property alias rowModel: rowModel
    // Set while this page itself writes the draft, so the row model is not rebuilt (that would
    // drop the focus of the name field being typed in).
    property bool selfEdit: false

    function copyList(list) {
        return (list || []).map(e => ({
                    "id": e.id,
                    "name": e.name,
                    "color": e.color
                }));
    }

    function reloadRows() {
        rowModel.clear();
        for (const e of copyList(page.environments))
            rowModel.append(e);
    }

    function snapshot() {
        const out = [];
        for (let i = 0; i < rowModel.count; ++i) {
            const e = rowModel.get(i);
            out.push({
                "id": e.id,
                "name": e.name,
                "color": e.color
            });
        }
        return out;
    }

    function commit() {
        if (!Window.window)
            return;
        selfEdit = true;
        Window.window.edit("environments", snapshot());
        selfEdit = false;
    }

    onEnvironmentsChanged: {
        if (!selfEdit)
            reloadRows();
    }
    Component.onCompleted: reloadRows()

    function nextId() {
        let n = 1;
        const taken = id => {
            for (let i = 0; i < rowModel.count; ++i)
                if (rowModel.get(i).id === id)
                    return true;
            return false;
        };
        while (taken("custom-" + n))
            ++n;
        return "custom-" + n;
    }

    function addEnvironment() {
        rowModel.append({
            "id": nextId(),
            "name": qsTr("New label"),
            "color": "#3daee9"
        });
        commit();
    }

    function removeEnvironment(index) {
        rowModel.remove(index);
        commit();
    }

    function pickColor(index, current) {
        page.colorIndex = index;
        colorDialog.selectedColor = current || "#3daee9";
        colorDialog.open();
    }

    ListModel {
        id: rowModel
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        QQC2.Label {
            text: qsTr("Labels")
            font.bold: true
            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.25
            Accessible.role: Accessible.Heading
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 30
            text: qsTr("Categories an entry or folder can be marked with. The colour shows in the host list, the Label menu and the tab (and around the terminal when enabled below). Removing a category also clears it from the entries and folders that used it.")
            wrapMode: Text.Wrap
            font: Kirigami.Theme.smallFont
            color: Qt.alpha(Kirigami.Theme.textColor, 0.75)
        }

        PrefCheck {
            key: "environment_frame"
            text: qsTr("Frame the whole terminal in the label colour")
            tip: qsTr("Off by default: the colour then shows only on the tab. With this on, a connection with an label also gets a border around its terminal panel.")
        }

        ListView {
            id: rows

            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: rowModel
            spacing: Kirigami.Units.smallSpacing

            delegate: RowLayout {
                required property var model
                required property int index

                width: rows.width
                spacing: Kirigami.Units.smallSpacing

                QQC2.TextField {
                    Layout.fillWidth: true
                    text: model.name
                    placeholderText: qsTr("Category name")
                    placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
                    Accessible.name: qsTr("Label name")
                    onTextEdited: {
                        rowModel.setProperty(index, "name", text);
                        page.commit();
                    }
                }

                QQC2.Button {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 7
                    Accessible.name: qsTr("Colour of %1").arg(model.name || model.id)
                    onClicked: page.pickColor(index, model.color)
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.smallSpacing

                        Rectangle {
                            Layout.preferredWidth: Kirigami.Units.gridUnit
                            Layout.preferredHeight: Layout.preferredWidth
                            radius: width / 2
                            color: model.color || "transparent"
                            border.width: 1
                            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.4)
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: model.color || qsTr("No colour")
                            elide: Text.ElideRight
                        }
                    }
                }

                QQC2.ToolButton {
                    icon.name: "edit-delete"
                    text: qsTr("Remove")
                    Accessible.name: qsTr("Remove %1").arg(model.name || model.id)
                    QQC2.ToolTip.visible: hovered
                    QQC2.ToolTip.text: qsTr("Remove this category")
                    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                    onClicked: page.removeEnvironment(index)
                }
            }
        }

        QQC2.Button {
            text: qsTr("Add label")
            icon.name: "list-add"
            onClicked: page.addEnvironment()
        }
    }

    ColorDialog {
        id: colorDialog

        title: qsTr("Label colour")
        onAccepted: {
            if (page.colorIndex >= 0 && page.colorIndex < rowModel.count) {
                rowModel.setProperty(page.colorIndex, "color", String(selectedColor));
                page.commit();
            }
        }
    }
}
