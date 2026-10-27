use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Everything the user can trigger from anywhere: keys, mouse and the command palette all
/// end up here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Home,
    Library,
    PlayPause,
    Next,
    Previous,
    SeekBack,
    SeekForward,
    VolumeUp,
    VolumeDown,
    Shuffle,
    Repeat,
    Search,
    NowPlaying,
    /// The queue page: what plays next.
    ShowQueue,
    Like,
    Queue,
    AddToPlaylist,
    Devices,
    Dj,
    Crossfade,
    Menu,
    Palette,
    Help,
    Update,
    Quit,
}

impl Command {
    /// Commands offered in the palette, in display order.
    pub const PALETTE: [Command; 24] = [
        Command::Home,
        Command::Library,
        Command::PlayPause,
        Command::Next,
        Command::Previous,
        Command::NowPlaying,
        Command::ShowQueue,
        Command::Like,
        Command::Queue,
        Command::AddToPlaylist,
        Command::Menu,
        Command::Devices,
        Command::Dj,
        Command::Crossfade,
        Command::Search,
        Command::Shuffle,
        Command::Repeat,
        Command::VolumeUp,
        Command::VolumeDown,
        Command::SeekForward,
        Command::SeekBack,
        Command::Help,
        Command::Update,
        Command::Quit,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Command::Home => "Home",
            Command::Library => "Your Library",
            Command::PlayPause => "Play / Pause",
            Command::Next => "Next song",
            Command::Previous => "Previous song",
            Command::SeekBack => "Rewind 5 seconds",
            Command::SeekForward => "Forward 5 seconds",
            Command::VolumeUp => "Volume up",
            Command::VolumeDown => "Volume down",
            Command::Shuffle => "Shuffle on/off",
            Command::Repeat => "Toggle repeat",
            Command::Search => "Search",
            Command::NowPlaying => "Now playing & lyrics",
            Command::ShowQueue => "Queue",
            Command::Like => "Like / save",
            Command::Queue => "Add to queue",
            Command::AddToPlaylist => "Add to playlist…",
            Command::Devices => "Switch device…",
            Command::Dj => "Start DJ",
            Command::Crossfade => "Toggle crossfade",
            Command::Menu => "Actions for selection…",
            Command::Palette => "Command palette",
            Command::Help => "Keyboard shortcuts",
            Command::Update => "Check for updates",
            Command::Quit => "Quit cloofy",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Command::Home => "1",
            Command::Library => "3",
            Command::PlayPause => "space",
            Command::Next => "]",
            Command::Previous => "[",
            Command::SeekBack => "⇧←",
            Command::SeekForward => "⇧→",
            Command::VolumeUp => "+",
            Command::VolumeDown => "-",
            Command::Shuffle => "s",
            Command::Repeat => "r",
            Command::Search => "/",
            Command::NowPlaying => "z",
            Command::ShowQueue => "Q",
            Command::Like => "f",
            Command::Queue => "q",
            Command::AddToPlaylist => "a",
            Command::Devices => "d",
            Command::Dj => "D",
            Command::Crossfade => "X",
            Command::Menu => ".",
            Command::Palette => "^K",
            Command::Help => "?",
            Command::Update => "U",
            Command::Quit => "^C",
        }
    }

    /// Global key bindings. List navigation keys are handled per pane.
    pub fn from_key(key: &KeyEvent) -> Option<Command> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        Some(match key.code {
            KeyCode::Char('c' | 'q') if ctrl => Command::Quit,
            KeyCode::Char('k') if ctrl => Command::Palette,
            _ if ctrl => return None,
            KeyCode::Char(':') => Command::Palette,
            KeyCode::Char('?') => Command::Help,
            KeyCode::Char('/' | '2') => Command::Search,
            KeyCode::Char('1') => Command::Home,
            KeyCode::Char('3') => Command::Library,
            KeyCode::Char(' ') => Command::PlayPause,
            KeyCode::Char(']') => Command::Next,
            KeyCode::Char('[') => Command::Previous,
            KeyCode::Char('+' | '=') => Command::VolumeUp,
            KeyCode::Char('-') => Command::VolumeDown,
            KeyCode::Char('s') => Command::Shuffle,
            KeyCode::Char('r') => Command::Repeat,
            KeyCode::Char('z') => Command::NowPlaying,
            KeyCode::Char('f') => Command::Like,
            KeyCode::Char('q') => Command::Queue,
            KeyCode::Char('Q') => Command::ShowQueue,
            KeyCode::Char('a') => Command::AddToPlaylist,
            KeyCode::Char('d') => Command::Devices,
            KeyCode::Char('D') => Command::Dj,
            KeyCode::Char('X') => Command::Crossfade,
            KeyCode::Char('U') => Command::Update,
            KeyCode::Char('.') => Command::Menu,
            KeyCode::Char('H') => Command::SeekBack,
            KeyCode::Char('L') => Command::SeekForward,
            KeyCode::Left if shift => Command::SeekBack,
            KeyCode::Right if shift => Command::SeekForward,
            _ => return None,
        })
    }
}
