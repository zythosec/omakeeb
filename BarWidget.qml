import QtQuick
import qs.Ui

// packaging/open decides: open omakeeb, or build it first in a terminal.
// Adding or updating the plugin never builds anything by itself.
BarWidget {
    id: root
    moduleName: "zythosec.omakeeb"

    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight

    function pluginPath() {
        var url = Qt.resolvedUrl(".").toString();
        if (url.startsWith("file://"))
            url = url.substring(7);
        if (url.endsWith("/"))
            url = url.slice(0, -1);
        return decodeURIComponent(url);
    }

    WidgetButton {
        id: button
        anchors.fill: parent
        bar: root.bar
        text: ""
        onPressed: function(mouseButton) {
            if (!root.bar || mouseButton !== Qt.LeftButton)
                return;
            root.bar.run("sh '" + root.pluginPath() + "/packaging/open'");
        }
    }
}
