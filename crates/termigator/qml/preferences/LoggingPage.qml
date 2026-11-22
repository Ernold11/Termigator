import QtQuick
import org.kde.kirigami as Kirigami

PrefPage {
    title: qsTr("Logging")

    PrefCheck {
        key: "session_log_auto"
        text: qsTr("Log every remote session automatically")
    }
    PrefField {
        key: "session_log_template"
        placeholderText: "~/termigator-logs/{label}/{date}_{time}.log"
        Kirigami.FormData.label: qsTr("Log file:")
    }
    PrefHint {
        text: qsTr("Placeholders: {label} {host} {user} {date} {time}")
    }
    PrefCheck {
        key: "session_log_strip_ansi"
        text: qsTr("Plain text — strip colours and escape codes")
    }
    PrefCheck {
        key: "session_log_timestamps"
        text: qsTr("Timestamp every line")
    }
}
