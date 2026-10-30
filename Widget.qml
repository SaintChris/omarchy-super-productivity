import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Shows Super Productivity's current task and today's time on it, read from
// the app's Local REST API (Settings > Misc > Local REST API). The app has to
// be running for the API to answer; with "Minimize to tray" on, closing its
// window keeps it running in the background.
BarWidget {
  id: root
  moduleName: "saintchris.super-productivity"

  readonly property int port: Number(setting("port", 3876))
  readonly property string tokenFile: expandHome(setting("tokenFile", "~/.config/superProductivity/local-rest-api-token"))
  readonly property string launchCommand: setting("launchCommand", "superproductivity")
  readonly property real maxLabelWidth: Number(setting("maxWidth", 220))

  readonly property color fg: bar ? bar.barForeground : Color.foreground
  readonly property color popupFg: bar ? bar.foreground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // "offline" (app not running), "noauth" (token missing or rejected),
  // "idle" (running, no task tracked) or "running".
  property string status: "offline"
  property var task: null
  property real spentToday: 0
  property var todayTasks: []
  property bool popupOpen: false
  property bool hovered: false
  // Only the newest poll may update the state, so a slow poll that returns
  // after a start/stop click can't flip the bar back to the old task.
  property int pollSeq: 0

  readonly property bool isRunning: status === "running" && task !== null
  readonly property string glyph: isRunning ? "󱎫" : "󰔛"
  readonly property string tooltipText: {
    if (status === "offline") return "Super Productivity isn't running. Click to start it."
    if (status === "noauth") return "Can't use the REST API: check the token file in this widget's settings."
    if (!isRunning) return "No task running. Click to pick one of today's tasks."
    return task.title + "\n" + formatSpent(spentToday) + " today. Click to stop, right-click for today's tasks."
  }

  function close() { popupOpen = false }

  function expandHome(path) {
    return path.indexOf("~/") === 0 ? Quickshell.env("HOME") + path.slice(1) : path
  }

  // Calendar date. If Super Productivity's day is set to start after midnight,
  // the time reads 0:00 between midnight and that hour.
  function todayStr() {
    return Qt.formatDate(new Date(), "yyyy-MM-dd")
  }

  function formatSpent(ms) {
    var minutes = Math.floor(ms / 60000)
    var rest = minutes % 60
    return Math.floor(minutes / 60) + ":" + (rest < 10 ? "0" : "") + rest
  }

  function api(method, path, body, done) {
    var token = String(tokenView.text() || "").trim()
    if (token === "") {
      if (done) done(false, null, 401)
      return
    }
    var xhr = new XMLHttpRequest()
    xhr.onreadystatechange = function() {
      if (xhr.readyState !== XMLHttpRequest.DONE) return
      var ok = false
      var data = null
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          var parsed = JSON.parse(xhr.responseText)
          ok = parsed.ok === true
          data = parsed.data
        } catch (e) {}
      }
      if (done) done(ok, data, xhr.status)
    }
    xhr.open(method, "http://127.0.0.1:" + port + path)
    xhr.setRequestHeader("Authorization", "Bearer " + token)
    if (body !== undefined) xhr.setRequestHeader("Content-Type", "application/json")
    xhr.send(body === undefined ? null : JSON.stringify(body))
  }

  function poll() {
    // The app rewrites the token file by rename, which a file watch can miss.
    if (status === "noauth") tokenView.reload()
    var seq = ++pollSeq
    api("GET", "/task-control/current", undefined, function(ok, data, httpStatus) {
      if (seq !== pollSeq) return
      if (!ok) {
        status = httpStatus === 401 || httpStatus === 403 ? "noauth" : "offline"
        task = null
        return
      }
      task = data
      spentToday = data && data.timeSpentOnDay ? (data.timeSpentOnDay[todayStr()] || 0) : 0
      status = data ? "running" : "idle"
    })
  }

  function loadTodayTasks() {
    api("GET", "/tasks?tagId=TODAY", undefined, function(ok, today) {
      if (!ok || !today) {
        todayTasks = []
        return
      }
      var parentIds = today.filter(function(t) { return t.subTaskIds && t.subTaskIds.length > 0 })
        .map(function(t) { return t.id })
      var leaves = today.filter(function(t) {
        return !t.isDone && !(t.subTaskIds && t.subTaskIds.length > 0)
      })
      if (parentIds.length === 0) {
        todayTasks = leaves
        return
      }
      // Time can't be tracked on a parent task, only on its subtasks, and the
      // Today filter returns the parent without them.
      api("GET", "/tasks", undefined, function(ok2, open) {
        var subtasks = ok2 && open ? open.filter(function(t) {
          return !t.isDone && parentIds.indexOf(t.parentId) !== -1
        }) : []
        todayTasks = leaves.concat(subtasks)
      })
    })
  }

  function startTask(taskId) {
    api("POST", "/task-control/current", { taskId: taskId }, function() { poll() })
    popupOpen = false
  }

  function stopTask() {
    api("POST", "/task-control/stop", {}, function() { poll() })
  }

  function launchApp(url) {
    if (!bar) return
    bar.run(launchCommand + (url ? " " + Util.shellQuote(url) : ""))
  }

  function togglePopup() {
    if (!popupOpen) loadTodayTasks()
    popupOpen = !popupOpen
  }

  onTooltipTextChanged: if (hovered && bar) bar.showTooltip(root, tooltipText)

  implicitWidth: row.implicitWidth + Style.space(14)
  implicitHeight: barSize

  FileView {
    id: tokenView
    path: root.tokenFile
    watchChanges: true
    printErrors: false
    // The token is a few bytes; reading it synchronously means the first poll
    // already has it.
    blockLoading: true
    onFileChanged: reload()
    onLoaded: root.poll()
  }

  // Poll every 3 s while the app answers; back off while it's closed or the
  // token is rejected.
  Timer {
    interval: root.status === "offline" || root.status === "noauth" ? 15000 : 3000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.poll()
  }

  Row {
    id: row
    anchors.centerIn: parent
    spacing: Style.space(6)

    Text {
      textFormat: Text.PlainText
      anchors.verticalCenter: parent.verticalCenter
      text: root.glyph
      color: root.isRunning ? root.fg : Qt.darker(root.fg, 1.5)
      font.family: root.fontFamily
      font.pixelSize: Style.font.body
    }

    Text {
      textFormat: Text.PlainText
      anchors.verticalCenter: parent.verticalCenter
      visible: root.isRunning && !root.vertical
      width: Math.min(root.maxLabelWidth, implicitWidth)
      elide: Text.ElideRight
      text: root.isRunning ? root.task.title : ""
      color: root.fg
      font.family: root.fontFamily
      font.pixelSize: Style.font.body
    }

    Text {
      textFormat: Text.PlainText
      anchors.verticalCenter: parent.verticalCenter
      visible: root.isRunning
      text: root.formatSpent(root.spentToday)
      color: Qt.darker(root.fg, 1.3)
      font.family: root.fontFamily
      font.pixelSize: Style.font.body
    }
  }

  MouseArea {
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton

    onClicked: function(mouse) {
      if (root.status === "offline") {
        root.launchApp("")
      } else if (mouse.button === Qt.MiddleButton) {
        root.launchApp("superproductivity://toggle-visibility")
      } else if (mouse.button === Qt.LeftButton && root.isRunning) {
        root.stopTask()
      } else if (root.status !== "noauth") {
        root.togglePopup()
      }
    }
    onEntered: {
      root.hovered = true
      if (root.bar) root.bar.showTooltip(root, root.tooltipText)
    }
    onExited: {
      root.hovered = false
      if (root.bar) root.bar.hideTooltip(root)
    }
  }

  PopupCard {
    id: popup
    anchorItem: root
    bar: root.bar
    owner: root
    open: root.popupOpen
    contentWidth: popup.fittedContentWidth(Style.space(320))
    contentHeight: popup.fittedContentHeight(column.implicitHeight)

    Column {
      id: column
      anchors.fill: parent
      spacing: Style.space(8)

      Text {
        textFormat: Text.PlainText
        width: parent.width
        text: root.isRunning ? root.task.title : "No task running"
        color: root.popupFg
        font.family: root.fontFamily
        font.pixelSize: Style.font.subtitle
        font.bold: true
        elide: Text.ElideRight
      }

      Text {
        textFormat: Text.PlainText
        visible: root.isRunning
        text: root.formatSpent(root.spentToday) + " today"
        color: Qt.darker(root.popupFg, 1.4)
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
      }

      PanelSeparator { foreground: root.popupFg }

      Text {
        textFormat: Text.PlainText
        visible: root.todayTasks.length === 0
        width: parent.width
        wrapMode: Text.Wrap
        text: "Nothing planned for today. Add tasks in the app to start them from here."
        color: Qt.darker(root.popupFg, 1.4)
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
      }

      // A long Today list scrolls instead of pushing the buttons off screen.
      Flickable {
        id: taskList
        visible: root.todayTasks.length > 0
        width: parent.width
        height: Math.min(taskColumn.implicitHeight, Style.space(360))
        contentHeight: taskColumn.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        Column {
          id: taskColumn
          width: taskList.width
          spacing: Style.space(4)

          Repeater {
            model: root.todayTasks

            BorderSurface {
              id: taskRow
              required property var modelData
              readonly property bool current: root.isRunning && root.task.id === modelData.id

              width: taskColumn.width
              height: taskInner.implicitHeight + Style.space(10)
              radius: Style.spacing.labelGap
              color: current || taskMouse.containsMouse ? Style.selectedFillFor(root.popupFg, Color.accent) : "transparent"
              borderSpec: current ? Border.controlSpec("normal", root.popupFg, Color.accent) : Border.none()

              Row {
                id: taskInner
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.leftMargin: taskRow.borderLeft + Style.space(8)
                anchors.rightMargin: taskRow.borderRight + Style.space(8)
                spacing: Style.space(8)

                Text {
                  textFormat: Text.PlainText
                  text: taskRow.current ? "󰏤" : "󰐊"
                  color: root.popupFg
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.body
                  width: Style.space(18)
                  horizontalAlignment: Text.AlignHCenter
                  anchors.verticalCenter: parent.verticalCenter
                }

                Text {
                  textFormat: Text.PlainText
                  text: taskRow.modelData.title
                  color: root.popupFg
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.bodySmall
                  font.bold: taskRow.current
                  elide: Text.ElideRight
                  width: parent.width - Style.space(26)
                  anchors.verticalCenter: parent.verticalCenter
                }
              }

              MouseArea {
                id: taskMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: taskRow.current ? root.stopTask() : root.startTask(taskRow.modelData.id)
              }
            }
          }
        }
      }

      PanelSeparator { foreground: root.popupFg }

      Row {
        spacing: Style.space(6)

        Button {
          visible: root.isRunning
          iconText: "󰏤"
          text: "Stop"
          foreground: root.popupFg
          onClicked: root.stopTask()
        }

        Button {
          iconText: "󰖯"
          text: "Open Super Productivity"
          foreground: root.popupFg
          onClicked: {
            root.popupOpen = false
            root.launchApp("superproductivity://toggle-visibility")
          }
        }
      }
    }
  }
}
