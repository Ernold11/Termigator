import QtQuick

// Smoke-only check (tests/theme_smoke.rs): switches Preferences → Colours to the theme named by
// `--switch-theme=<light|dark>` and applies it, then saves the toolbar (<file>.bar.png) and the
// window content, so a switch without a restart can be inspected.
Item {
    id: check

    required property var shell
    readonly property string target: Qt.application.arguments.filter(a => a.startsWith("--switch-theme=")).map(a => a.slice(15))[0] || "dark"

    Timer {
        interval: 1500
        running: true
        onTriggered: {
            check.shell.openPreferences();
            const dlg = check.shell.preferencesDialog;
            const draft = Object.assign({}, dlg.draft);
            draft.ui_theme = check.target;
            dlg.draft = draft;
            dlg.apply();
            dlg.close();
            grab.start();
        }
    }

    Timer {
        id: grab

        interval: 1500
        onTriggered: {
            const file = check.shell.controller.smokeTestShot;
            check.shell.header.grabToImage(bar => {
                bar.saveToFile(file.replace(/\.png$/, ".bar.png"));
                check.shell.contentArea.grabToImage(r => Qt.exit(r.saveToFile(file) ? 0 : 2));
            });
        }
    }

    Timer {
        interval: 30000
        running: true
        onTriggered: Qt.exit(4)
    }
}
