import QtQuick
import QtQuick.Effects

// The title and the artist on one line, no wider than `maxWidth`. A line that doesn't fit fades
// out at the edge and, while `scrolling`, slides by now and then: a pause to read the start,
// one lap, and the start again.
Item {
  id: root

  property string title: ""
  // After the title and dimmer, such as the artist.
  property string detail: ""
  property color color: "white"
  property color detailColor: color
  property string fontFamily: ""
  property real fontSize: 12
  property real maxWidth: 220
  property bool scrolling: false

  readonly property bool overflows: line.implicitWidth > maxWidth
  readonly property real gap: fontSize * 3
  readonly property real lap: line.implicitWidth + gap
  readonly property real fadeWidth: Math.min(fontSize * 2, width / 4)

  implicitWidth: Math.min(maxWidth, line.implicitWidth)
  implicitHeight: line.implicitHeight
  clip: true

  component Line: Row {
    Text {
      textFormat: Text.PlainText
      text: root.title
      color: root.color
      font.family: root.fontFamily
      font.pixelSize: root.fontSize
    }
    Text {
      textFormat: Text.PlainText
      visible: root.detail !== ""
      text: " · " + root.detail
      color: root.detailColor
      font.family: root.fontFamily
      font.pixelSize: root.fontSize
    }
  }

  Item {
    anchors.fill: parent
    layer.enabled: root.overflows
    layer.smooth: true
    layer.effect: MultiEffect {
      maskEnabled: true
      maskSource: fade
      // With these, the mask's alpha ramps the line in smoothly rather than cutting it.
      maskThresholdMin: 0.5
      maskSpreadAtMin: 1
    }

    Item {
      id: strip
      height: root.height

      Line { id: line }
      // A second copy follows the first, so a lap ends where it began.
      Line { x: root.lap; visible: root.overflows }
    }
  }

  SequentialAnimation {
    id: slide
    running: root.scrolling && root.overflows && root.visible
    loops: Animation.Infinite
    PauseAnimation { duration: 2200 }
    NumberAnimation {
      target: strip
      property: "x"
      from: 0
      to: -root.lap
      // About 40 pixels a second, whatever the length.
      duration: root.lap * 25
      easing.type: Easing.InOutSine
    }
    onRunningChanged: if (!running) strip.x = 0
  }

  // Soft edges instead of a hard cut: the right one while the line overflows, the left one too
  // while it moves.
  Rectangle {
    id: fade
    anchors.fill: parent
    visible: false
    layer.enabled: true
    gradient: Gradient {
      orientation: Gradient.Horizontal
      GradientStop { position: 0; color: strip.x < 0 ? "transparent" : "black" }
      GradientStop { position: root.width > 0 ? root.fadeWidth / root.width : 0; color: "black" }
      GradientStop { position: root.width > 0 ? 1 - root.fadeWidth / root.width : 1; color: "black" }
      GradientStop { position: 1; color: "transparent" }
    }
  }
}
