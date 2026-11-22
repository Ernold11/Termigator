import QtQuick
import QtQuick.Controls as QQC2

// The "Credential" combo of the SSH and RDP editors. Picking a named credential locks the
// user (and domain) fields to its values and shows "from credential …" in the password field;
// "(own login)" gives the fields back with what the user had typed.
QtObject {
    id: picker

    required property var combo
    required property var user
    required property var password
    property var domain: null
    // The credentials of the config: [{name, label, user, domain}].
    property var credentials: []
    readonly property string name: currentName()
    readonly property bool shared: name !== ""
    // What the fields held before a credential was picked.
    property var own: ({
            "user": "",
            "domain": "",
            "password": ""
        })

    // A function so sync() sees the new choice at once: the `name` binding follows only after
    // the combo's currentIndexChanged handlers have run.
    function currentName() {
        return combo.currentIndex >= 0 ? String(combo.valueAt(combo.currentIndex)) : "";
    }

    function reload(list, current) {
        credentials = list;
        const rows = [
            {
                "text": qsTr("(own login — fields below)"),
                "value": ""
            }
        ];
        for (const c of list)
            rows.push({
                "text": c.label,
                "value": c.name
            });
        // A dangling reference is shown rather than silently dropped.
        if (current !== "" && !list.some(c => c.name === current))
            rows.push({
                "text": qsTr("%1  (missing)").arg(current),
                "value": current
            });
        combo.model = rows;
        combo.setValue(current);
        combo.visible = list.length > 0 || current !== "";
    }

    // The dialog filled the fields: remember them as the entry's own login, then apply the choice.
    function remember() {
        own = snapshot();
        sync();
    }

    function snapshot() {
        return {
            "user": user.text,
            "domain": domain ? domain.text : "",
            "password": password.text
        };
    }

    function sync() {
        const chosen = currentName();
        const cred = credentials.find(c => c.name === chosen) || null;
        const locked = chosen !== "";
        if (locked && user.enabled)
            own = snapshot();
        for (const w of [user, password, domain]) {
            if (!w)
                continue;
            w.readOnly = locked;
            w.enabled = !locked;
        }
        if (locked) {
            user.text = cred ? cred.user : "";
            if (domain)
                domain.text = cred ? cred.domain : "";
            password.text = "";
        } else {
            user.text = own.user;
            if (domain)
                domain.text = own.domain;
            password.text = own.password;
        }
    }
}
