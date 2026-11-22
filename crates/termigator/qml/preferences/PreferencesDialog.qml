import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import Termigator

// Settings → Configure Termigator…: a KConfigDialog-like window with
// a list of pages and OK / Apply / Cancel / Restore Defaults.
//
// `draft` is the edited copy of the page values (flat JSON keyed like the config keys, see
// preferences_qobj.rs). The editors write it through edit(); Apply puts it into the shared config
// and emits `applied`, after which the owner saves it (`controller.persist()`, which emits
// `configChanged`) and applies what concerns the shell. Terminal tabs re-read the `launch`
// keys on `configChanged`. The dialog registers its own action handler on creation.
Window {
    id: dlg

    // The main window (ShellWindow.qml): `controller`, `shellActions`.
    required property var shell
    property var draft: ({})
    // Apply is enabled by any edit, even one that is changed back.
    property bool dirty: false
    property var fonts: []
    property var schemes: []
    property var styles: []
    property string runningStyle
    readonly property alias controller: prefs
    readonly property alias applyButton: applyButton
    readonly property alias okButton: okButton
    readonly property alias restoreButton: restoreButton
    readonly property alias pageList: pageListView
    readonly property alias pageStack: pageStack

    signal applied

    function reload() {
        draft = JSON.parse(prefs.load());
        dirty = false;
    }

    function edit(key, value) {
        draft = Object.assign({}, draft, {
            [key]: value
        });
        dirty = true;
    }

    // Puts every page back to the defaults; nothing is stored until OK or Apply.
    function restoreDefaults() {
        draft = JSON.parse(prefs.defaults());
        dirty = true;
    }

    function apply() {
        prefs.apply(JSON.stringify(draft));
        dirty = false;
        applied();
        // A new interface style (or Colours with another style): offer the restart, once per choice.
        const choice = draft.quick_style + "/" + draft.ui_theme;
        if (appearancePage.needsRestart && shell.restartAskedFor !== choice) {
            shell.restartAskedFor = choice;
            Qt.callLater(shell.askRestart);
        }
    }

    function ok() {
        if (dirty)
            apply();
        close();
    }

    // Grow to the shown page's height so it fits without scrolling, within the screen; only the
    // shown page has a laid-out form (StackLayout leaves the others unpolished), so this runs on
    // opening and on every page change. The window never shrinks here; the page scrolls if the
    // screen is lower still. The width needs nothing: the forms fit the minimum width (smoke).
    function fitPage() {
        const p = dlg.pageStack.children[dlg.pageStack.currentIndex];
        if (!p || p.neededHeight === undefined)
            return;
        // The window around the page: the margins, the button row and the gap above it.
        const chrome = 2 * Kirigami.Units.largeSpacing + buttonBox.implicitHeight + mainColumn.spacing;
        const want = Math.ceil(p.neededHeight + chrome);
        const screenH = Screen.desktopAvailableHeight > 0 ? Screen.desktopAvailableHeight - 80 : want;
        if (height < want)
            height = Math.min(want, Math.max(screenH, minimumHeight));
        screenTooSmall = height < want;
    }
    // The screen is lower than the shown page: it scrolls (smoke).
    property bool screenTooSmall: false

    function open() {
        show();
        raise();
        requestActivate();
    }

    title: qsTr("Configure Termigator")
    width: 720
    height: 480
    minimumWidth: 720
    minimumHeight: 480
    flags: Qt.Dialog
    modality: Qt.ApplicationModal
    transientParent: shell
    color: Kirigami.Theme.backgroundColor

    onVisibleChanged: {
        if (!visible)
            return;
        if (fonts.length === 0)
            fonts = prefs.monospaceFonts();
        schemes = prefs.colorSchemes();
        styles = prefs.quickStyles();
        runningStyle = prefs.runningStyle();
        reload();
        if (shell.runningTheme === "")
            shell.runningTheme = draft.ui_theme;
        pageListView.currentIndex = 0;
        pageListView.forceActiveFocus();
        fitTimer.restart();
    }

    PreferencesController {
        id: prefs
    }

    DialogSize {
        window: dlg
        name: "PreferencesDialog"
        controller: dlg.shell.controller
    }

    ColumnLayout {
        id: mainColumn

        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Kirigami.Units.largeSpacing

            ListView {
                id: pageListView

                Kirigami.WheelHandler {
                    target: pageListView
                }

                Layout.preferredWidth: 180
                Layout.fillHeight: true
                clip: true
                keyNavigationEnabled: true
                activeFocusOnTab: true
                currentIndex: 0
                Accessible.name: qsTr("Pages")
                model: [
                    {
                        "title": qsTr("Appearance"),
                        "icon": "preferences-desktop-theme"
                    },
                    {
                        "title": qsTr("Terminal"),
                        "icon": "utilities-terminal"
                    },
                    {
                        "title": qsTr("Clipboard"),
                        "icon": "edit-paste"
                    },
                    {
                        "title": qsTr("Logging"),
                        "icon": "document-save"
                    },
                    {
                        "title": qsTr("Connections"),
                        "icon": "network-connect"
                    },
                    {
                        "title": qsTr("Labels"),
                        "icon": "preferences-desktop-color"
                    },
                    {
                        "title": qsTr("Application"),
                        "icon": "preferences-system"
                    }
                ]
                delegate: QQC2.ItemDelegate {
                    required property var modelData
                    required property int index

                    width: ListView.view.width
                    text: modelData.title
                    icon.name: modelData.icon
                    icon.width: 22
                    icon.height: 22
                    highlighted: ListView.isCurrentItem
                    onClicked: ListView.view.currentIndex = index
                }
            }

            Kirigami.Separator {
                Layout.fillHeight: true
            }

            StackLayout {
                id: pageStack

                Layout.fillWidth: true
                Layout.fillHeight: true
                currentIndex: pageListView.currentIndex
                // After the layout settles (the page's form is laid out once shown).
                onCurrentIndexChanged: fitTimer.restart()

                AppearancePage {
                    id: appearancePage

                    fonts: dlg.fonts
                    schemes: dlg.schemes
                    styles: dlg.styles
                    runningStyle: dlg.runningStyle
                    runningTheme: dlg.shell.runningTheme
                    dialog: dlg
                    // The restart hint and button show up after a style change: grow to keep them in view.
                    onNeededHeightChanged: if (dlg.visible)
                        fitTimer.restart()
                }
                TerminalPage {}
                ClipboardPage {}
                LoggingPage {}
                ConnectionsPage {}
                EnvironmentsPage {}
                ApplicationPage {}
            }
        }

        QQC2.DialogButtonBox {
            id: buttonBox

            Layout.fillWidth: true

            QQC2.Button {
                id: restoreButton

                text: qsTr("Restore Defaults")
                icon.name: "document-revert"
                QQC2.DialogButtonBox.buttonRole: QQC2.DialogButtonBox.ResetRole
                QQC2.ToolTip.text: qsTr("Put every page back to Termigator's defaults (nothing is saved until OK or Apply)")
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                onClicked: dlg.restoreDefaults()
            }
            QQC2.Button {
                id: okButton

                text: qsTr("OK")
                icon.name: "dialog-ok"
                QQC2.DialogButtonBox.buttonRole: QQC2.DialogButtonBox.AcceptRole
                onClicked: dlg.ok()
            }
            QQC2.Button {
                id: applyButton

                text: qsTr("Apply")
                icon.name: "dialog-ok-apply"
                enabled: dlg.dirty
                QQC2.DialogButtonBox.buttonRole: QQC2.DialogButtonBox.ApplyRole
                onClicked: dlg.apply()
            }
            QQC2.Button {
                text: qsTr("Cancel")
                icon.name: "dialog-cancel"
                QQC2.DialogButtonBox.buttonRole: QQC2.DialogButtonBox.RejectRole
                onClicked: dlg.close()
            }
        }
    }

    Timer {
        id: fitTimer

        interval: 0
        onTriggered: dlg.fitPage()
    }

    onWidthChanged: if (visible) fitTimer.restart() // narrower = more wrapped lines

    Shortcut {
        sequences: [StandardKey.Cancel]
        onActivated: dlg.close()
    }

    Component.onCompleted: shell.shellActions.handle("settings.configure", () => dlg.open())

    AppPalette {
        target: dlg
    }
}
