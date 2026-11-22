import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import Termigator
import Termigator.Terminal

// A terminal tab's view: the terminal over the tab's
// session, the find bar, the "Session stopped" overlay, the context menu and the key handling that
// belongs to the pane (zoom, the overlay's R / Enter / S, the clipboard keys in a window that has
// no window shortcuts of its own). Views are disposable (termigator-term-bridge): PanesArea makes a
// new one wherever the tab is shown and destroys the old; the session is `tab.session`.
FocusScope {
    id: view

    required property var tab
    property var host: null
    readonly property var session: tab.session
    property var term: null
    readonly property alias findBar: findBar
    readonly property alias menu: menu
    readonly property alias pasteDialog: pasteDialog
    // Opens an allowed link; the smoke check replaces it (no xdg-open).
    property var openExternally: url => Qt.openUrlExternally(url)
    // The user looks at it: the tab is current in its pane / window and the window is active.
    readonly property bool seen: visible && Window.window !== null && Window.window.active
    readonly property var area: host ? host.panes : null

    function copy() {
        if (term)
            term.copyClipboard();
    }

    function paste() {
        if (term)
            term.pasteClipboard();
    }

    function zoomIn() {
        if (term)
            term.zoomIn();
    }

    function zoomOut() {
        if (term)
            term.zoomOut();
    }

    function resetZoom() {
        if (term)
            term.resetZoom();
    }

    // The draggable scroll indicator: moves the scrollback to `value` (bounded). The wheel and
    // the keys still reach the terminal itself; this is only for the thumb's MouseArea.
    function scrollToValue(value) {
        if (!term)
            return;
        const min = term.scrollbarMinimum;
        const max = term.scrollbarMaximum;
        term.setScrollbarValue(Math.max(min, Math.min(max, Math.round(value))));
    }

    function toggleFind() {
        findBar.toggle();
    }

    function focusTerminal() {
        if (term)
            term.forceActiveFocus();
    }

    // A session dying in another pane must not steal the keys being typed (the next "r" or
    // Enter would restart or close it): take the focus only if it was here or nowhere.
    function takeFocusIfHere() {
        const w = Window.window;
        if (term && (w === null || w.activeFocusItem === null || view.activeFocus))
            term.forceActiveFocus();
    }

    // A pasted script would run line by line — ask first (Konsole's own question).
    function confirmPaste(text, appendReturn) {
        pasteDialog.text = text;
        pasteDialog.appendReturn = appendReturn;
        pasteDialog.open();
    }

    // Only ordinary schemes: terminal output is untrusted, and a custom handler must not be
    // reachable by printing text and getting a Ctrl+click.
    function openLink(url) {
        const m = /^([A-Za-z][A-Za-z0-9+.-]*):/.exec(url);
        if (m && ["http", "https", "ftp", "mailto", "file"].indexOf(m[1].toLowerCase()) >= 0)
            openExternally(url);
    }

    function handleKey(event) {
        const m = event.modifiers;
        const ctrl = (m & Qt.ControlModifier) !== 0;
        const alt = (m & Qt.AltModifier) !== 0;
        const meta = (m & Qt.MetaModifier) !== 0;
        const shift = (m & Qt.ShiftModifier) !== 0;
        // The "Session stopped" keys on a dead session: R restarts, Enter exits the tab,
        // S saves the output. Only this terminal takes them — the find bar's field is another item.
        if (session && session.overlayActive && session.finished && !ctrl && !alt && !meta) {
            let key = "";
            if (event.key === Qt.Key_R)
                key = "r";
            else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter)
                key = "enter";
            else if (event.key === Qt.Key_S)
                key = "s";
            if (key !== "") {
                session.core.overlayKey(key, false);
                event.accepted = true;
                return;
            }
        }
        if (ctrl && !alt && !meta) {
            // Zoom before the terminal sees the key (ADR-11); Ctrl+- but not Ctrl+_: that is undo
            // in readline and go-to-line in nano.
            if (event.key === Qt.Key_Plus || event.key === Qt.Key_Equal) {
                zoomIn();
                event.accepted = true;
            } else if (event.key === Qt.Key_Minus) {
                zoomOut();
                event.accepted = true;
            } else if (event.key === Qt.Key_0) {
                resetZoom();
                event.accepted = true;
            } else if (shift && event.key === Qt.Key_C) {
                copy();
                event.accepted = true;
            } else if (shift && event.key === Qt.Key_V) {
                paste();
                event.accepted = true;
            } else if (shift && event.key === Qt.Key_F) {
                toggleFind();
                event.accepted = true;
            }
        } else if (shift && !ctrl && !alt && !meta && event.key === Qt.Key_Insert) {
            paste();
            event.accepted = true;
        }
    }

    // ---- the context menu

    function menuEntries(x, y) {
        const s = session;
        const out = [];
        const url = term.hotspotUrlAt(x, y);
        if (url !== "") {
            const mail = url.toLowerCase().startsWith("mailto:");
            out.push({
                "text": mail ? qsTr("Send Email To…") : qsTr("Open Link"),
                "fn": () => openLink(url)
            }, {
                "text": mail ? qsTr("Copy Email Address") : qsTr("Copy Link Address"),
                "fn": () => host.copyText(mail ? url.slice(7) : url)
            }, "-");
        }
        const hasSelection = term.selectedText(false).trim() !== "";
        out.push({
            "text": qsTr("Copy"),
            "enabled": hasSelection,
            "tip": qsTr("Copy selection to the system clipboard"),
            "fn": () => copy()
        }, {
            "text": qsTr("Paste"),
            "tip": qsTr("Paste from the system clipboard"),
            "fn": () => paste()
        }, {
            "text": qsTr("Paste Selection"),
            "tip": qsTr("Paste the last mouse selection (the primary selection on Linux)"),
            "fn": () => term.pasteSelection()
        }, "-", {
            "text": qsTr("Enable folder tracking in this shell"),
            "tip": qsTr("Types a one-line prompt hook (bash / zsh) into this session so it reports its\ncurrent directory — the Files panel then follows every cd."),
            "fn": () => s.ext.sendText(" " + s.core.osc7Snippet() + "\n") // a leading space keeps it out of the history
        }, "-", {
            "text": qsTr("Find…"),
            "fn": () => toggleFind()
        }, "-", {
            "text": qsTr("Zoom in"),
            "fn": () => zoomIn()
        }, {
            "text": qsTr("Zoom out"),
            "fn": () => zoomOut()
        }, {
            "text": qsTr("Reset zoom"),
            "fn": () => resetZoom()
        }, "-", {
            "text": qsTr("Clear"),
            "fn": () => s.ext.clear()
        }, "-");
        if (s.logOn)
            out.push({
                "text": qsTr("Stop session log"),
                "fn": () => s.core.stopLog()
            });
        else
            out.push({
                "text": qsTr("Log output to file…"),
                "tip": qsTr("Append this terminal’s output to a file (audit / debug)"),
                "fn": () => view.askLog()
            });
        return out;
    }

    function askLog() {
        host.askSave(qsTr("Session log file"), session.core.homeFile("termigator-session.log"), [qsTr("Log files (*.log)"), qsTr("All files (*)")], path => {
            const err = session.core.startLog(path);
            if (err !== "")
                area.message(err);
        });
    }

    function showMenu(x, y) {
        while (menu.count > 0)
            menu.takeItem(0).destroy();
        let afterSep = true;
        for (const e of menuEntries(x, y)) {
            if (e === "-") {
                if (!afterSep)
                    menu.addItem(separatorComponent.createObject(null));
                afterSep = true;
                continue;
            }
            menu.addItem(itemComponent.createObject(null, {
                "text": e.text,
                "enabled": e.enabled !== false,
                "tip": e.tip || "",
                "fn": e.fn
            }));
            afterSep = false;
        }
        if (afterSep && menu.count > 0)
            menu.takeItem(menu.count - 1).destroy();
        menu.popup(term, x, y);
    }

    onSeenChanged: {
        if (seen && session)
            session.core.tabSeen();
    }

    Component.onCompleted: {
        host.ensureSession(tab);
        if (seen && session)
            session.core.tabSeen();
    }

    Connections {
        target: view.session

        function onOverlayActiveChanged() {
            if (view.session.overlayActive)
                view.takeFocusIfHere();
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Item {
            id: screen

            Layout.fillWidth: true
            Layout.fillHeight: true

            Loader {
                anchors.left: parent.left
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                anchors.right: parent.right
                // The bar sits beside the terminal, not over its text, and is always there (like Konsole:
                // a full-length handle with nothing to scroll), so the terminal's width never changes with it.
                anchors.rightMargin: thumb.width
                active: view.session !== null
                focus: true
                sourceComponent: termComponent
            }

            // The scroll bar is the style's own: always shown, and it highlights on hover like
            // the file lists'.
            Rectangle {
                anchors.fill: thumb
                color: view.term ? view.term.fillColor : "transparent" // the terminal's own background
            }

            QQC2.ScrollBar {
                id: thumb

                objectName: "termScrollThumb"
                readonly property int total: view.term ? view.term.lines + view.term.scrollbarMaximum : 0

                anchors.right: parent.right
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                orientation: Qt.Vertical
                policy: QQC2.ScrollBar.AlwaysOn
                // Below 1 even with nothing to scroll: the desktop style hides a bar of size 1, Konsole
                // shows a full-length handle.
                size: view.term ? Math.min(0.999, view.term.lines / Math.max(1, total - view.term.scrollbarMinimum)) : 0.999
                // Not a binding on `position`: dragging the handle sets it, which would break one.
                onPositionChanged: {
                    if (pressed && view.term)
                        view.scrollToValue(view.term.scrollbarMinimum + position * total);
                }

                Binding {
                    target: thumb
                    property: "position"
                    value: view.term ? (view.term.scrollbarCurrentValue - view.term.scrollbarMinimum) / Math.max(1, thumb.total) : 0
                    when: !thumb.pressed
                }
            }

            // Restart hint: a floating label over the dead session, near the top edge
            // (the freshest output sits at the bottom). Mouse events go through to the terminal
            // (the error text stays selectable); the text is plain ("<Enter>" is literal).
            Rectangle {
                id: overlay

                readonly property int pad: 14
                readonly property string overlayMessage: view.session ? view.session.overlayText : ""

                visible: view.session !== null && view.session.overlayActive
                width: Math.min(Math.ceil(overlayMeasure.implicitWidth) + pad * 2, Math.max(120, screen.width - 24))
                height: overlayText.implicitHeight + pad * 2
                x: (screen.width - width) / 2
                y: 8
                radius: 6
                color: Qt.rgba(20 / 255, 20 / 255, 20 / 255, 210 / 255)
                border.width: 1
                border.color: Qt.rgba(1, 204 / 255, 102 / 255, 130 / 255)

                Text {
                    id: overlayText

                    x: overlay.pad
                    y: overlay.pad
                    width: overlay.width - overlay.pad * 2
                    text: overlay.overlayMessage
                    textFormat: Text.PlainText
                    wrapMode: Text.WordWrap
                    lineHeight: 1.22
                    color: "#ffcc66"
                    font.weight: Font.DemiBold
                    Accessible.role: Accessible.StaticText
                    Accessible.name: text
                }

                // Unwrapped copy: its implicitWidth is the widest line (TextMetrics would add the lines up).
                Text {
                    id: overlayMeasure

                    visible: false
                    text: overlay.overlayMessage
                    textFormat: Text.PlainText
                    font: overlayText.font
                }
            }
        }

        TerminalFindBar {
            id: findBar

            Layout.fillWidth: true
            term: view.term
            onClosed: view.focusTerminal()
        }
    }

    // A marked session is framed in its environment's colour.
    EnvMark {
        objectName: "envFrame"
        width: view.width
        height: view.height
        z: 100
        frame: true
        environment: view.tab.environment
    }

    Component {
        id: termComponent

        QMLTermWidget {
            id: terminal

            // Set at creation and never again: a view stays on the session it was made for.
            session: view.session.ext
            font.family: view.session.fontFamily
            font.pointSize: view.session.fontSize
            colorScheme: view.session.scheme
            blinkingCursor: view.session.look.blinking_cursor
            bracketedPasteDisabled: !view.session.look.bracketed_paste
            confirmMultilinePaste: view.session.look.confirm_multiline
            copyOnSelect: view.session.look.copy_on_select
            pasteOnRightClick: view.session.look.paste_on_right_click
            pasteOnMiddleClick: view.session.look.paste_on_middle_click
            focus: true
            Accessible.role: Accessible.Terminal
            Accessible.name: view.tab.text

            Keys.priority: Keys.BeforeItem
            Keys.onPressed: event => view.handleKey(event)
            onMultilinePasteRequested: (text, appendReturn) => view.confirmPaste(text, appendReturn)
            onLinkActivated: url => view.openLink(url)
            // A right click without paste-on-right-click, or Ctrl+right click with it.
            onConfigureRequest: position => view.showMenu(position.x, position.y)

            // The terminal does not take the focus on a click by itself.
            TapHandler {
                acceptedButtons: Qt.AllButtons
                onPressedChanged: if (pressed)
                    terminal.forceActiveFocus()
            }

            Component.onCompleted: {
                view.term = terminal;
            }
        }
    }

    QQC2.Menu {
        id: menu
    }

    Component {
        id: itemComponent

        QQC2.MenuItem {
            property var fn
            property string tip

            QQC2.ToolTip.visible: hovered && tip !== ""
            QQC2.ToolTip.text: tip
            QQC2.ToolTip.delay: 700
            onTriggered: {
                fn();
                view.focusTerminal();
            }
        }
    }

    Component {
        id: separatorComponent

        QQC2.MenuSeparator {}
    }

    QQC2.Dialog {
        id: pasteDialog

        property string text
        property bool appendReturn

        title: qsTr("Paste multiline text")
        modal: true
        parent: QQC2.Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(Math.max(pasteBody.implicitWidth + Kirigami.Units.gridUnit * 4, 480),
            (parent ? parent.width : 800) - Kirigami.Units.gridUnit * 2)
        padding: Kirigami.Units.largeSpacing
        standardButtons: QQC2.Dialog.Yes | QQC2.Dialog.No
        onAccepted: if (view.term)
            view.term.pasteText(text, appendReturn)
        onClosed: view.focusTerminal()

        contentItem: ColumnLayout {
            id: pasteBody

            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: qsTr("Are you sure you want to paste this text?")
            }

            QQC2.ScrollView {
                id: pasteScroll

                Layout.fillWidth: true
                Layout.preferredWidth: pasteDialog.width - pasteDialog.padding * 2
                Layout.preferredHeight: Math.min(
                    Math.max(Kirigami.Units.gridUnit * 8, pastePreview.contentHeight + Kirigami.Units.gridUnit),
                    Kirigami.Units.gridUnit * 18,
                    (pasteDialog.parent ? pasteDialog.parent.height * 0.55 : 420))
                clip: true

                QQC2.TextArea {
                    id: pastePreview

                    readOnly: true
                    text: pasteDialog.text
                    font: Kirigami.Theme.fixedWidthFont
                    wrapMode: TextEdit.NoWrap
                    selectByMouse: true
                    Accessible.name: qsTr("Text to paste")

                    Kirigami.WheelHandler {
                        target: pasteScroll.contentItem
                    }
                }
            }
        }
    }
}
