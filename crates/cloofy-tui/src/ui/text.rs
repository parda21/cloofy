use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Three thin bars drawn with braille dots, bouncing out of phase.
const EQUALIZER: [&str; 8] = ["⡄⡇⡆", "⡆⡄⡇", "⡇⡆⡄", "⡆⡇⡆", "⡄⡆⡇", "⡇⡄⡆", "⡆⡇⡄", "⡄⡆⡆"];

/// Cuts `text` to `width` columns with an ellipsis, then pads it to exactly `width`.
pub fn fit(text: &str, width: usize) -> String {
    let mut out = truncate(text, width);
    let pad = width.saturating_sub(out.width());
    out.extend(std::iter::repeat_n(' ', pad));
    out
}

pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

pub fn right(text: &str, width: usize) -> String {
    format!("{text:>width$}")
}

/// `icon  label`: Nerd Font icons spill into the cell after them, so one space would vanish.
pub fn icon_label(icon: &str, label: &str) -> String {
    format!("{icon}  {label}")
}

/// `3:58`
pub fn duration(ms: u32) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// `2 hr 51 min`, or `43 min` below an hour.
pub fn total_duration(ms: u64) -> String {
    let minutes = ms / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} hr {m} min"),
    }
}

/// `28118277` → `28,118,277`
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn spinner(frame: u64) -> &'static str {
    SPINNER[frame as usize % SPINNER.len()]
}

pub fn equalizer(frame: u64) -> &'static str {
    EQUALIZER[frame as usize % EQUALIZER.len()]
}
