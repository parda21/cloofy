import QtQuick
import QtQuick.Effects
import Quickshell
import Quickshell.Services.Mpris
import qs.Commons
import qs.Commons as Commons
import qs.Ui

// cloofy in the bar: what plays, with a card for the cover, the controls and the volume.
//
// It talks to cloofy over MPRIS, the way the media keys do, and tells Spotify from YouTube Music
// by the link cloofy gives each track. Colors come from the theme, so they follow it live.
//
// Left click opens the card, middle click plays or pauses, right click brings cloofy to the
// front, and scrolling skips, like Omarchy's own media widget.
BarWidget {
  id: root
  moduleName: "cloofy.nowplaying"

  readonly property int maxWidth: Math.max(80, Number(setting("maxWidth", 220)) || 220)
  readonly property string overflow: String(setting("overflow", "Scroll"))
  readonly property bool showArtist: setting("showArtist", true) !== false
  readonly property bool showEqualizer: setting("equalizer", true) !== false
  readonly property bool launcher: setting("whenClosed", "Hide") === "Show a launcher"

  // cloofy's player; a second cloofy adds its process ID to the name. One that plays wins.
  readonly property var player: {
    var players = Mpris.players ? Mpris.players.values : []
    var found = null
    for (var i = 0; i < players.length; i++) {
      var name = String(players[i].dbusName || "")
      if (name !== "org.mpris.MediaPlayer2.cloofy" && name.indexOf("org.mpris.MediaPlayer2.cloofy.") !== 0) continue
      if (players[i].isPlaying) return players[i]
      if (!found) found = players[i]
    }
    return found
  }
  readonly property bool running: player !== null
  readonly property string title: running ? (player.trackTitle || "") : ""
  readonly property bool hasTrack: title !== ""
  readonly property bool playing: hasTrack && player.isPlaying
  readonly property string artist: running ? (player.trackArtist || "") : ""
  readonly property string album: running ? (player.trackAlbum || "") : ""
  readonly property string artUrl: running ? (player.trackArtUrl || "") : ""
  readonly property string link: running && player.metadata ? String(player.metadata["xesam:url"] || "") : ""
  readonly property string service: link.indexOf("music.youtube.com") !== -1 ? "ytmusic"
    : (link.indexOf("spotify.com") !== -1 ? "spotify" : "")
  readonly property string serviceGlyph: service === "ytmusic" ? "󰗃" : (service === "spotify" ? "󰓇" : "󰝚")
  readonly property string serviceName: service === "ytmusic" ? "YouTube Music" : (service === "spotify" ? "Spotify" : "")

  readonly property color foreground: bar ? bar.barForeground : Commons.Color.foreground
  readonly property color accent: Commons.Color.accent
  readonly property color popupText: Commons.Color.popups.text

  property bool popupOpen: false
  property bool hovered: false
  property real wheelRemainder: 0

  // The shape `omarchy-shell shell toggle cloofy.nowplaying` looks for.
  readonly property bool opened: popupOpen
  function open() { if (running) popupOpen = true }
  function close() { popupOpen = false }

  // The bar's open-card dot spans what the widget draws, not its padded slot.
  readonly property real openPanelIndicatorWidth: content.implicitWidth

  function showCloofy() {
    close()
    if (running && player.canRaise) player.raise()
    else if (bar) bar.run("omarchy-launch-or-focus-tui cloofy")
  }

  function playPause() {
    if (running && player.canTogglePlaying) player.togglePlaying()
  }

  function skip(steps) {
    if (!running) return
    if (steps > 0 && player.canGoNext) player.next()
    else if (steps < 0 && player.canGoPrevious) player.previous()
  }

  function clock(seconds) {
    var total = Math.max(0, Math.floor(seconds || 0))
    var minutes = Math.floor(total / 60)
    var rest = total % 60
    if (minutes >= 60) {
      var hours = Math.floor(minutes / 60)
      minutes = minutes % 60
      return hours + ":" + (minutes < 10 ? "0" : "") + minutes + ":" + (rest < 10 ? "0" : "") + rest
    }
    return minutes + ":" + (rest < 10 ? "0" : "") + rest
  }

  visible: running || launcher
  implicitHeight: barSize
  implicitWidth: visible ? (vertical ? barSize : content.implicitWidth + Style.space(14)) : 0
  onRunningChanged: if (!running) close()

  Row {
    id: content
    anchors.centerIn: parent
    spacing: Style.space(6)
    // Full while playing, a little dimmer while paused, faint while nothing is loaded.
    opacity: root.playing ? 1 : (root.hasTrack ? 0.75 : 0.5)

    Behavior on opacity { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }

    Text {
      id: glyph
      anchors.verticalCenter: parent.verticalCenter
      textFormat: Text.PlainText
      text: root.serviceGlyph
      color: root.foreground
      font.family: root.bar ? root.bar.fontFamily : Style.font.family
      font.pixelSize: Style.font.body
      renderType: Text.NativeRendering
    }

    Equalizer {
      anchors.verticalCenter: parent.verticalCenter
      visible: root.showEqualizer && root.hasTrack && !root.vertical
      height: Math.round(Style.font.body * 0.85)
      playing: root.playing
      color: root.accent
    }

    Marquee {
      anchors.verticalCenter: parent.verticalCenter
      visible: root.hasTrack && !root.vertical
      title: root.title
      detail: root.showArtist ? root.artist : ""
      color: root.foreground
      detailColor: Util.alpha(root.foreground, 0.6)
      fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
      fontSize: Style.font.body
      maxWidth: root.maxWidth
      scrolling: !root.popupOpen && (root.overflow === "Scroll" ? root.playing
        : root.overflow === "Scroll on hover" && root.hovered)
    }
  }

  MouseArea {
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton

    onClicked: function(mouse) {
      if (root.bar) root.bar.hideTooltip(root)
      if (!root.running || mouse.button === Qt.RightButton) root.showCloofy()
      else if (mouse.button === Qt.MiddleButton) root.playPause()
      else root.popupOpen = !root.popupOpen
    }
    onWheel: function(wheel) {
      var result = Util.wheelSteps(root.wheelRemainder, wheel.angleDelta.y)
      root.wheelRemainder = result.remainder
      // Up goes back, down goes on, like Omarchy's media widget.
      if (result.steps !== 0) root.skip(-result.steps)
    }
    onEntered: {
      root.hovered = true
      if (!root.bar) return
      if (!root.running) root.bar.showTooltip(root, "Open cloofy")
      else if (root.hasTrack && !root.popupOpen) root.bar.showTooltip(root, root.title + (root.artist ? " — " + root.artist : ""))
    }
    onExited: {
      root.hovered = false
      if (root.bar) root.bar.hideTooltip(root)
    }
  }

  // Follows the song while the card is open; MPRIS doesn't tick the position by itself.
  Timer {
    running: root.popupOpen && root.playing
    interval: 500
    repeat: true
    triggeredOnStart: true
    onTriggered: if (root.player) root.player.positionChanged()
  }

  PopupCard {
    id: popup
    anchorItem: root
    bar: root.bar
    owner: root
    open: root.popupOpen
    contentWidth: popup.fittedContentWidth(Style.space(300))
    contentHeight: popup.fittedContentHeight(card.implicitHeight)

    Column {
      id: card
      width: parent.width
      spacing: Style.space(12)

      // The cover, as large as the card is wide, with the service in a corner.
      Item {
        id: cover
        width: parent.width
        height: width

        readonly property real radius: Math.min(Style.cornerRadius, width / 8)

        Rectangle {
          id: coverMask
          anchors.fill: parent
          radius: cover.radius
          visible: false
          layer.enabled: true
        }

        Item {
          anchors.fill: parent
          layer.enabled: cover.radius > 0
          layer.effect: MultiEffect {
            maskEnabled: true
            maskSource: coverMask
          }

          Rectangle {
            anchors.fill: parent
            color: Style.normalFillFor(root.popupText, root.accent)

            Text {
              anchors.centerIn: parent
              visible: art.status !== Image.Ready
              textFormat: Text.PlainText
              text: "󰝚"
              color: Util.alpha(root.popupText, 0.35)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: cover.width / 4
            }
          }

          Image {
            id: art
            anchors.fill: parent
            source: root.artUrl
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            smooth: true
            mipmap: true
            sourceSize.width: width * 2
            sourceSize.height: height * 2
            opacity: status === Image.Ready ? 1 : 0

            Behavior on opacity { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
          }
        }

        BorderSurface {
          visible: root.serviceName !== ""
          anchors.left: parent.left
          anchors.bottom: parent.bottom
          anchors.margins: Style.space(8)
          width: badge.implicitWidth + Style.space(14)
          height: badge.implicitHeight + Style.space(8)
          radius: Math.min(Style.cornerRadius, height / 2)
          color: Util.alpha(Commons.Color.popups.background, 0.82)
          borderSpec: Border.controlSpec("normal", root.popupText, root.accent)

          Row {
            id: badge
            anchors.centerIn: parent
            spacing: Style.space(5)

            Text {
              anchors.verticalCenter: parent.verticalCenter
              textFormat: Text.PlainText
              text: root.serviceGlyph
              color: root.accent
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.body
            }

            Text {
              anchors.verticalCenter: parent.verticalCenter
              textFormat: Text.PlainText
              text: root.serviceName
              color: root.popupText
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
            }
          }
        }
      }

      Column {
        width: parent.width
        spacing: Style.space(3)

        Text {
          width: parent.width
          textFormat: Text.PlainText
          text: root.hasTrack ? root.title : (root.running ? "Nothing playing" : "cloofy isn't open")
          color: root.popupText
          font.family: root.bar ? root.bar.fontFamily : Style.font.family
          font.pixelSize: Style.font.heading
          font.bold: true
          wrapMode: Text.Wrap
          maximumLineCount: 2
          elide: Text.ElideRight
        }

        Text {
          width: parent.width
          visible: text !== ""
          textFormat: Text.PlainText
          text: root.hasTrack ? root.artist : "Pick something in cloofy and it shows up here."
          color: Util.alpha(root.popupText, 0.75)
          font.family: root.bar ? root.bar.fontFamily : Style.font.family
          font.pixelSize: Style.font.body
          wrapMode: root.hasTrack ? Text.NoWrap : Text.Wrap
          elide: Text.ElideRight
        }

        Text {
          width: parent.width
          visible: root.hasTrack && text !== ""
          textFormat: Text.PlainText
          text: root.album
          color: Util.alpha(root.popupText, 0.5)
          font.family: root.bar ? root.bar.fontFamily : Style.font.family
          font.pixelSize: Style.font.bodySmall
          elide: Text.ElideRight
        }
      }

      // Where the song is; click or drag to go elsewhere in it.
      Column {
        width: parent.width
        spacing: 0
        visible: root.hasTrack && root.player.lengthSupported && root.player.length > 0

        PanelSlider {
          id: progress
          width: parent.width
          bar: root.bar
          minimum: 0
          maximum: root.running ? Math.max(1, root.player.length) : 1
          value: root.running ? root.player.position : 0
          step: 5
          enabled: root.running && root.player.canSeek
          fillColor: root.accent
          knobColor: root.popupText
          onReleased: function(value) { if (root.player && root.player.canSeek) root.player.position = value }
        }

        Item {
          width: parent.width
          height: elapsed.implicitHeight

          Text {
            id: elapsed
            anchors.left: parent.left
            textFormat: Text.PlainText
            text: root.clock(progress.liveValue)
            color: Util.alpha(root.popupText, 0.6)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
          }

          Text {
            anchors.right: parent.right
            textFormat: Text.PlainText
            text: "-" + root.clock(progress.maximum - progress.liveValue)
            color: Util.alpha(root.popupText, 0.6)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
          }
        }
      }

      Row {
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: Style.space(10)

        Button {
          anchors.verticalCenter: parent.verticalCenter
          iconText: "󰒮"
          foreground: root.popupText
          iconSize: Style.font.iconLarge
          enabled: root.running && root.player.canGoPrevious
          opacity: enabled ? 1 : 0.4
          onClicked: root.skip(-1)
        }

        Button {
          anchors.verticalCenter: parent.verticalCenter
          iconText: root.playing ? "󰏤" : "󰐊"
          foreground: root.popupText
          bordered: true
          horizontalPadding: Style.space(18)
          verticalPadding: Style.space(9)
          iconSize: Style.font.display
          enabled: root.running && root.player.canTogglePlaying
          opacity: enabled ? 1 : 0.4
          onClicked: root.playPause()
        }

        Button {
          anchors.verticalCenter: parent.verticalCenter
          iconText: "󰒭"
          foreground: root.popupText
          iconSize: Style.font.iconLarge
          enabled: root.running && root.player.canGoNext
          opacity: enabled ? 1 : 0.4
          onClicked: root.skip(1)
        }
      }

      Row {
        width: parent.width
        spacing: Style.space(8)
        visible: root.running && root.player.volumeSupported

        Text {
          id: speaker
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(18)
          textFormat: Text.PlainText
          text: volume.liveValue <= 0.001 ? "󰝟" : (volume.liveValue < 0.34 ? "󰕿" : (volume.liveValue < 0.67 ? "󰖀" : "󰕾"))
          color: root.popupText
          font.family: root.bar ? root.bar.fontFamily : Style.font.family
          font.pixelSize: Style.font.icon
        }

        PanelSlider {
          id: volume
          anchors.verticalCenter: parent.verticalCenter
          width: parent.width - speaker.width - percent.width - parent.spacing * 2
          bar: root.bar
          minimum: 0
          maximum: 1
          step: 0.05
          value: root.running ? root.player.volume : 0
          knobColor: root.popupText
          fillColor: root.popupText
          onMoved: function(value) { if (root.player) root.player.volume = value }
        }

        Text {
          id: percent
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(34)
          horizontalAlignment: Text.AlignRight
          textFormat: Text.PlainText
          text: Math.round(volume.liveValue * 100) + "%"
          color: Util.alpha(root.popupText, 0.6)
          font.family: root.bar ? root.bar.fontFamily : Style.font.family
          font.pixelSize: Style.font.caption
        }
      }

      PanelSeparator {
        foreground: root.popupText
      }

      Button {
        width: parent.width
        iconText: "󰆍"
        text: "Open cloofy"
        foreground: root.popupText
        bordered: true
        onClicked: root.showCloofy()
      }
    }
  }
}
