import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A wrapped secondary note under a field: muted but legible.
QQC2.Label {
    // A wrapping label's implicit width is its unwrapped text, which would widen the form (the
    // FormLayout also counts hidden hints): a fixed preferred width, it wraps and stretches instead.
    Layout.preferredWidth: Kirigami.Units.gridUnit * 16
    Layout.fillWidth: true
    Layout.maximumWidth: Kirigami.Units.gridUnit * 30
    wrapMode: Text.Wrap
    font: Kirigami.Theme.smallFont
    color: Qt.alpha(Kirigami.Theme.textColor, 0.75)
}
