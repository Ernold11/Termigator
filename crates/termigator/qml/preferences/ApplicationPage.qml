import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// The Application preferences page.
PrefPage {
    title: qsTr("Application")

    PrefCheck {
        key: "tray_icon"
        text: qsTr("Show an icon in the system tray (a click shows / hides the window)")
    }
    PrefCheck {
        key: "show_recent"
        text: qsTr("Show Recent connections above the host list")
    }
    // No form label: the spin box and its text sit in the check boxes' column, like they do.
    RowLayout {
        spacing: Kirigami.Units.smallSpacing

        PrefSpin {
            key: "quick_history_limit"
            from: 0
            to: 200
            special: qsTr("Off")
            Layout.maximumWidth: 160
            tip: qsTr("How many targets the toolbar's Quick connect field remembers (newest first).\nPasswords are never stored; 0 keeps none.")
            Accessible.name: historyLabel.text
        }
        QQC2.Label {
            id: historyLabel

            objectName: "quick_history_label"
            text: qsTr("Quick connect history (targets remembered)")
        }
    }
    PrefCheck {
        key: "confirm_close"
        text: qsTr("Confirm closing the window while sessions are running")
    }
    PrefCheck {
        key: "confirm_close_tab"
        text: qsTr("Confirm closing a tab with a running session")
    }
    PrefCheck {
        key: "notify_on_disconnect"
        text: qsTr("Notify when a session ends on its own")
        tip: qsTr("A window with the host's reachability and a Reconnect button. Off by default: the\ndead tab itself shows “Session stopped” (R restart, Enter close, S save output).")
    }
    PrefCheck {
        key: "check_updates"
        text: qsTr("Check for new versions on GitHub at startup (once a day)")
        tip: qsTr("Asks GitHub for the latest release; a newer one shows a banner with a link. Nothing is downloaded.")
    }
    PrefCheck {
        key: "always_on_top"
        text: qsTr("Keep the window always on top")
    }
    PrefCheck {
        key: "session_restore"
        text: qsTr("Reopen the last session's tabs and splits on start")
        tip: qsTr("Passwords are never stored for this — saved / encrypted entries are reused.")
    }
}
