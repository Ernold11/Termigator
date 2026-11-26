pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Environment picker: "Inherit from folder" plus the configured categories, each with
// a dot in its colour. Same API as DataCombo (setValue / currentValue).
QQC2.ComboBox {
    id: combo

    property string tip: ""
    property var categories: []

    function setValue(value) {
        const i = indexOfValue(value);
        currentIndex = i >= 0 ? i : 0;
    }

    function reload(value) {
        model = [
            {
                "text": qsTr("Inherit from folder"),
                "value": "",
                "color": ""
            }
        ].concat((categories || []).map(e => ({
                "text": e.name,
                "value": e.id,
                "color": e.color
            })));
        setValue(value || "");
    }

    textRole: "text"
    valueRole: "value"
    Layout.fillWidth: true
    QQC2.ToolTip.visible: hovered && tip !== ""
    QQC2.ToolTip.text: tip
    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay

    delegate: QQC2.ItemDelegate {
        id: entry

        required property int index
        required property var modelData

        width: ListView.view ? ListView.view.width : implicitWidth
        highlighted: combo.highlightedIndex === entry.index
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing

            Rectangle {
                implicitWidth: Kirigami.Units.smallSpacing * 2
                implicitHeight: implicitWidth
                radius: width / 2
                visible: entry.modelData && entry.modelData.color !== ""
                color: entry.modelData && entry.modelData.color !== "" ? entry.modelData.color : "transparent"
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: entry.modelData && entry.modelData.text !== undefined ? entry.modelData.text : entry.modelData
                elide: Text.ElideRight
            }
        }
        onClicked: {
            combo.currentIndex = entry.index;
            combo.popup.close();
        }
    }
}
