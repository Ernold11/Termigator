import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// The MultiExec warning above the terminals, in the main window and in every
// detached one, naming the sessions that receive what is typed or pasted.
Rectangle {
    id: banner

    required property var shell

    implicitHeight: bannerText.implicitHeight
    visible: shell.multiexecTargets >= 0
    color: Kirigami.Theme.neutralBackgroundColor
    border.color: Kirigami.Theme.neutralTextColor
    border.width: 1
    Accessible.role: Accessible.AlertMessage
    Accessible.name: bannerText.text

    QQC2.Label {
        id: bannerText

        width: parent.width
        padding: 4
        leftPadding: 8
        rightPadding: 8
        wrapMode: Text.Wrap
        color: Kirigami.Theme.textColor
        font.weight: Font.DemiBold
        text: qsTr("MultiExec — what you type or paste goes to %1 remote session(s): %2. Ctrl+Shift+M or Session → MultiExec stops it; new sessions stay out until included from the tab menu.").arg(banner.shell.multiexecTargets).arg(banner.shell.multiexecNames || "—")
    }
}
