import QtQuick
import Termigator.Terminal.Probe

// Quick-connect history offscreen (tests/quick_connect_smoke.rs): the toolbar field shows the
// stored history newest first (the password-shaped entry scrubbed), Up/Down/Enter/Escape and
// Shift+Delete work as key presses, a click on the field and on a row picks with the mouse, and
// "Clear history" empties it. The handler is replaced by a stub so nothing connects.
Item {
    id: check

    required property var shell
    property int step: 0
    property int tries: 0
    property var field: null
    property var picked: []

    function fail(reason) {
        console.error("termigator quick: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok)
            fail(what);
    }

    function press(key, modifiers, text) {
        Probe.key(shell, key, modifiers || 0, text || "");
    }

    function findByName(name, item) {
        if (!item)
            return null;
        if (item.objectName === name)
            return item;
        for (const c of item.children) {
            const f = findByName(name, c);
            if (f)
                return f;
        }
        return null;
    }

    function clickItem(item) {
        const p = item.mapToItem(null, item.width / 2, item.height / 2);
        Probe.click(shell, p.x, p.y, Qt.LeftButton, 1, 0);
    }

    function rows() {
        return field.matches.map(r => r.text).join(",");
    }

    readonly property var steps: [
        // Down on the empty field opens the history, newest first, the leaked password gone.
        () => {
            field.handler = text => {
                check.picked.push(text);
                return "";
            };
            shell.requestActivate();
            field.forceActiveFocus();
            press(Qt.Key_Down);
        },
        () => {
            expect(field.popup.visible, "Down did not open the history");
            expect(rows() === "u@new,u@old,web prod", "rows " + rows());
            expect(field.list.currentIndex === 0, "current " + field.list.currentIndex);
            press(Qt.Key_Down);
            press(Qt.Key_Down);
            press(Qt.Key_Up);
            expect(field.list.currentIndex === 1, "after Down Down Up " + field.list.currentIndex);
        },
        // Shift+Delete forgets the current row and saves.
        () => {
            press(Qt.Key_Delete, Qt.ShiftModifier);
        },
        () => {
            expect(rows() === "u@new,web prod", "after Shift+Delete " + rows());
            expect(shell.controller.quickHistory().join(",") === "u@new,web prod", "not forgotten in the config");
            press(Qt.Key_Escape);
        },
        // Enter on a picked row connects to it.
        () => {
            expect(!field.popup.visible, "Escape did not close the list");
            press(Qt.Key_Down);
            press(Qt.Key_Down);
            press(Qt.Key_Return);
        },
        () => {
            expect(check.picked.join(",") === "web prod", "Enter picked " + check.picked);
            expect(field.text === "", "field not cleared");
            // Mouse: a click on the field shows the list, a click on a row connects to it.
            clickItem(field);
        },
        () => {
            expect(field.popup.visible, "a click did not open the history");
            clickItem(field.list.itemAtIndex(0));
        },
        () => {
            expect(check.picked.join(",") === "web prod,u@new", "click picked " + check.picked);
            // Typing filters (history and entry labels).
            field.text = "";
            press(Qt.Key_W, 0, "w");
        },
        () => {
            expect(field.popup.visible && rows() === "u@new,web prod", "filter w: " + rows());
            press(Qt.Key_E, 0, "e");
        },
        () => {
            expect(rows() === "web prod", "filter we: " + rows());
            expect(!field.list.footerItem.visible, "Clear history shown while filtering");
            field.text = "";
            clickItem(field);
        },
        // A row's remove button, then "Clear history".
        () => {
            expect(field.popup.visible, "no list for the remove button");
            clickItem(findByName("forget", field.list.itemAtIndex(1)));
        },
        () => {
            expect(rows() === "u@new", "after the remove button " + rows());
            clickItem(field.list.footerItem);
        },
        () => {
            expect(field.history.length === 0, "history not cleared");
            expect(!field.popup.visible, "list open with nothing in it");
            console.info("termigator quick: ok");
            Qt.exit(0);
        }
    ]

    Timer {
        interval: 150
        repeat: true
        running: true
        onTriggered: {
            if (!check.field) {
                const f = check.findByName("quickConnect", check.shell.header);
                if (f && f.handler) {
                    check.field = f;
                } else if (++check.tries > 60) {
                    check.fail("no quick-connect field with a handler");
                }
                return;
            }
            if (check.step < check.steps.length) {
                check.steps[check.step]();
                ++check.step;
            }
        }
    }
}
