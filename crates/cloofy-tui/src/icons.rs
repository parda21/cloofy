//! Icons: Nerd Font glyphs where the terminal ships them, plain symbols otherwise.
//!
//! Symbols missing from the user's font get pulled from fallback fonts and stretched to the
//! cell, which looks off. Nerd Font glyphs are made for terminal cells, and Ghostty, kitty and
//! WezTerm bundle them.

use std::sync::atomic::{AtomicBool, Ordering};

pub struct Icons {
    pub home: &'static str,
    pub search: &'static str,
    pub library: &'static str,
    pub shuffle: &'static str,
    pub previous: &'static str,
    pub next: &'static str,
    pub play: &'static str,
    pub pause: &'static str,
    pub repeat: &'static str,
    pub repeat_one: &'static str,
    pub volume: &'static str,
    /// In the library (Spotify's check circle).
    pub saved: &'static str,
    /// Add to the library.
    pub save: &'static str,
    pub note: &'static str,
    pub playlist: &'static str,
    pub album: &'static str,
    pub artist: &'static str,
    pub podcast: &'static str,
    pub audiobook: &'static str,
    pub queue: &'static str,
    /// Take out of the queue.
    pub remove: &'static str,
    pub open: &'static str,
    pub link: &'static str,
    pub more: &'static str,
    /// Open the library as a full page.
    pub expand: &'static str,
    /// Page a shelf sideways.
    pub chevron_left: &'static str,
    pub chevron_right: &'static str,
    pub speaker: &'static str,
    pub computer: &'static str,
    pub phone: &'static str,
    pub tablet: &'static str,
    pub tv: &'static str,
    pub cast: &'static str,
    /// Moves DJ on to different picks.
    pub dj: &'static str,
    /// A newer cloofy to install.
    pub update: &'static str,
    /// Start the installed update.
    pub restart: &'static str,
    /// The music services, in the switch between them; none in plain text.
    pub spotify: &'static str,
    pub youtube: &'static str,
}

const NERD: Icons = Icons {
    home: "\u{F02DC}",
    search: "\u{F0349}",
    library: "\u{F0331}",
    shuffle: "\u{F049D}",
    previous: "\u{F04AE}",
    next: "\u{F04AD}",
    play: "\u{F040A}",
    pause: "\u{F03E4}",
    repeat: "\u{F0456}",
    repeat_one: "\u{F0458}",
    volume: "\u{F057E}",
    saved: "\u{F05E0}",
    save: "\u{F0419}",
    note: "\u{F0387}",
    playlist: "\u{F0CB8}",
    album: "\u{F0025}",
    artist: "\u{F0803}",
    podcast: "\u{F0994}",
    audiobook: "\u{F14F7}",
    queue: "\u{F0411}",
    remove: "\u{F0377}",
    open: "\u{F03CC}",
    link: "\u{F0339}",
    more: "\u{F01D8}",
    expand: "\u{F0616}",
    chevron_left: "\u{F0141}",
    chevron_right: "\u{F0142}",
    speaker: "\u{F04C3}",
    computer: "\u{F0322}",
    phone: "\u{F011C}",
    tablet: "\u{F04F6}",
    tv: "\u{F0502}",
    cast: "\u{F0118}",
    dj: "\u{F147D}",
    update: "\u{F0CE1}",
    restart: "\u{F0709}",
    spotify: "\u{F04C7}",
    youtube: "\u{F05C3}",
};

const TEXT: Icons = Icons {
    home: "⌂",
    search: "⌕",
    library: "≡",
    shuffle: "⤨",
    previous: "◂◂",
    next: "▸▸",
    play: "▶",
    pause: "❚❚",
    repeat: "↻",
    repeat_one: "↻1",
    volume: "♪",
    saved: "✓",
    save: "+",
    note: "♪",
    playlist: "♫",
    album: "◫",
    artist: "◉",
    podcast: "◎",
    audiobook: "❡",
    queue: "≡",
    remove: "−",
    open: "↗",
    link: "⧉",
    more: "···",
    expand: "⤢",
    chevron_left: "‹",
    chevron_right: "›",
    speaker: "◉",
    computer: "▣",
    phone: "▯",
    tablet: "▭",
    tv: "▭",
    cast: "◎",
    dj: "DJ",
    update: "↑",
    restart: "↻",
    spotify: "",
    youtube: "",
};

static NERD_ICONS: AtomicBool = AtomicBool::new(false);

/// Picks the icon set: `"nerd"` or `"text"` from the settings, else by terminal. Called again
/// when the setting changes; the next frame draws with the new set.
pub fn init(setting: Option<&str>) {
    let nerd = match setting {
        Some("nerd") => true,
        Some("text") => false,
        _ => terminal_bundles_nerd_font(),
    };
    NERD_ICONS.store(nerd, Ordering::Relaxed);
}

pub fn icons() -> &'static Icons {
    if NERD_ICONS.load(Ordering::Relaxed) { &NERD } else { &TEXT }
}

fn terminal_bundles_nerd_font() -> bool {
    let var = |name| std::env::var(name).unwrap_or_default();
    matches!(var("TERM_PROGRAM").as_str(), "ghostty" | "WezTerm")
        || var("TERM").contains("kitty")
        || std::env::var_os("KITTY_WINDOW_ID").is_some()
}

/// The icon of a music service, empty without Nerd Font glyphs.
pub fn service(service: cloofy_core::service::Service) -> &'static str {
    match service {
        cloofy_core::service::Service::Spotify => icons().spotify,
        cloofy_core::service::Service::YouTubeMusic => icons().youtube,
    }
}

/// The icon for a Spotify Connect device type.
pub fn device(kind: &str) -> &'static str {
    let icons = icons();
    match kind {
        "Smartphone" => icons.phone,
        "Tablet" => icons.tablet,
        "TV" | "STB" | "CastVideo" => icons.tv,
        "CastAudio" => icons.cast,
        "Speaker" | "AVR" | "Automobile" => icons.speaker,
        _ => icons.computer,
    }
}
