import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Help → Keyboard shortcuts, built from the action table (core actions::shortcut_rows),
// grouped by menu, with a search over action and keys.
Window {
    id: dialog

    // [[menu, action, keys], …]
    required property var rows
    readonly property alias shownCount: shown.count

    function filter(needle) {
        shown.clear();
        const n = needle.trim().toLowerCase();
        for (const [menu, action, keys] of rows) {
            if (n === "" || `${action} ${keys}`.toLowerCase().includes(n))
                shown.append({ menu, actionText: action, keys });
        }
    }

    title: qsTr("Keyboard Shortcuts")
    width: 560
    height: 520
    flags: Qt.Dialog
    modality: Qt.ApplicationModal
    color: Kirigami.Theme.backgroundColor

    ListModel {
        id: shown
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing

        Kirigami.SearchField {
            id: search

            Layout.fillWidth: true
            focus: true
            placeholderText: qsTr("Search shortcuts…")
            placeholderTextColor: Qt.alpha(Kirigami.Theme.textColor, 0.65)
            onTextChanged: dialog.filter(text)
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing

            Kirigami.Heading {
                level: 5
                text: qsTr("Action")
                Layout.fillWidth: true
            }
            Kirigami.Heading {
                level: 5
                text: qsTr("Shortcut")
            }
        }

        QQC2.ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ListView {
                id: list

                Kirigami.WheelHandler {
                    target: list
                }

                clip: true
                model: shown
                section.property: "menu"
                section.delegate: Kirigami.ListSectionHeader {
                    required property string section

                    width: ListView.view.width
                    text: section
                }
                delegate: QQC2.ItemDelegate {
                    id: entry

                    required property string actionText
                    required property string keys

                    width: ListView.view.width
                    contentItem: RowLayout {
                        QQC2.Label {
                            text: entry.actionText
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                        QQC2.Label {
                            text: entry.keys
                        }
                    }
                }
            }
        }

        QQC2.DialogButtonBox {
            Layout.fillWidth: true
            standardButtons: QQC2.DialogButtonBox.Close
            onRejected: dialog.close()
        }
    }

    Shortcut {
        sequences: [StandardKey.Cancel]
        onActivated: dialog.close()
    }

    Component.onCompleted: filter("")

    AppPalette {
        target: dialog
    }
}
