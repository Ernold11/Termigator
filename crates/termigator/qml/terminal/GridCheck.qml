import QtQuick
import Termigator.Terminal.Probe

// Offscreen check (tests/grid_smoke.rs, run at several QT_SCALE_FACTORs): glyphs land on their
// cells. A cell with a background but no text (mc paints its panels with ED/ECH) still takes its
// column, and a box corner meets the line below it (k9s): fractional scaling rounds a glyph's
// advance differently from the cell grid, so a long run of "─" drifts off it.
Item {
    id: check

    required property var shell
    property var tab: null
    property bool failed: false

    function fail(reason) {
        failed = true;
        console.error("termigator grid: FAILED: " + reason);
        Qt.exit(1);
    }

    function expect(ok, what) {
        if (!ok && !failed)
            fail(what);
        return ok;
    }

    function findArea(item) {
        if (item.objectName === "panesArea")
            return item;
        for (const c of item.children) {
            const f = findArea(c);
            if (f)
                return f;
        }
        return null;
    }

    function run() {
        const area = findArea(shell.contentItem);
        if (!area)
            return false;
        if (!tab) {
            shell.openInner("exec cat", "grid", "embedded", "");
            tab = area.activeTab;
            return false;
        }
        const v = area.terminalHost.viewFor(tab);
        if (!v || !v.term || v.term.width < 200 || !tab.session)
            return false;
        const term = v.term;
        const ext = check.tab.session.ext;
        const font = term.font;
        // Fractional sizes too: there a glyph's own advance drifts from the cell width.
        const looks = [];
        for (const family of [font.family, "DejaVu Sans Mono", "Hack", "Noto Mono"])
            for (const size of [font.pointSize, 9.7, 10, 10.3, 11.1, 13.7])
                looks.push({ "family": family, "pointSize": size });
        for (const look of looks) {
            const size = look.family + " " + look.pointSize;
            term.font = Qt.font(look);
            const cw = term.cellWidth;
            const ch = term.cellHeight;
            const cols = Math.floor(term.width / cw);
            const span = cols - 3; // the line between the corners
            ext.feedBytes("\x1b[44m\x1b[2J\x1b[HA\x1b[10C|\x1b[0m\x1b[3;1H\x1b[J\u250c" + "\u2500".repeat(span) + "\u2510\x1b[4;1H\u2502\x1b[4;" + (span + 2) + "H\u2502\x1b[H");
            const what = " (" + size + " pt, cell " + cw + " px), ink ";
            const bar = Probe.inkSpans(term, Math.floor(ch / 2), Math.floor(5.5 * cw));
            const strayInk = bar.some((x, i) => i % 2 === 0 && x >= Math.ceil(cw) && x < Math.floor(11 * cw));
            const barInk = bar.some((x, i) => i % 2 === 0 && x >= Math.floor(11 * cw) && bar[i + 1] <= Math.ceil(12 * cw));
            expect(bar.length >= 4 && bar[0] < cw && !strayInk && barInk,
                   "'A' at column 0 and '|' at column 11" + what + JSON.stringify(bar));
            // The horizontal line of the corner ends on the vertical line below it.
            // The stroke is a pixel or two thick somewhere in the row: scan the row for it.
            let lineEnd = -1;
            for (let y = 2 * ch; y < 3 * ch; y += 0.25) {
                const spans = Probe.inkSpans(term, y, (span + 2.5) * cw);
                if (spans.length >= 2 && spans[0] < cw && spans[1] > lineEnd)
                    lineEnd = spans[1];
            }
            const side = Probe.inkSpans(term, 3.5 * ch, (span + 2.5) * cw);
            expect(side.length === 4 && Math.abs(lineEnd - side[3]) <= 1 && Math.abs(side[3] - (span + 1.5) * cw) <= cw / 2,
                   "'\u2510' ends the line over the '\u2502' below" + what + "line end " + lineEnd + ", side " + JSON.stringify(side));
        }
        term.font = font;
        // A selection shows over a coloured background (mc's panels) and over the empty cells a
        // prompt skips with CSI n C: a block over columns 0-9 is one span against an unselected
        // cell of the same row (blue on row 0, the default background on row 1).
        const cw = term.cellWidth;
        const ch = term.cellHeight;
        ext.feedBytes("\x1b[0m\x1b[2J\x1b[H\x1b[44m" + " ".repeat(30) + "\x1b[0m\r\nab\x1b[5Ccd\x1b[H");
        term.select(0, 0, 9, 1, true);
        for (const row of [0, 1]) {
            const spans = Probe.inkSpans(term, (row + 0.5) * ch, 20.5 * cw);
            expect(spans.length >= 2 && spans[0] < 1 && Math.abs(spans[1] - 10 * cw) <= cw / 2,
                   "selection highlighted over columns 0-9 on row " + row + ": " + JSON.stringify(spans) + ", cell " + cw);
        }
        term.select(0, 0, 0, 0, false);
        ext.feedBytes("\x1b[2J\x1b[H");
        if (check.failed)
            return true;
        console.info("GRID_DONE");
        Qt.exit(0);
        return true;
    }

    Timer {
        property int waited: 0

        interval: 100
        repeat: true
        running: !check.failed
        onTriggered: {
            let done;
            try {
                done = check.run();
            } catch (e) {
                check.fail("exception: " + e);
                return;
            }
            if (done !== false)
                running = false;
            else if ((waited += 100) > 10000)
                check.fail("timed out waiting for the terminal");
        }
    }
}
