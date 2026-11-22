import QtQuick

// Smoke-only (Windows): imports this machine's MobaXterm (registry or MobaXterm.ini), PuTTY
// (registry) and SuperPuTTY (Documents\SuperPuTTY\Sessions.xml) sessions without a file picker and
// logs what was found and the result texts. The test data is set up by the caller.
Item {
    id: check

    required property var shell
    readonly property var backend: shell.importHost.backend

    Timer {
        interval: 500
        running: true
        onTriggered: {
            const b = check.backend;
            console.log("import check: moba", b.mobaLocal());
            console.log("import check: putty", b.puttyLocalCount(), b.puttySessionsDir());
            const sp = b.superputtyPath();
            console.log("import check: superputty", JSON.stringify(sp));
            for (const [name, res] of [["moba", b.importMoba("")], ["putty", b.importPutty("")], ["superputty", sp ? b.importPutty(sp) : "{}"]])
                console.log("import check:", name, JSON.stringify(JSON.parse(res).text));
            console.log("import check: persist", check.shell.controller.persist());
            exit.start();
        }
    }

    // Let the save land first.
    Timer {
        id: exit

        interval: 1500
        onTriggered: Qt.exit(0)
    }
}
