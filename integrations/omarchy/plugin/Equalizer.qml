import QtQuick
import qs.Commons

// A few blocky bars that bounce while a song plays and rest when it doesn't. They step a few
// times a second rather than animate every frame, which suits cloofy's pixel look and keeps the
// bar from redrawing all the time.
Item {
  id: root

  property bool playing: false
  property color color: "white"
  property int bars: 3
  // Each bar moves between this many heights.
  property int steps: 4
  readonly property real barWidth: Math.max(2, Math.round(height / 4.5))
  readonly property real gap: Math.max(1, Math.round(barWidth * 0.6))
  readonly property real stepHeight: height / steps

  // Bar heights, in steps.
  property var levels: [2, 3, 1]

  implicitWidth: bars * barWidth + (bars - 1) * gap

  function bounce() {
    var next = []
    for (var i = 0; i < bars; i++) {
      // Each bar moves at most two steps at a time, so the motion reads as rhythm, not noise.
      var current = levels[i] || 1
      var target = 1 + Math.floor(Math.random() * steps)
      next.push(Math.max(1, Math.min(steps, current + Math.max(-2, Math.min(2, target - current)))))
    }
    levels = next
  }

  Repeater {
    model: root.bars

    Rectangle {
      required property int index

      x: index * (root.barWidth + root.gap)
      width: root.barWidth
      height: Math.max(1, Math.round((root.playing ? (root.levels[index] || 1) : 1) * root.stepHeight))
      anchors.bottom: parent.bottom
      color: root.color
      opacity: root.playing ? 1 : 0.55

      Behavior on opacity { NumberAnimation { duration: 160 } }
    }
  }

  Timer {
    running: root.playing && root.visible
    interval: 170
    repeat: true
    onTriggered: root.bounce()
  }
}
