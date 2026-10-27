//! Desktop notifications for new tracks while cloofy's window is in the background. They go
//! through the terminal's own escape sequence, so they need no permission of their own and a
//! click leads back to the terminal. Under Omarchy they go to the desktop directly instead,
//! which shows the cover with them.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use cloofy_core::player::CurrentTrack;

/// How long a track has to play before it gets announced, so skipping through doesn't flood.
const SETTLE: Duration = Duration::from_millis(1500);

/// The escape sequence a terminal understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    /// kitty's OSC 99: title and body, and a new one replaces the last.
    Kitty,
    /// rxvt's OSC 777, also spoken by Ghostty and WezTerm: title and body.
    Rxvt,
    /// iTerm2's OSC 9: a single line.
    ITerm,
}

pub struct Notifier {
    channel: Channel,
    /// The track playing, and since when.
    pending: Option<(String, Instant)>,
    /// The track last announced, or that played while cloofy was in front.
    announced: Option<String>,
}

enum Channel {
    Terminal {
        protocol: Protocol,
        /// Inside tmux, which passes the sequence on only when wrapped.
        tmux: bool,
    },
    /// The desktop's notification server, under Omarchy: its notifications show the cover, and
    /// a click on one finds cloofy's window by the app's name.
    #[cfg(target_os = "linux")]
    Desktop(cloofy_os::notifications::Notifications),
}

impl Notifier {
    /// `None` in terminals without notifications, unless Omarchy shows them.
    pub fn detect() -> Option<Self> {
        #[cfg(target_os = "linux")]
        if crate::omarchy::state_dir().is_some()
            && let Some(desktop) = cloofy_os::notifications::Notifications::new()
        {
            return Some(Self::new(Channel::Desktop(desktop)));
        }
        let var = |name| std::env::var(name).unwrap_or_default();
        let set = |name| std::env::var_os(name).is_some();
        let protocol = if set("KITTY_WINDOW_ID") || var("TERM").contains("kitty") {
            Protocol::Kitty
        } else if set("GHOSTTY_RESOURCES_DIR") || set("WEZTERM_PANE") {
            Protocol::Rxvt
        } else if set("ITERM_SESSION_ID") {
            Protocol::ITerm
        } else {
            match var("TERM_PROGRAM").as_str() {
                "ghostty" | "WezTerm" => Protocol::Rxvt,
                "iTerm.app" => Protocol::ITerm,
                _ => return None,
            }
        };
        Some(Self::new(Channel::Terminal { protocol, tmux: set("TMUX") }))
    }

    fn new(channel: Channel) -> Self {
        Self { channel, pending: None, announced: None }
    }

