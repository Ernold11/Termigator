import QtQuick
import QtQuick.Window

// The colour of an entry's environment as a bar or, with `frame`, as a border around a
// terminal (only while Settings → Environments → "Frame the whole terminal" is on). Nothing for "" or a category without a colour. `colorHint` lets a caller pass the
// already-resolved colour (the host rows do); otherwise the shell's config is asked, so a
// colour changed in Settings applies without reopening a tab.
Rectangle {
    id: mark

    required property string environment
    // A resolved colour from the caller; empty = ask the controller.
    property string colorHint: ""
    property bool frame: false
    property color envColor: "transparent"
    // window.environment_frame; a frame mark draws nothing while it is off.
    property bool frameEnabled: false
    // The frame's line width; the terminal keeps its text inside it.
    readonly property int frameWidth: 2
    // Drawn (apart from the visibility of the items around it): a colour, and for a frame, the setting.
    readonly property bool shown: envColor.a > 0 && (!frame || frameEnabled)

    function appController() {
        const w = Window.window;
        if (!w)
            return null;
        if (w.controller)
            return w.controller;
        if (w.area && w.area.shell)
            return w.area.shell.controller;
        return null;
    }

    function refresh() {
        const fc = appController();
        frameEnabled = fc ? fc.windowBool("environment_frame", false) : false;
        if (colorHint !== "") {
            envColor = colorHint;
            return;
        }
        const c = appController();
        envColor = c && environment !== "" ? c.environmentColor(environment) : "transparent";
    }

    onColorHintChanged: refresh()
    onEnvironmentChanged: refresh()
    Component.onCompleted: refresh()

    Connections {
        target: mark.appController()

        function onConfigChanged() {
            mark.refresh();
        }
    }

    visible: shown
    color: frame ? "transparent" : envColor
    border.color: frame ? envColor : "transparent"
    border.width: frame ? frameWidth : 0
}
