//! Short transitions, drawn over each finished frame.

use std::time::Duration;

use ratatui::{
    buffer::{Buffer, Cell},
    layout::Rect,
    style::Color,
};

use crate::theme::Rgb;

#[derive(Clone, Copy)]
enum Kind {
    /// Text fades in out of `from`.
    FadeIn { from: Color },
    /// Cells appear top down out of `from`, over a gradient `rows` high.
    SweepDown { from: Color, rows: u16 },
}

#[derive(Clone, Copy)]
pub struct Effect {
    kind: Kind,
    area: Rect,
    duration: Duration,
    elapsed: Duration,
}

impl Effect {
    pub fn fade_in(from: Color, ms: u64, area: Rect) -> Self {
        Self::new(Kind::FadeIn { from }, ms, area)
    }

    pub fn sweep_down(from: Color, rows: u16, ms: u64, area: Rect) -> Self {
        Self::new(Kind::SweepDown { from, rows }, ms, area)
    }

    fn new(kind: Kind, ms: u64, area: Rect) -> Self {
        Self { kind, area, duration: Duration::from_millis(ms), elapsed: Duration::ZERO }
    }

    fn done(&self) -> bool {
        self.elapsed >= self.duration
    }

    fn apply(&self, buffer: &mut Buffer, skip: fn(&Cell) -> bool) {
        let t = (self.elapsed.as_secs_f32() / self.duration.as_secs_f32()).min(1.0);
        let area = self.area.intersection(buffer.area);
        match self.kind {
            Kind::FadeIn { from } => {
                // Eases out: most of the way there early on.
                let left = (1.0 - t) * (1.0 - t);
                for pos in area.positions() {
                    let cell = &mut buffer[pos];
                    if !skip(cell) && matches!(cell.fg, Color::Rgb(..)) {
                        cell.fg = Rgb::of(cell.fg).mix(Rgb::of(from), left).into();
                    }
                }
            }
            Kind::SweepDown { from, rows } => {
                let rows = rows as f32;
                let start = area.y as f32 - rows + (area.height as f32 + rows) * quad_out(t);
                for pos in area.positions() {
                    let cell = &mut buffer[pos];
                    let shown = (1.0 - (pos.y as f32 - start) / rows).clamp(0.0, 1.0);
                    if shown >= 1.0 || skip(cell) {
                        continue;
                    }
                    if shown <= 0.0 {
                        (cell.fg, cell.bg) = (from, from);
                        continue;
                    }
                    let shown = circ_out(shown);
                    for color in [&mut cell.fg, &mut cell.bg] {
                        if matches!(color, Color::Rgb(..)) {
                            *color = Rgb::of(from).mix(Rgb::of(*color), shown).into();
                        }
                    }
                }
            }
        }
    }
}

/// Running effects, at most one per slot.
pub struct Effects<K> {
    running: Vec<(K, Effect)>,
}

impl<K> Default for Effects<K> {
    fn default() -> Self {
        Self { running: Vec::new() }
    }
}

impl<K: PartialEq> Effects<K> {
    /// Starts `effect`, in place of the one in `slot`.
    pub fn start(&mut self, slot: K, effect: Effect) {
        self.cancel(&slot);
        self.running.push((slot, effect));
    }

    pub fn cancel(&mut self, slot: &K) {
        self.running.retain(|(k, _)| k != slot);
    }

    pub fn is_running(&self) -> bool {
        !self.running.is_empty()
    }

    /// Moves each effect on by `elapsed` and draws it, leaving out cells `skip` picks.
    pub fn process(&mut self, elapsed: Duration, buffer: &mut Buffer, skip: fn(&Cell) -> bool) {
        for (_, effect) in &mut self.running {
            effect.elapsed += elapsed;
            if !effect.done() {
                effect.apply(buffer, skip);
            }
        }
        self.running.retain(|(_, effect)| !effect.done());
    }
}

fn quad_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

fn circ_out(t: f32) -> f32 {
    (1.0 - (t - 1.0) * (t - 1.0)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: Color = Color::Rgb(0, 0, 0);
    const TEXT: Color = Color::Rgb(200, 200, 200);

    fn buffer() -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 10));
        for cell in &mut buffer.content {
            (cell.fg, cell.bg) = (TEXT, TEXT);
        }
        buffer
    }

    fn never(_: &Cell) -> bool {
        false
    }

    #[test]
    fn fade_starts_at_its_color_and_ends_on_the_frame() {
        let mut effects = Effects::default();
        effects.start((), Effect::fade_in(FROM, 100, Rect::new(0, 0, 4, 10)));
        let mut frame = buffer();
        effects.process(Duration::ZERO, &mut frame, never);
        assert_eq!(frame[(0, 0)].fg, FROM);
        let mut frame = buffer();
        effects.process(Duration::from_millis(50), &mut frame, never);
        assert!(!matches!(frame[(0, 0)].fg, c if c == FROM || c == TEXT));
        effects.process(Duration::from_millis(50), &mut buffer(), never);
        assert!(!effects.is_running());
    }

    #[test]
    fn sweep_shows_the_top_first() {
        let mut effects = Effects::default();
        effects.start((), Effect::sweep_down(FROM, 2, 100, Rect::new(0, 0, 4, 10)));
        let mut frame = buffer();
        effects.process(Duration::from_millis(30), &mut frame, never);
        assert_eq!(frame[(0, 0)].bg, TEXT);
        assert_eq!(frame[(0, 9)].bg, FROM);
    }

    #[test]
    fn a_new_effect_replaces_the_one_in_its_slot() {
        let mut effects = Effects::default();
        effects.start(1, Effect::fade_in(FROM, 100, Rect::new(0, 0, 4, 10)));
        effects.start(1, Effect::fade_in(FROM, 100, Rect::new(0, 0, 4, 10)));
        effects.start(2, Effect::fade_in(FROM, 100, Rect::new(0, 0, 4, 10)));
        assert_eq!(effects.running.len(), 2);
        effects.cancel(&1);
        assert_eq!(effects.running.len(), 1);
    }
}
