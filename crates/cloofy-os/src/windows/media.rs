//! Now Playing and media keys through the System Media Transport Controls.
//!
//! WinRT objects belong to the apartment of the thread that made them, so the player and its
//! controls live on a thread of their own, which this module starts: `update` only tells that
//! thread what to show.

use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, Playback::MediaPlayer, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    },
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference},
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize},
    core::{HSTRING, Result as WinResult},
};

use crate::media::{Command, State, Track};

pub struct Controls {
    /// Handed to the thread that owns the controls; it shows nothing once that thread is gone.
    updates: mpsc::Sender<State>,
}

impl Controls {
    pub fn new(on_command: Arc<dyn Fn(Command) + Send + Sync>) -> Option<Self> {
        let (updates, states) = mpsc::channel();
        let (ready, created) = mpsc::sync_channel(0);
        thread::Builder::new()
            .name("cloofy-media".into())
            .spawn(move || {
                // SAFETY: nothing else runs on this thread, and WinRT is initialized before use.
                let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
                let Some(mut session) = Session::new(on_command) else {
                    let _ = ready.send(false);
                    return;
                };
                let _ = ready.send(true);
                while let Ok(state) = states.recv() {
                    session.show(state);
                }
            })
            .ok()?;
        // Says whether the thread got a session at all; the channel reads as closed if it didn't.
        created.recv().ok()?.then_some(Self { updates })
    }

    pub fn update(&self, state: State) {
        let _ = self.updates.send(state);
    }
}

struct Session {
    /// Kept for the session to exist: dropping the player takes it out of the system's controls.
    _player: MediaPlayer,
    controls: SystemMediaTransportControls,
    /// The cover the system reads, with the bytes it was made from, so that an update which
    /// changes only the position doesn't make another stream.
    artwork: Option<(Arc<[u8]>, RandomAccessStreamReference)>,
}

impl Session {
    fn new(on_command: Arc<dyn Fn(Command) + Send + Sync>) -> Option<Self> {
        let player = MediaPlayer::new().ok()?;
        // The player's command manager would drive its own source from the keys; cloofy drives
        // the playback, so it handles them instead.
        player.CommandManager().ok()?.SetIsEnabled(false).ok()?;
        let controls = player.SystemMediaTransportControls().ok()?;
        controls.SetIsPlayEnabled(true).ok()?;
        controls.SetIsPauseEnabled(true).ok()?;
        controls.SetIsNextEnabled(true).ok()?;
        controls.SetIsPreviousEnabled(true).ok()?;
        let handler = TypedEventHandler::<
            SystemMediaTransportControls,
            SystemMediaTransportControlsButtonPressedEventArgs,
        >::new(move |_, args| {
            if let Some(command) = args.as_ref().and_then(|args| args.Button().ok()).and_then(command) {
                on_command(command);
            }
            Ok(())
        });
        controls.ButtonPressed(&handler).ok()?;
        controls.SetIsEnabled(true).ok()?;
        Some(Self { _player: player, controls, artwork: None })
    }

    fn show(&mut self, state: State) {
        if let Err(err) = self.publish(&state) {
            tracing::warn!("the media controls didn't take an update: {err}");
        }
    }

    fn publish(&mut self, state: &State) -> WinResult<()> {
        let Some(track) = &state.track else {
            self.controls.DisplayUpdater()?.ClearAll()?;
            self.controls.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
            return Ok(());
        };
        let updater = self.controls.DisplayUpdater()?;
        // Also drops the cover of the track before, which the next one may not have.
        updater.ClearAll()?;
        updater.SetType(MediaPlaybackType::Music)?;
        let music = updater.MusicProperties()?;
        music.SetTitle(&HSTRING::from(track.title.as_str()))?;
        music.SetArtist(&HSTRING::from(track.artist.as_str()))?;
        music.SetAlbumTitle(&HSTRING::from(track.album.as_str()))?;
        match self.artwork(track) {
            Some(artwork) => updater.SetThumbnail(&artwork)?,
            None if track.artwork.is_some() => tracing::warn!("the media controls got no cover"),
            None => {}
        }
        updater.Update()?;

        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        timeline.SetStartTime(TimeSpan { Duration: 0 })?;
        timeline.SetEndTime(time_span(track.duration))?;
        timeline.SetMinSeekTime(TimeSpan { Duration: 0 })?;
        timeline.SetMaxSeekTime(time_span(track.duration))?;
        timeline.SetPosition(time_span(state.position))?;
        self.controls.UpdateTimelineProperties(&timeline)?;
        let status = if state.playing { MediaPlaybackStatus::Playing } else { MediaPlaybackStatus::Paused };
        self.controls.SetPlaybackStatus(status)?;
        Ok(())
    }

    /// The cover as a stream the system reads, made once per cover; `None` where it can't be.
    fn artwork(&mut self, track: &Track) -> Option<RandomAccessStreamReference> {
        let cover = track.artwork.as_ref()?;
        if let Some((shown, reference)) = &self.artwork
            && Arc::ptr_eq(shown, cover)
        {
            return Some(reference.clone());
        }
        let reference = stream(cover).inspect_err(|err| tracing::warn!("cannot read the cover: {err}")).ok()?;
        self.artwork = Some((cover.clone(), reference.clone()));
        Some(reference)
    }
}

/// The button a media key sends, as the command cloofy knows. Windows has no toggle key: the
/// system sends Play or Pause, by the state it shows.
fn command(button: SystemMediaTransportControlsButton) -> Option<Command> {
    match button {
        SystemMediaTransportControlsButton::Play => Some(Command::Play),
        SystemMediaTransportControlsButton::Pause => Some(Command::Pause),
        SystemMediaTransportControlsButton::Next => Some(Command::Next),
        SystemMediaTransportControlsButton::Previous => Some(Command::Previous),
        _ => None,
    }
}

/// A WinRT span: 100-nanosecond ticks.
fn time_span(duration: Duration) -> TimeSpan {
    TimeSpan { Duration: (duration.as_nanos() / 100) as i64 }
}

/// The encoded cover in a stream, at its start, as the display updater takes one.
fn stream(cover: &[u8]) -> WinResult<RandomAccessStreamReference> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(cover)?;
    writer.StoreAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    RandomAccessStreamReference::CreateFromStream(&stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_keys_become_commands() {
        assert_eq!(command(SystemMediaTransportControlsButton::Play), Some(Command::Play));
        assert_eq!(command(SystemMediaTransportControlsButton::Pause), Some(Command::Pause));
        assert_eq!(command(SystemMediaTransportControlsButton::Next), Some(Command::Next));
        assert_eq!(command(SystemMediaTransportControlsButton::Previous), Some(Command::Previous));
        assert_eq!(command(SystemMediaTransportControlsButton::Stop), None);
    }

    #[test]
    fn spans_count_ticks_of_a_hundred_nanoseconds() {
        assert_eq!(time_span(Duration::from_millis(1)).Duration, 10_000);
        assert_eq!(time_span(Duration::ZERO).Duration, 0);
    }
}
