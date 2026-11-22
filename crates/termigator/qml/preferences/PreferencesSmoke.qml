import QtQuick
import QtQuick.Window
import Termigator.Terminal.Probe

// Offscreen check of the settings dialog (tests/preferences_smoke.rs): opens it through the
// Settings action, edits with real key presses, Apply / Restore Defaults / Cancel / OK, then saves
// a grab of the last page and quits with 0 (1 after a console.error).
Item {
    id: check

    required property var shell
    property int step: 0
    property int applied: 0
    property int changed: 0
    property var dlg: null

    Component.onCompleted: {
        shell.openPreferences();
        dlg = shell.preferencesDialog;
    }

    function fail(reason) {
        console.error("termigator prefs smoke: FAILED at step " + step + ": " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok)
            fail(what);
    }

    // The editor with objectName `name` somewhere below `item`.
    function find(name, item) {
        item = item || dlg.contentItem;
        if (item.objectName === name)
            return item;
        for (const child of item.children) {
            const hit = find(name, child);
            if (hit)
                return hit;
        }
        return null;
    }

    // WCAG contrast ratio of two colours.
    function contrast(a, b) {
        const lum = c => {
            const f = v => v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
            return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
        };
        const la = lum(a), lb = lum(b);
        return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
    }

    function stored() {
        return JSON.parse(dlg.controller.load());
    }

    function press(key) {
        Probe.key(dlg, key, Qt.NoModifier, "");
    }

    // Model entries far longer than the page: a host with hundreds of fonts, or the CI container
    // with one, must lay out the same (a combo's implicit width follows its longest entry).
    readonly property string longName: "Extremely Long Monospace Family Name Nerd Font Propo Condensed Semibold Italic Oblique"

    // Every visible item below `item` that sticks out of `page` (coordinates mapped into the page).
    function overflows(item, page, out) {
        for (const child of item.children) {
            if (!child.visible || child.width <= 0 || child.height <= 0)
                continue;
            const p = child.mapToItem(page, 0, 0);
            if (p.x < -1 || p.x + child.width > page.width + 1)
                out.push(child + " (" + child.objectName + ") x=" + Math.round(p.x) + " w=" + Math.round(child.width) + " implicit=" + Math.round(child.implicitWidth) + " page=" + Math.round(page.width));
            overflows(child, page, out);
        }
        return out;
    }

    // Two steps per page: show it (the layout settles), then check that the form fits the page.
    function geometrySteps() {
        const list = [];
        for (let i = 0; i < 6; ++i) {
            list.push(() => {
                dlg.width = dlg.minimumWidth;
                dlg.pageList.currentIndex = i;
            });
            list.push(() => {
                const page = dlg.pageStack.children[i];
                const out = overflows(page, page, []);
                expect(dlg.width === dlg.minimumWidth && page.visible, "page " + i + " not shown at the minimum width");
                expect(out.length === 0, "page " + i + " overflows: " + out.join("; "));
                expect(page.neededHeight === undefined || dlg.screenTooSmall || page.neededHeight <= page.height + 1, "page " + i + " lower than its form: needs " + Math.round(page.neededHeight) + ", has " + Math.round(page.height));
            });
        }
        return list;
    }

    readonly property var steps: [
        // Settings → Configure Termigator… opens it; nothing is changed yet.
        () => {
            dlg.fonts = [longName, "Mono"];
            const act = shell.shellActions.actions["settings.configure"];
            expect(act.enabled, "action not handled");
            act.trigger();
        },
        () => {
            expect(dlg.visible, "dialog not shown");
            // The page it opens on fits the window (no text under the buttons, no scrolling).
            const first = dlg.pageStack.children[0];
            expect(dlg.screenTooSmall || first.neededHeight <= first.height + 1, "the first page does not fit: needs " + Math.round(first.neededHeight) + ", has " + Math.round(first.height));
            const titles = [];
            for (let i = 0; i < dlg.pageList.count; ++i) {
                dlg.pageList.currentIndex = i;
                titles.push(dlg.pageList.model[i].title);
            }
            expect(titles.join(",") === "Appearance,Terminal,Clipboard,Logging,Connections,Labels,Application", "pages " + titles);
            expect(dlg.pageStack.count === 7, "page count");
            const envPage = dlg.pageStack.itemAt(5);
            expect(envPage.environments.length === 4 && envPage.environments[0].name === "Production" && envPage.environments[0].color === "#da4453", "environment categories " + JSON.stringify(envPage.environments));
            dlg.pageList.currentIndex = 0;
            expect(!dlg.applyButton.enabled && !dlg.dirty, "Apply enabled before an edit");
            expect(find("font_size").value === 0, "font size " + find("font_size").value);
            expect(find("font_size").textFromValue(0, Qt.locale()) === "Konsole's default", "special text");
            expect(find("embedded_pyte_history").value === 10000, "history " + find("embedded_pyte_history").value);
            expect(find("color_scheme").model.indexOf("Solarized") > 0, "built-in schemes: " + find("color_scheme").model);
            expect(find("font_family").editText === "" && find("color_scheme").editText === "", "free-text combos not empty");
            // The "default"/hint text of a field stays readable (the style's own colour was too faint on light).
            const hint = find("rdp_client");
            expect(hint && contrast(hint.placeholderTextColor, Qt.color(dlg.color)) >= 4, "placeholder contrast " + (hint ? contrast(hint.placeholderTextColor, Qt.color(dlg.color)) : "no field"));
            expect(find("default_terminal").currentIndex === 0 && find("default_terminal").currentText === "Embedded in Termigator", "open sessions");
            expect(find("quick_style").count > 0 && find("quick_style").currentValue === "org.kde.desktop", "style combo " + find("quick_style").currentValue);
            expect(!find("style_restart_hint").visible, "restart hint before a style change");
            expect(find("bracketed_paste").checked && !find("copy_on_select").checked && find("paste_on_middle_click").checked, "check boxes");
            expect(find("ssh_keepalive_seconds").enabled, "keepalive seconds");
        },
        // Long model entries must not widen the form beyond the page, at the minimum window width.
        () => {
            dlg.schemes = dlg.schemes.concat([longName]);
            dlg.styles = dlg.styles.concat([longName]);
        },
        ...geometrySteps(),
        () => {
            dlg.pageList.currentIndex = 0;
        },
        // A key press in the font size box is an edit: Apply becomes enabled, the config does not move.
        () => {
            const spin = find("font_size");
            spin.forceActiveFocus();
            press(Qt.Key_Up);
            press(Qt.Key_Up);
        },
        () => {
            expect(find("font_size").value === 2 && dlg.dirty && dlg.applyButton.enabled, "spin edit " + find("font_size").value);
            expect(stored().font_size === 0, "config changed before Apply");
            dlg.applyButton.click();
        },
        () => {
            expect(stored().font_size === 2 && !dlg.dirty && !dlg.applyButton.enabled, "Apply: " + stored().font_size);
            expect(applied === 1 && changed === 1, "applied " + applied + " configChanged " + changed);
        },
        // Environments: typing renames a row without rebuilding it, a new row takes its own
        // colour, and Apply stores both.
        () => {
            const env = dlg.pageStack.itemAt(5);
            env.addEnvironment();
        },
        () => {
            const env = dlg.pageStack.itemAt(5);
            expect(env.rowModel.count === 5, "added row " + env.rowModel.count);
            const field = env.rowList.itemAtIndex(0).children[0];
            field.forceActiveFocus();
            field.selectAll();
            Probe.key(dlg, Qt.Key_X, Qt.NoModifier, "X");
            Probe.key(dlg, Qt.Key_Y, Qt.NoModifier, "Y");
            env.colorIndex = 4;
            env.rowModel.setProperty(4, "color", "#112233");
            env.commit();
        },
        () => {
            const env = dlg.pageStack.itemAt(5);
            expect(env.rowList.itemAtIndex(0).children[0].text === "XY", "typed name " + env.rowList.itemAtIndex(0).children[0].text);
            expect(dlg.draft.environments[0].name === "XY" && dlg.draft.environments[4].color === "#112233", "draft " + JSON.stringify(dlg.draft.environments));
            dlg.applyButton.click();
        },
        () => {
            const envs = stored().environments;
            expect(envs.length === 5 && envs[0].name === "XY" && envs[0].id === "production" && envs[4].color === "#112233", "stored environments " + JSON.stringify(envs));
        },
        // Restore Defaults: pages back to defaults, dirty, config untouched until Apply / OK.
        () => {
            dlg.restoreButton.click();
        },
        () => {
            expect(find("font_size").value === 0 && dlg.dirty && dlg.applyButton.enabled, "restore");
            expect(stored().font_size === 2, "Restore Defaults saved");
            dlg.close();
        },
        // Cancel dropped the draft; reopening reads the config (2).
        () => {
            expect(!dlg.visible, "not closed");
            shell.shellActions.actions["settings.configure"].trigger();
        },
        () => {
            expect(dlg.visible && find("font_size").value === 2 && !dlg.dirty, "reopen " + find("font_size").value);
            // Unlimited scrollback greys the line count; Terminal page via the page list keys.
            dlg.pageList.forceActiveFocus();
            press(Qt.Key_Down);
        },
        () => {
            expect(dlg.pageList.currentIndex === 1 && dlg.pageStack.currentIndex === 1, "page keys");
            find("scrollback_unlimited").click();
            find("shell").forceActiveFocus();
            find("shell").text = "/bin/zsh";
            find("shell").textEdited();
            dlg.pageList.currentIndex = 0;
            const scheme = find("color_scheme");
            scheme.activated(scheme.find("Solarized"));
            dlg.pageList.currentIndex = 0;
            const style = find("quick_style");
            const at = style.indexOfValue("Fusion");
            if (at >= 0) {
                style.currentIndex = at;
                style.activated(at);
            }
        },
        () => {
            expect(find("color_scheme").editText === "Solarized", "scheme text " + find("color_scheme").editText);
            expect(!find("embedded_pyte_history").enabled, "line count still enabled");
            if (find("quick_style").indexOfValue("Fusion") >= 0)
                expect(find("quick_style").currentValue === "Fusion" && find("style_restart_hint").visible, "style choice / restart hint");
            expect(dlg.dirty, "not dirty");
            dlg.okButton.click();
        },
        // OK applies (dirty) and closes.
        () => {
            expect(!dlg.visible, "OK did not close");
            expect(applied === 3 && changed === 3, "OK applied " + applied);
            const v = stored();
            expect(v.scrollback_unlimited === true && v.shell === "/bin/zsh" && v.color_scheme === "Solarized" && v.font_size === 2, "stored " + JSON.stringify(v));
            if (find("quick_style").indexOfValue("Fusion") >= 0) {
                expect(v.quick_style === "Fusion", "stored style " + v.quick_style);
                // A new style: OK asks whether to restart now ("No" keeps this process).
                expect(shell.restartBox.visible, "no restart question after OK");
                shell.restartBox.close();
            }
            shell.shellActions.actions["settings.configure"].trigger();
        },
        // OK without an edit applies nothing.
        () => {
            expect(dlg.visible && !dlg.dirty, "reopened dirty");
            dlg.okButton.click();
        },
        () => {
            expect(!dlg.visible && applied === 3, "clean OK applied " + applied);
            shell.shellActions.actions["settings.configure"].trigger();
        },
        () => {
            dlg.pageList.currentIndex = 0;
        },
        () => {
            const started = dlg.contentItem.grabToImage(result => {
                if (!result.saveToFile(shell.controller.smokeTestShot))
                    check.fail("cannot save the grab");
                console.info("termigator prefs smoke: done");
                Qt.exit(0);
            });
            if (!started)
                fail("grab not started");
        }
    ]

    Connections {
        target: check.dlg

        function onApplied() {
            ++check.applied;
        }
    }

    Connections {
        target: check.shell.controller

        function onConfigChanged() {
            ++check.changed;
        }
    }

    Timer {
        interval: 200
        repeat: true
        running: check.step < check.steps.length
        onTriggered: {
            const i = check.step++;
            check.steps[i]();
        }
    }
}
