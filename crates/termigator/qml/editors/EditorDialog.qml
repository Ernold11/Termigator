pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import Termigator

// The frame of every connection editor:
// content, then [Test connection] … [OK] [Cancel]. Enter accepts and Esc cancels from any field
// that does not use the key itself. The size is kept in window.dialog_sizes.<sizeName>.
//
// Users fill `ctx` (EditorBackend.prepare), put their fields in the default content, and answer
// `accepted()` / `testRequested()`; collect() is theirs and returns the form for EditorBackend.build.
Window {
    id: dlg

    required property var controller
    // The EditorBackend (the serial dialog asks it about the device).
    property var backend: null
    // The key in window.dialog_sizes; "" = size not kept.
    property string sizeName: ""
    property bool testable: false
    property int minWidth: 460
    default property alias content: body.data
    readonly property int margin: Kirigami.Units.largeSpacing * 2

    // fn() -> "" or why the form cannot be stored; the window stays open with the text shown.
    property var validate: null
    property string error: ""

    signal accepted
    signal testRequested

    function accept() {
        error = validate ? (validate() || "") : "";
        if (error !== "")
            return;
        close();
        accepted();
    }

    function reject() {
        close();
    }

    width: Math.max(minWidth, layout.implicitWidth + 2 * margin)
    height: layout.implicitHeight + 2 * margin
    minimumWidth: Math.max(minWidth, layout.Layout.minimumWidth + 2 * margin)
    minimumHeight: layout.Layout.minimumHeight + 2 * margin
    flags: Qt.Dialog
    modality: Qt.ApplicationModal
    color: Kirigami.Theme.backgroundColor

    // A window that is hidden is done with: the host reads the form from the signal handler
    // first, the object goes right after.
    onVisibleChanged: {
        if (!visible)
            Qt.callLater(dlg.destroy);
    }

    Component {
        id: sizeKeeper

        DialogSize {
            controller: dlg.controller
            window: dlg
            name: dlg.sizeName
        }
    }

    Component.onCompleted: {
        if (sizeName !== "")
            sizeKeeper.createObject(dlg);
    }

    ColumnLayout {
        id: layout

        anchors.fill: parent
        anchors.margins: dlg.margin
        spacing: Kirigami.Units.largeSpacing

        Keys.onReturnPressed: dlg.accept()
        Keys.onEnterPressed: dlg.accept()
        Keys.onEscapePressed: dlg.reject()

        Kirigami.InlineMessage {
            objectName: "editorError"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            text: dlg.error
            visible: dlg.error !== ""
        }

        ColumnLayout {
            id: body

            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Kirigami.Units.largeSpacing
        }

        RowLayout {
            Layout.fillWidth: true

            QQC2.Button {
                id: testButton

                visible: dlg.testable
                text: qsTr("Test connection")
                icon.name: "network-connect"
                onClicked: dlg.testRequested()
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.text: qsTr("Check whether the host answers on this port. No login is attempted.")
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
            }

            Item {
                Layout.fillWidth: true
            }

            QQC2.DialogButtonBox {
                id: buttons

                standardButtons: QQC2.DialogButtonBox.Ok | QQC2.DialogButtonBox.Cancel
                onAccepted: dlg.accept()
                onRejected: dlg.reject()
            }
        }
    }

    AppPalette {
        target: dlg
    }
}
