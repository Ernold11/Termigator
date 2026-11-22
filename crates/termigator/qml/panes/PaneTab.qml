import QtQuick

// One tab. It outlives its views: a tab that
// moves (split, detach, attach) keeps this object while its view is created anew where it lands,
// so anything that must survive the move (the session) belongs here, not in the view.
// Who sets what is listed at the top of PanesArea.qml.
QtObject {
    required property int tabId

    // Set when the tab opens (PanesArea.addTab); Rename sets title and text.
    property string title          // termigator_title: the full title (snapshot, restart, duplicate)
    property string text           // the tab bar's text: title cut to 32 characters
    property string inner          // the command (snapshot, restart, duplicate)
    property var extraEnv: null    // object of strings for the spawn, or null; never saved
    property var conn: null        // the connection entry (as stored), or null for a plain command
    property bool embedded: true   // an embedded terminal: counted, snapshotted, Duplicate/Reconnect
    property Component content: null // this tab's view instead of PanesArea.tabContent (the Files tab)
    property bool detectDistro: false // watch for the distro (SSH entries opened from the list)

    // Set by the view's owner (the terminal, the Files tab).
    property bool live: embedded   // the session still runs (close questions, liveSessions)
    property string iconName       // state icon: stopped, recording, activity, bell
    property string toolTip
    property color textColor: "transparent" // a logging tab's red title; transparent = theme
    property bool logActive: false          // tab menu: "Stop session log" / "Log output to file…"
    property bool recordingActive: false    // tab menu: "Stop recording" / "Record (asciinema)…"
    property bool multiexecExcluded: false  // tab menu: Exclude from MultiExec (the terminal reads it)
    property bool multiexecTarget: false    // MultiExec is on and this tab receives its input
    // The connection's environment id (session.yaml `environment:`), or "" — the tab
    // bar and the terminal frame ask the config for its colour.
    readonly property string environment: {
        return conn && typeof conn === "object" ? String(conn.environment || "").trim() : "";
    }
    property var session: null     // the session behind the views (survives moves)
    property var closeQuestion: null // fn() -> "" or a Yes/No question asked before closing

    // Set by PanesArea.
    property bool closingByUser: false // the user (or quitting) closed it — no disconnect notice

    // Emitted by PanesArea.
    signal closing                 // the tab closes for good: end its session; views go next
    signal logRequested            // the tab menu's log entry (the terminal starts or stops the log)
    signal recordingRequested      // the tab menu's record entry
}
