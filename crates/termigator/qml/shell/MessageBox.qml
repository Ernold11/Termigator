pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import org.kde.kirigami as Kirigami
import Termigator

// MessageDialog's API (text, title, buttons, open(), close(), visible, buttonClicked), but every open()
// shows a fresh dialog. The system dialog is a real QMessageBox (NativeMessageBox, message_native.cpp):
// QtQuick.Dialogs MessageDialog is native only when the platform theme has a
// message dialog helper, which plasma-integration does not (it falls back to a QML dialog). Besides, a
// reused MessageDialog keeps its platform helper, and the KDE one adds the buttons and connects
// buttonClicked again on each show: 2, 4, 6 … buttons and one click reported N times.
QtObject {
    id: box

    property string text
    // Smaller details under `text` (empty: none).
    property string informativeText
    property string title: "Termigator"
    // QtQuick.Dialogs StandardButton flags (same values as DialogButtonBox and QMessageBox).
    property int buttons: MessageDialog.Ok
    // System dialog only: own texts for standard buttons ({ [MessageDialog.Ok]: "…" }; the roles,
    // order and Esc stay theirs) and the button Enter presses (0: QMessageBox's choice).
    property var buttonTexts: ({})
    property int defaultButton: 0
    // "auto" (a question for Yes / No, else information), "none", "information", "warning",
    // "critical" or "question".
    property string icon: "auto"
    // false: the in-scene QQC2 dialog instead of the system (QMessageBox) one.
    property bool systemDialog: true
    // true: interpret `text` as a subset of HTML (Help → About).
    property bool richText: false
    // Non-empty: a check box under the text; `checked` is its state, current when buttonClicked fires.
    property string checkBoxText
    property bool checked: false
    // The dialog shown now, else null.
    property var current: null
    readonly property bool visible: current !== null && current.visible

    signal buttonClicked(int button, int role)

    function iconName() {
        if (icon !== "auto")
            return icon;
        return (buttons & (MessageDialog.Yes | MessageDialog.No)) ? "question" : "information";
    }

    function open() {
        close();
        // Parented to this box, so the dialog finds the window this box lives in and dies with it.
        current = systemDialog ? systemComponent.createObject(box, {
            "title": box.title,
            "text": box.text,
            "informativeText": box.informativeText,
            "buttons": box.buttons,
            "buttonTexts": box.buttonTexts,
            "defaultButton": box.defaultButton,
            "icon": box.iconName(),
            "richText": box.richText,
            "checkBoxText": box.checkBoxText,
            "checked": box.checked
        }) : sceneComponent.createObject(box);
        current.open();
    }

    function close() {
        const d = current;
        current = null;
        if (d) {
            d.close();
            d.destroy();
        }
    }

    // A dialog closed by its own buttons / Esc; destroy() is deferred, so it is safe inside its signals.
    function release(d) {
        if (current !== d)
            return;
        current = null;
        d.destroy();
    }

    readonly property Component systemComponent: Component {
        NativeMessageBox {
            id: sys

            onButtonClicked: (button, role) => {
                box.checked = sys.checked;
                box.buttonClicked(button, role);
            }
            onVisibleChanged: {
                if (!sys.visible)
                    box.release(sys);
            }
        }
    }

    readonly property Component sceneComponent: Component {
        QQC2.Dialog {
            id: scene

            parent: QQC2.Overlay.overlay
            anchors.centerIn: parent
            modal: true
            // The QQC2 default palette is not the app's (Kirigami) one.
            palette.window: Kirigami.Theme.backgroundColor
            palette.windowText: Kirigami.Theme.textColor
            palette.text: Kirigami.Theme.textColor
            palette.button: Kirigami.Theme.backgroundColor
            palette.buttonText: Kirigami.Theme.textColor
            palette.highlight: Kirigami.Theme.highlightColor
            background: Rectangle {
                color: Kirigami.Theme.backgroundColor
                border.color: Qt.alpha(Kirigami.Theme.textColor, 0.25)
            }
            enter: null
            exit: null
            title: box.title
            width: Math.min(Math.max(implicitWidth, 320), parent ? parent.width - 40 : 320)

            contentItem: Column {
                spacing: Kirigami.Units.largeSpacing

                QQC2.Label {
                    width: parent.width
                    text: box.text
                    wrapMode: Text.Wrap
                    textFormat: box.richText ? Text.RichText : Text.PlainText
                    color: Kirigami.Theme.textColor
                }
                QQC2.Label {
                    width: parent.width
                    visible: box.informativeText !== ""
                    text: box.informativeText
                    wrapMode: Text.Wrap
                    color: Kirigami.Theme.textColor
                }
                QQC2.CheckBox {
                    visible: box.checkBoxText !== ""
                    text: box.checkBoxText
                    checked: box.checked
                    onToggled: box.checked = checked
                }
            }

            standardButtons: box.buttons
            // Yes/Ok accept, No/Cancel/Esc reject (the button box maps the roles).
            onAccepted: box.buttonClicked(box.buttons & MessageDialog.Yes ? MessageDialog.Yes : MessageDialog.Ok, MessageDialog.AcceptRole)
            onRejected: box.buttonClicked(box.buttons & MessageDialog.No ? MessageDialog.No : MessageDialog.Cancel, MessageDialog.RejectRole)
            onClosed: box.release(scene)
        }
    }
}
