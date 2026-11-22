import QtQuick

// "(direct)" or one of the saved SSH entries to tunnel through; a stored name that is no
// longer an SSH entry is kept and marked "(missing)".
DataCombo {
    function reload(names, value) {
        const rows = [
            {
                "text": qsTr("(direct — no SSH gateway)"),
                "value": ""
            }
        ];
        for (const g of names)
            rows.push({
                "text": g,
                "value": g
            });
        if (value !== "" && !names.includes(value))
            rows.push({
                "text": qsTr("%1  (missing)").arg(value),
                "value": value
            });
        model = rows;
        setValue(value);
    }

    tip: qsTr("Reach this desktop through an SSH host (ssh -L in the background, for the\nsession's lifetime) — for machines only visible behind a bastion.")
}
