import QtQuick

// Preferences: clipboard options.
PrefPage {
    title: qsTr("Clipboard")

    PrefCheck {
        key: "confirm_multiline_paste"
        text: qsTr("Ask before pasting several lines")
        tip: qsTr("A pasted multi-line text runs line by line — confirm it first.")
    }
    PrefCheck {
        key: "copy_on_select"
        text: qsTr("Copy selected text automatically")
    }
    PrefCheck {
        key: "paste_on_right_click"
        text: qsTr("Paste with a right-click")
        tip: qsTr("The context menu then opens with Ctrl+right-click")
    }
    PrefHint {
        text: qsTr("With right-click paste on, Ctrl+right-click opens the terminal's menu.")
    }
    PrefCheck {
        key: "paste_on_middle_click"
        text: qsTr("Paste the selection with a middle-click")
        tip: qsTr("Select text, then middle-click to paste it; the clipboard stays as it is.")
    }
    PrefHint {
        text: qsTr("Hold Shift to select or paste in programs that use the mouse (mc, tmux, vim).")
    }
    PrefCheck {
        key: "allow_clipboard_write"
        text: qsTr("Let programs copy to the clipboard")
        tip: qsTr("vim, tmux or nvim, also on a remote host, can then put text on the clipboard (OSC 52). They can never read it.")
    }
}
