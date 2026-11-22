import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts

// A TCP port: five digits wide, no group separators like QSpinBox.
QQC2.SpinBox {
    from: 1
    to: 65535
    value: 22
    editable: true
    textFromValue: value => String(value)
    valueFromText: text => Number(text)
    Layout.preferredWidth: 120
    Layout.maximumWidth: 120
}
