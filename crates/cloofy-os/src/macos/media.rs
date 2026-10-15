//! Now Playing and remote commands through the MediaPlayer framework. Both are only touched
//! from the main thread.

use std::{cell::RefCell, ptr::NonNull, sync::Arc, time::Duration};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::{AllocAnyThread, rc::Retained, runtime::AnyObject};
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist,
    MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle, MPNowPlayingInfoCenter,
    MPNowPlayingInfoMediaType, MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyMediaType,
    MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommand, MPRemoteCommandCenter,
    MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};

use crate::media::{Command, State, Track};

pub struct Controls;

impl Controls {
    pub fn new(on_command: Arc<dyn Fn(Command) + Send + Sync>) -> Option<Self> {
        DispatchQueue::main().exec_async(move || register(&on_command));
        Some(Self)
    }

    pub fn update(&self, state: State) {
        DispatchQueue::main().exec_async(move || publish(&state));
    }
}

fn register(on_command: &Arc<dyn Fn(Command) + Send + Sync>) {
    // SAFETY: the shared command center lives for the whole process.
    let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
    // SAFETY: getters of the command center's fixed set of commands.
    let simple = unsafe {
        [
            (center.playCommand(), Command::Play),
            (center.pauseCommand(), Command::Pause),
            (center.togglePlayPauseCommand(), Command::Toggle),
            (center.nextTrackCommand(), Command::Next),
            (center.previousTrackCommand(), Command::Previous),
        ]
    };
    for (remote, command) in simple {
        let on_command = on_command.clone();
        handle(&remote, move |_| {
            on_command(command);
            MPRemoteCommandHandlerStatus::Success
        });
    }

    let on_command = on_command.clone();
    // SAFETY: as above.
    let position = unsafe { center.changePlaybackPositionCommand() };
    handle(&position, move |event| {
        let target = event
            .downcast_ref::<MPChangePlaybackPositionCommandEvent>()
            // SAFETY: a plain property getter.
            .and_then(|event| Duration::try_from_secs_f64(unsafe { event.positionTime() }).ok());
        match target {
            Some(position) => {
                on_command(Command::Seek(position));
                MPRemoteCommandHandlerStatus::Success
            }
            None => MPRemoteCommandHandlerStatus::CommandFailed,
        }
    });
}

fn handle(
    command: &MPRemoteCommand,
    handler: impl Fn(&MPRemoteCommandEvent) -> MPRemoteCommandHandlerStatus + 'static,
) {
    // SAFETY: the framework passes a valid event that outlives the call.
    let block = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| handler(unsafe { event.as_ref() }));
    // SAFETY: the command keeps the block alive for as long as it may call it.
    unsafe {
        command.setEnabled(true);
        command.addTargetWithHandler(&block);
    }
}

fn publish(state: &State) {
    // SAFETY: the default center lives for the whole process.
    let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
    let Some(track) = &state.track else {
        // SAFETY: clearing is allowed at any time.
        unsafe {
            center.setNowPlayingInfo(None);
            center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
        }
        return;
    };
    let info = now_playing_info(track, state);
    let playback = if state.playing { MPNowPlayingPlaybackState::Playing } else { MPNowPlayingPlaybackState::Paused };
    // SAFETY: the dictionary only holds the documented keys with values of their documented types.
    unsafe {
        center.setNowPlayingInfo(Some(&info));
        center.setPlaybackState(playback);
    }
}

fn now_playing_info(track: &Track, state: &State) -> Retained<NSDictionary<NSString, AnyObject>> {
    let title = NSString::from_str(&track.title);
    let artist = NSString::from_str(&track.artist);
    let album = NSString::from_str(&track.album);
    let duration = NSNumber::new_f64(track.duration.as_secs_f64());
    let elapsed = NSNumber::new_f64(state.position.as_secs_f64());
    let rate = NSNumber::new_f64(if state.playing { 1.0 } else { 0.0 });
    let media_type = NSNumber::new_usize(MPNowPlayingInfoMediaType::Audio.0);
    let artwork = track.artwork.as_ref().and_then(artwork);

    // SAFETY: the keys are immutable constants of the framework.
    let mut keys: Vec<&NSString> = unsafe {
        vec![
            MPMediaItemPropertyTitle,
            MPMediaItemPropertyArtist,
            MPMediaItemPropertyAlbumTitle,
            MPMediaItemPropertyPlaybackDuration,
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            MPNowPlayingInfoPropertyPlaybackRate,
            MPNowPlayingInfoPropertyMediaType,
        ]
    };
    let mut values: Vec<&AnyObject> = vec![
        title.as_ref(),
        artist.as_ref(),
        album.as_ref(),
        duration.as_ref(),
        elapsed.as_ref(),
        rate.as_ref(),
        media_type.as_ref(),
    ];
    if let Some(artwork) = &artwork {
        // SAFETY: as above.
        keys.push(unsafe { MPMediaItemPropertyArtwork });
        values.push(artwork.as_ref());
    }
    NSDictionary::from_slices(&keys, &values)
}

/// Artwork with the cover it was made from.
struct CachedArtwork {
    cover: Arc<[u8]>,
    artwork: Retained<MPMediaItemArtwork>,
}

thread_local! {
    /// The artwork shown last, so updates of the position don't decode the cover again.
    static ARTWORK: RefCell<Option<CachedArtwork>> = const { RefCell::new(None) };
}

fn artwork(cover: &Arc<[u8]>) -> Option<Retained<MPMediaItemArtwork>> {
    ARTWORK.with_borrow_mut(|cached| {
        if let Some(cached) = cached.as_ref()
            && Arc::ptr_eq(&cached.cover, cover)
        {
            return Some(cached.artwork.clone());
        }
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(cover))?;
        let size = image.size();
        // The system scales the image itself, whatever size it asks for.
        let handler = RcBlock::new(move |_: CGSize| NonNull::from(&*image));
        // SAFETY: the handler returns an image that it keeps alive itself.
        let artwork = unsafe {
            MPMediaItemArtwork::initWithBoundsSize_requestHandler(MPMediaItemArtwork::alloc(), size, &handler)
        };
        *cached = Some(CachedArtwork { cover: cover.clone(), artwork: artwork.clone() });
        Some(artwork)
    })
}
