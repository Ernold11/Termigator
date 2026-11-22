import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// One settings page: a bold title over a form. Children go
// into the form; label them with Kirigami.FormData.label. It scrolls when the window is lower
// than the page, so nothing ends up under the dialog's buttons.
QQC2.ScrollView {
    id: page

    property string title
    default property alias content: form.data
    // What the page needs to show without scrolling (PreferencesDialog sizes itself to it).
    readonly property real neededWidth: column.implicitWidth + Kirigami.Units.largeSpacing
    readonly property real neededHeight: column.implicitHeight

    clip: true
    contentWidth: availableWidth
    QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

    ColumnLayout {
        id: column

        x: Kirigami.Units.largeSpacing
        width: page.availableWidth - Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        QQC2.Label {
            text: page.title
            font.bold: true
            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.25
            Accessible.role: Accessible.Heading
        }

        Kirigami.FormLayout {
            id: form

            wideMode: true
            Layout.fillWidth: true
        }
    }
}
