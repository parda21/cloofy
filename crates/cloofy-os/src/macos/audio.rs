//! Changes of the default output, through a CoreAudio property listener.

use std::{ffi::c_void, ptr::NonNull, sync::Arc};

use objc2_core_audio::{
    AudioObjectAddPropertyListener, AudioObjectID, AudioObjectPropertyAddress, AudioObjectRemovePropertyListener,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectSystemObject,
};

type Callback = Arc<dyn Fn() + Send + Sync>;

static DEFAULT_OUTPUT: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioHardwarePropertyDefaultOutputDevice,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
};

pub struct OutputWatch {
    /// Handed to CoreAudio as the listener's data; freed once the listener is removed.
    callback: NonNull<Callback>,
}

// SAFETY: the callback is `Send + Sync` and only ever called, never changed.
unsafe impl Send for OutputWatch {}

impl OutputWatch {
    pub fn new(on_change: Callback) -> Option<Self> {
        let callback = NonNull::from(Box::leak(Box::new(on_change)));
        // SAFETY: the address is a valid static, and the callback stays alive until the
        // listener is removed in `drop`.
        let status = unsafe {
            AudioObjectAddPropertyListener(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&DEFAULT_OUTPUT),
                Some(changed),
                callback.as_ptr().cast(),
            )
        };
        if status != 0 {
            tracing::warn!("cannot watch the default audio output: OSStatus {status}");
            // SAFETY: the listener wasn't added, so nothing else holds the pointer.
            drop(unsafe { Box::from_raw(callback.as_ptr()) });
            return None;
        }
        Some(Self { callback })
    }
}

impl Drop for OutputWatch {
    fn drop(&mut self) {
        // SAFETY: removes exactly the listener added in `new`; once it is gone, CoreAudio no
        // longer uses the callback and it can be freed.
        unsafe {
            AudioObjectRemovePropertyListener(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&DEFAULT_OUTPUT),
                Some(changed),
                self.callback.as_ptr().cast(),
            );
            drop(Box::from_raw(self.callback.as_ptr()));
        }
    }
}

unsafe extern "C-unwind" fn changed(
    _: AudioObjectID,
    _: u32,
    _: NonNull<AudioObjectPropertyAddress>,
    data: *mut c_void,
) -> i32 {
    // SAFETY: `data` is the callback of a registered listener, alive until it is removed.
    let callback = unsafe { &*data.cast::<Callback>() };
    callback();
    0
}
