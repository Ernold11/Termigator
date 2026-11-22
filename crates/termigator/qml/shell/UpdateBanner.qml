import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// "Termigator X is available": start() asks GitHub for the latest release (at most once a day,
// window.check_updates; the decisions are in termigator-core update_check) and shows this banner
// above the terminals when it is newer. Nothing is downloaded; failures stay silent.
Kirigami.InlineMessage {
    id: banner

    // {"version", "url"} of the newer release, else empty.
    property string version
    property string url

    function start() {
        const target = appController.updateCheckUrl();
        if (target === "")
            return;
        const xhr = new XMLHttpRequest();
        xhr.onreadystatechange = () => {
            if (xhr.readyState !== XMLHttpRequest.DONE)
                return;
            abortTimer.stop();
            // The status is not looked at: an error body (404 while the repository is private,
            // a captive portal page) is simply not a newer release.
            const found = appController.updateCheckResult(xhr.responseText);
            if (found === "")
                return;
            const r = JSON.parse(found);
            banner.url = r.url;
            banner.version = r.version;
        };
        xhr.open("GET", target);
        xhr.setRequestHeader("Accept", "application/vnd.github+json");
        abortTimer.xhr = xhr;
        abortTimer.start();
        xhr.send();
    }

    visible: version !== ""
    type: Kirigami.MessageType.Information
    showCloseButton: true
    text: qsTr("Termigator %1 is available (you have %2).").arg(version).arg(appController.version)
    Accessible.role: Accessible.AlertMessage
    Accessible.name: text
    actions: [
        Kirigami.Action {
            text: qsTr("Open release page")
            icon.name: "internet-services"
            onTriggered: Qt.openUrlExternally(banner.url)
        },
        Kirigami.Action {
            text: qsTr("Skip this version")
            onTriggered: {
                appController.skipUpdate(banner.version);
                banner.version = "";
            }
        }
    ]
    // The close button hides it until the next start.
    onVisibleChanged: {
        if (!visible)
            version = "";
    }

    Timer {
        id: abortTimer

        property var xhr: null

        interval: 20000
        onTriggered: {
            if (xhr)
                xhr.abort();
        }
    }
}