    /// Announces a track that has been playing for a moment while cloofy is in the background,
    /// with its cover where the desktop shows one. Returns the escape sequence to write, when
    /// the terminal is the one to tell. `track` is cloofy's own playback only.
    pub fn update(
        &mut self,
        track: Option<&CurrentTrack>,
        playing: bool,
        focused: bool,
        artwork: Option<&Arc<[u8]>>,
    ) -> Option<String> {
        let Some(track) = track.filter(|_| playing) else {
            self.pending = None;
            return None;
        };
        if self.announced.as_deref() == Some(track.uri.as_str()) {
            return None;
        }
        match &self.pending {
            Some((uri, since)) if *uri == track.uri => {
                if since.elapsed() < SETTLE {
                    return None;
                }
                self.pending = None;
                self.announced = Some(track.uri.clone());
                if focused {
                    return None;
                }
                self.announce(track, artwork)
            }
            _ => {
                self.pending = Some((track.uri.clone(), Instant::now()));
                None
            }
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    fn announce(&self, track: &CurrentTrack, artwork: Option<&Arc<[u8]>>) -> Option<String> {
        match &self.channel {
            Channel::Terminal { protocol, tmux } => Some(sequence(*protocol, *tmux, &track.name, &byline(track))),
            #[cfg(target_os = "linux")]
            Channel::Desktop(desktop) => {
                desktop.show(&track.name, &byline(track), artwork.cloned());
                None
            }
        }
    }
}

fn sequence(protocol: Protocol, tmux: bool, title: &str, body: &str) -> String {
    let (title, body) = (clean(title), clean(body));
    let sequence = match protocol {
        Protocol::Kitty => {
            let app = STANDARD.encode("cloofy");
            let (title, body) = (STANDARD.encode(title), STANDARD.encode(body));
            format!(
                "\x1b]99;i=cloofy:d=0:e=1:o=unfocused:f={app};{title}\x1b\\\x1b]99;i=cloofy:p=body:e=1;{body}\x1b\\"
            )
        }
        Protocol::Rxvt => format!("\x1b]777;notify;{title};{body}\x07"),
        Protocol::ITerm if body.is_empty() => format!("\x1b]9;{title}\x07"),
        Protocol::ITerm => format!("\x1b]9;{title} · {body}\x07"),
    };
    if tmux { passthrough(&sequence) } else { sequence }
}

/// Artists and album, leaving out what is missing: episodes have no album.
fn byline(track: &CurrentTrack) -> String {
    [track.artists.join(", "), track.album.clone()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Without control characters, which would end the sequence, or semicolons, which separate its
/// fields (and turn OSC 9 into other commands in some terminals).
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).map(|c| if c == ';' { ',' } else { c }).collect()
}

/// tmux passes a sequence on to the terminal when wrapped like this. It needs
/// `allow-passthrough all`: with `on`, tmux drops it for panes out of view, the usual case.
fn passthrough(sequence: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notifier(protocol: Protocol) -> Notifier {
        Notifier::new(Channel::Terminal { protocol, tmux: false })
    }

    fn track(uri: &str) -> CurrentTrack {
        CurrentTrack {
            uri: uri.into(),
            name: "One More Time".into(),
            artists: vec!["Daft Punk".into()],
            artist_uris: Vec::new(),
            album: "Discovery".into(),
            album_uri: None,
            duration_ms: 320_000,
            cover_url: None,
            cover_url_large: None,
        }
    }

    /// Plays `track` until it settles.
    fn settle(notifier: &mut Notifier, track: &CurrentTrack, focused: bool) -> Option<String> {
        assert_eq!(notifier.update(Some(track), true, focused, None), None, "not before it settled");
        notifier.pending.as_mut().unwrap().1 -= SETTLE;
        notifier.update(Some(track), true, focused, None)
    }

    #[test]
    fn announces_a_track_once_it_settled() {
        let mut notifier = notifier(Protocol::Rxvt);
        let one = track("spotify:track:1");
        let sequence = settle(&mut notifier, &one, false);
        assert_eq!(sequence.as_deref(), Some("\x1b]777;notify;One More Time;Daft Punk · Discovery\x07"));
        assert_eq!(notifier.update(Some(&one), true, false, None), None, "only once");
    }

    #[test]
    fn stays_quiet_in_front_and_for_that_track_afterwards() {
        let mut notifier = notifier(Protocol::Rxvt);
        let one = track("spotify:track:1");
        assert_eq!(settle(&mut notifier, &one, true), None);
        assert_eq!(notifier.update(Some(&one), true, false, None), None);
        assert!(settle(&mut notifier, &track("spotify:track:2"), false).is_some());
    }

    #[test]
    fn waits_while_paused() {
        let mut notifier = notifier(Protocol::Rxvt);
        let one = track("spotify:track:1");
        notifier.update(Some(&one), true, false, None);
        notifier.update(Some(&one), false, false, None);
        assert!(notifier.pending.is_none(), "pausing starts the wait over");
    }

    #[test]
    fn speaks_each_terminal_s_protocol() {
        let mut kitty = notifier(Protocol::Kitty);
        let sequence = settle(&mut kitty, &track("spotify:track:1"), false).unwrap();
        assert!(sequence.starts_with("\x1b]99;i=cloofy:d=0:e=1:o=unfocused:f=Y2xvb2Z5;T25lIE1vcmUgVGltZQ==\x1b\\"));
        let mut iterm = notifier(Protocol::ITerm);
        let sequence = settle(&mut iterm, &track("spotify:track:1"), false);
        assert_eq!(sequence.as_deref(), Some("\x1b]9;One More Time · Daft Punk · Discovery\x07"));
    }

    #[test]
    fn leaves_out_what_an_episode_lacks() {
        let episode = CurrentTrack { album: String::new(), ..track("spotify:episode:1") };
        let mut iterm = notifier(Protocol::ITerm);
        let sequence = settle(&mut iterm, &episode, false);
        assert_eq!(sequence.as_deref(), Some("\x1b]9;One More Time · Daft Punk\x07"));
        let nobody = CurrentTrack { artists: Vec::new(), ..episode };
        assert_eq!(byline(&nobody), "");
    }

    #[test]
    fn keeps_titles_from_breaking_the_sequence() {
        assert_eq!(clean("a;b\x07c\x1bd"), "a,bcd");
        let mut tmux = Notifier::new(Channel::Terminal { protocol: Protocol::Rxvt, tmux: true });
        let sequence = settle(&mut tmux, &track("spotify:track:1"), false).unwrap();
        assert!(sequence.starts_with("\x1bPtmux;\x1b\x1b]777;notify;"));
        assert!(sequence.ends_with("\x07\x1b\\"));
    }
}
