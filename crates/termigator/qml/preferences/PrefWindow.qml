import QtQuick
import QtQuick.Window
import Termigator

// What the settings pages and controls use of their window; PreferencesDialog derives from it.
// A type of its own because typing them as PreferencesDialog, which contains the pages, makes a
// cycle that Qt 6.10 refuses to load ("Cyclic dependency detected", the main window then fails).
Window {
    // The edited copy of the page values (see PreferencesDialog.qml).
    property var draft: ({})
    // Apply is enabled by any edit, even one that is changed back.
    property bool dirty: false
    property PreferencesController controller: null

    // Enter in a field: the dialog applies (when dirty) and closes.
    signal okRequested

    function edit(key, value) {
        draft = Object.assign({}, draft, {
            [key]: value
        });
        dirty = true;
    }

    function ok() {
        okRequested();
    }
}
