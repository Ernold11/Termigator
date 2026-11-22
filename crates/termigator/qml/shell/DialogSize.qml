import QtQuick

// A dialog window reopens at the size it was left at. Put one inside the dialog's Window:
//   DialogSize { window: dlg; name: "SnippetsDialog"; controller: … }
// Restored when created, stored in window.dialog_sizes.<name> when hidden (written with the next
// config save; the legacy window.dialogs key is never touched, ADR-13). Use the dialog's own
// name.
QtObject {
    id: keeper

    required property Window window
    required property string name
    required property var controller

    property Connections watcher: Connections {
        target: keeper.window

        function onVisibleChanged() {
            if (!keeper.window.visible)
                keeper.controller.setDialogSize(keeper.name, keeper.window.width, keeper.window.height);
        }
    }

    Component.onCompleted: {
        const size = controller.dialogSize(name);
        if (size.width > 0 && size.height > 0) {
            window.width = size.width;
            window.height = size.height;
        }
    }
}
