pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import Termigator

// The frame of the vault dialogs: content, then the buttons.
// Enter accepts and Esc cancels from any field that does not use the key itself. A window is done
// with once hidden (destroyed after `accepted` / `rejected`), unless `keepAlive`.
//
// `validator`: fn() -> bool run before accepting; false keeps the dialog open (it shows why).
// The size is kept in window.dialog_sizes.<sizeName>, "" = not kept.
Window {
    id: dlg

    // The VaultHost (controller, backend, message boxes).
    required property var host
    property var validator: null
    property int standardButtons: QQC2.DialogButtonBox.Ok | QQC2.DialogButtonBox.Cancel
    property string sizeName: ""
    property bool keepAlive: false
    property int minWidth: 380
    property bool accepting: false
    default property alias content: body.data
    readonly property alias buttons: buttons
    readonly property int margin: Kirigami.Units.largeSpacing * 2

    signal accepted
    signal rejected

    function accept() {
        if (validator && !validator())
            return;
        accepting = true;
        close();
        accepting = false;
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
    color: Kirigami.Theme.backgroundColor

    onVisibleChanged: {
        if (visible)
            return;
        if (!accepting)
            rejected();
        if (!keepAlive)
            Qt.callLater(dlg.destroy);
    }

    Component {
        id: sizeKeeper

        DialogSize {
            controller: dlg.host.shell.controller
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

        ColumnLayout {
            id: body

            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Kirigami.Units.largeSpacing
        }

        QQC2.DialogButtonBox {
            id: buttons

            Layout.fillWidth: true
            visible: dlg.standardButtons !== 0
            standardButtons: dlg.standardButtons
            onAccepted: dlg.accept()
            onRejected: dlg.reject()
        }
    }

    AppPalette {
        target: dlg
    }
}
