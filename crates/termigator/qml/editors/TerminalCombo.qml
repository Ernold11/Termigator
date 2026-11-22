import QtQuick

// "Embedded in Termigator" / "External window (launch.template)" — the entry's `terminal`.
// Hidden on Windows, which has no external terminal (the entry opens embedded there).
DataCombo {
    visible: Qt.platform.os !== "windows"
    model: [
        {
            "text": qsTr("Embedded in Termigator"),
            "value": "embedded"
        },
        {
            "text": qsTr("External window (launch.template)"),
            "value": "external"
        }
    ]
}
