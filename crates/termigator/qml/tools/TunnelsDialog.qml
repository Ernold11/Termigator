pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Termigator

// Tools → Tunnels…: the saved tunnels with their live status;
// start / stop / edit. Not modal, and one object for the whole run (ToolsHost keeps it).
ToolDialog {
    id: dlg

    required property var host
    readonly property var backend: host.backend
    property var rows: []
    property string selectedName: ""
    readonly property var selected: rows.find(r => r.name === selectedName) || null
    readonly property bool live: selected !== null && (selected.status === "starting" || selected.status === "running")
    readonly property alias list: tunnelList
    readonly property alias deleteBox: deleteConfirm
    property var lastEdit: null
    property bool loading: false
    readonly property int wStatus: Kirigami.Units.gridUnit * 7
    readonly property int wName: Kirigami.Units.gridUnit * 9
    readonly property int wAuto: Kirigami.Units.gridUnit * 6

    // The model is replaced on every change; the selection follows the tunnel's name.
    function reload() {
        loading = true;
        rows = JSON.parse(backend.tunnelsJson());
        const i = rows.findIndex(r => r.name === selectedName);
        tunnelList.currentIndex = i;
        if (i < 0)
            selectedName = "";
        loading = false;
    }

    function select(name) {
        selectedName = name;
        const i = rows.findIndex(r => r.name === name);
        if (i >= 0)
            tunnelList.currentIndex = i;
    }

    function add() {
        openEditor("", {});
    }

    function edit() {
        if (selected)
            openEditor(selected.name, selected);
    }

    function openEditor(original, tunnel) {
        lastEdit = editComponent.createObject(dlg, {
            "controller": controller,
            "backend": backend,
            "original": original,
            "tunnel": tunnel,
            "transientParent": dlg
        });
        lastEdit.accepted.connect(() => saveEdited(original, lastEdit.form()));
        lastEdit.show();
    }

    // Saving: a running tunnel is restarted with its new definition.
    function saveEdited(original, form) {
        const wasLive = original !== "" && live && selectedName === original;
        if (original !== "")
            backend.forgetTunnel(original);
        backend.saveTunnel(original, JSON.stringify(form));
        host.persist();
        reload();
        select(form.name);
        if (wasLive)
            restart.begin(form.name);
    }

    function askDelete() {
        if (!selected)
            return;
        deleteConfirm.text = qsTr("Delete the tunnel “%1”?").arg(selected.name) + (live ? qsTr(" It is running and will be stopped.") : "");
        deleteConfirm.target = selected.name;
        deleteConfirm.open();
    }

    function startSelected() {
        if (selected)
            host.startTunnel(selected.name);
    }

    function stopSelected() {
        if (selected)
            backend.stopTunnel(selected.name);
    }

    function setAutostart(name, on) {
        backend.setAutostart(name, on);
        host.persist();
        reload();
    }

    function popup(row) {
        if (row >= 0)
            select(rows[row].name);
        const m = menuComponent.createObject(dlg, {
            "hasRow": row >= 0
        });
        m.closed.connect(() => m.destroy());
        m.popup();
    }

    title: qsTr("Tunnels")
    okCancel: false
    modal: false
    keepAlive: true
    sizeName: "TunnelsDialog"
    minWidth: 780
    minHeight: 340
    width: 780
    height: 340

    Component.onCompleted: reload()

    Connections {
        target: dlg.backend

        function onTunnelChanged(name) {
            dlg.reload();
        }
    }

    Timer {
        id: restart

        property string target

        function begin(name) {
            target = name;
            start();
        }

        // The old ssh needs a moment to let go of its ports.
        interval: 400
        onTriggered: dlg.host.startTunnel(target)
    }

    MessageBox {
        id: deleteConfirm

        property string target

        title: qsTr("Delete tunnel")
        buttons: MessageDialog.Yes | MessageDialog.No
        onButtonClicked: (button, role) => {
            if (button !== MessageDialog.Yes)
                return;
            dlg.backend.deleteTunnel(target);
            dlg.host.persist();
            dlg.reload();
        }
    }

    Component {
        id: editComponent

        TunnelEditDialog {}
    }

    Component {
        id: menuComponent

        QQC2.Menu {
            id: rowMenu

            property bool hasRow: false

            QQC2.MenuItem {
                visible: rowMenu.hasRow && !dlg.live
                height: visible ? implicitHeight : 0
                text: qsTr("Start")
                onTriggered: dlg.startSelected()
            }
            QQC2.MenuItem {
                visible: rowMenu.hasRow && dlg.live
                height: visible ? implicitHeight : 0
                text: qsTr("Stop")
                onTriggered: dlg.stopSelected()
            }
            QQC2.MenuSeparator {
                visible: rowMenu.hasRow
                height: visible ? implicitHeight : 0
            }
            QQC2.MenuItem {
                visible: rowMenu.hasRow
                height: visible ? implicitHeight : 0
                text: qsTr("Edit…")
                onTriggered: dlg.edit()
            }
            QQC2.MenuItem {
                visible: rowMenu.hasRow
                height: visible ? implicitHeight : 0
                text: qsTr("Delete")
                onTriggered: dlg.askDelete()
            }
            QQC2.MenuItem {
                visible: !rowMenu.hasRow
                height: visible ? implicitHeight : 0
                text: qsTr("Add…")
                onTriggered: dlg.add()
            }
        }
    }

    component Cell: QQC2.Label {
        elide: Text.ElideRight
        horizontalAlignment: Text.AlignLeft
        verticalAlignment: Text.AlignVCenter
    }

    // The table: Status | Name | Through | Forwards | Autostart.
    ColumnLayout {
        Layout.fillWidth: true
        Layout.fillHeight: true
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.smallSpacing
            Layout.rightMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.largeSpacing

            Kirigami.Heading {
                level: 5
                text: qsTr("Status")
                Layout.preferredWidth: dlg.wStatus
                Layout.minimumWidth: dlg.wStatus
                Layout.maximumWidth: dlg.wStatus
            }
            Kirigami.Heading {
                level: 5
                text: qsTr("Name")
                Layout.preferredWidth: dlg.wName
                Layout.minimumWidth: dlg.wName
                Layout.maximumWidth: dlg.wName
            }
            Kirigami.Heading {
                level: 5
                text: qsTr("Through")
                Layout.preferredWidth: dlg.wName
                Layout.minimumWidth: dlg.wName
                Layout.maximumWidth: dlg.wName
            }
            Kirigami.Heading {
                level: 5
                text: qsTr("Forwards")
                Layout.fillWidth: true
            }
            Kirigami.Heading {
                level: 5
                text: qsTr("Autostart")
                Layout.preferredWidth: dlg.wAuto
                Layout.minimumWidth: dlg.wAuto
                Layout.maximumWidth: dlg.wAuto
            }
        }
        Kirigami.Separator {
            Layout.fillWidth: true
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ListView {
                id: tunnelList

                Kirigami.WheelHandler {
                    target: tunnelList
                }

                anchors.fill: parent
                clip: true
                model: dlg.rows
                currentIndex: -1
                activeFocusOnTab: true
                keyNavigationEnabled: true
                Accessible.role: Accessible.List
                Accessible.name: qsTr("Tunnels")
                onCurrentIndexChanged: {
                    if (!dlg.loading && currentIndex >= 0 && currentIndex < dlg.rows.length)
                        dlg.selectedName = dlg.rows[currentIndex].name;
                }

                MouseArea {
                    anchors.fill: parent
                    z: -1
                    acceptedButtons: Qt.RightButton
                    onClicked: dlg.popup(-1)
                }

                delegate: Rectangle {
                    id: row

                    required property var modelData
                    required property int index
                    readonly property bool current: ListView.isCurrentItem

                    width: ListView.view.width
                    height: Math.max(rowLayout.implicitHeight, checkbox.implicitHeight) + Kirigami.Units.smallSpacing * 2
                    color: current ? Kirigami.Theme.highlightColor : (index % 2 === 1 ? Kirigami.Theme.alternateBackgroundColor : "transparent")
                    Accessible.role: Accessible.ListItem
                    Accessible.name: modelData.name + ", " + modelData.statusText

                    MouseArea {
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        onPressed: mouse => {
                            tunnelList.currentIndex = row.index;
                            tunnelList.forceActiveFocus();
                        }
                        onClicked: mouse => {
                            if (mouse.button === Qt.RightButton)
                                dlg.popup(row.index);
                        }
                        onDoubleClicked: dlg.edit()
                    }

                    RowLayout {
                        id: rowLayout

                        anchors.fill: parent
                        anchors.leftMargin: Kirigami.Units.smallSpacing
                        anchors.rightMargin: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.largeSpacing

                        RowLayout {
                            Layout.preferredWidth: dlg.wStatus
                            Layout.minimumWidth: dlg.wStatus
                            Layout.maximumWidth: dlg.wStatus
                            spacing: Kirigami.Units.smallSpacing

                            Rectangle {
                                implicitWidth: 10
                                implicitHeight: 10
                                radius: 5
                                color: row.modelData.color
                            }
                            Cell {
                                Layout.fillWidth: true
                                text: row.modelData.statusText
                                color: row.current ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                QQC2.ToolTip.visible: statusHover.hovered
                                QQC2.ToolTip.text: row.modelData.detail !== "" ? row.modelData.statusText + ": " + row.modelData.detail : row.modelData.statusText

                                HoverHandler {
                                    id: statusHover
                                }
                            }
                        }
                        Cell {
                            Layout.preferredWidth: dlg.wName
                            Layout.minimumWidth: dlg.wName
                            Layout.maximumWidth: dlg.wName
                            text: row.modelData.name
                            color: row.current ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        }
                        Cell {
                            Layout.preferredWidth: dlg.wName
                            Layout.minimumWidth: dlg.wName
                            Layout.maximumWidth: dlg.wName
                            text: row.modelData.via
                            color: row.current ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        }
                        Cell {
                            Layout.fillWidth: true
                            text: row.modelData.summary
                            color: row.current ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                            QQC2.ToolTip.visible: fwdHover.hovered
                            QQC2.ToolTip.text: text

                            HoverHandler {
                                id: fwdHover
                            }
                        }
                        QQC2.CheckBox {
                            id: checkbox

                            Layout.preferredWidth: dlg.wAuto
                            Layout.minimumWidth: dlg.wAuto
                            Layout.maximumWidth: dlg.wAuto
                            checked: row.modelData.autostart
                            Accessible.name: qsTr("Autostart")
                            QQC2.ToolTip.visible: hovered
                            QQC2.ToolTip.text: qsTr("Start this tunnel together with Termigator")
                            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                            onToggled: dlg.setAutostart(row.modelData.name, checked)
                        }
                    }
                }
            }

            Kirigami.PlaceholderMessage {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.gridUnit * 4
                visible: dlg.rows.length === 0
                icon.name: Kirigami.Settings.style === "org.kde.desktop" ? "network-vpn" : "network-vpn-symbolic" // other styles tint it into a square
                text: qsTr("No tunnels yet. A tunnel keeps SSH port forwards open in the background.")
                helpfulAction: Kirigami.Action {
                    text: qsTr("Add tunnel…")
                    onTriggered: dlg.add()
                }
            }
        }
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        visible: dlg.selected !== null && dlg.selected.status === "failed" && dlg.selected.detail !== ""
        type: Kirigami.MessageType.Error
        text: dlg.selected ? dlg.selected.name + ": " + dlg.selected.detail : ""
    }

    RowLayout {
        Layout.fillWidth: true

        QQC2.Button {
            text: qsTr("Start")
            enabled: dlg.selected !== null && !dlg.live
            onClicked: dlg.startSelected()
        }
        QQC2.Button {
            text: qsTr("Stop")
            enabled: dlg.live
            onClicked: dlg.stopSelected()
        }
        Item {
            implicitWidth: Kirigami.Units.largeSpacing * 2
        }
        QQC2.Button {
            text: qsTr("Add…")
            onClicked: dlg.add()
        }
        QQC2.Button {
            text: qsTr("Edit…")
            enabled: dlg.selected !== null
            onClicked: dlg.edit()
        }
        QQC2.Button {
            text: qsTr("Delete")
            enabled: dlg.selected !== null
            onClicked: dlg.askDelete()
        }
        Item {
            Layout.fillWidth: true
        }
        QQC2.Button {
            text: qsTr("Close")
            icon.name: "dialog-close"
            onClicked: dlg.close()
        }
    }

    QQC2.Label {
        Layout.fillWidth: true
        text: qsTr("Tunnels run in the background without a terminal (ssh -N) and stop when Termigator quits.")
        wrapMode: Text.Wrap
        color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.textColor, Kirigami.Theme.backgroundColor, 0.35)
    }
}
