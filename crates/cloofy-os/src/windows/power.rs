//! Sleep and wake: the system calls back before it sleeps, and again once it woke up.
//!
//! The subscription is the process's and is never removed, so what the callback needs stays
//! alive for as long as the callback can run.

use std::{ffi::c_void, sync::Arc};

use windows::Win32::{
    Foundation::HANDLE,
    System::Power::{DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, RegisterSuspendResumeNotification},
    UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND},
};

use crate::power::Event;

type Callback = Arc<dyn Fn(Event) + Send + Sync>;

pub fn watch(on_event: Callback) -> bool {
    let context: *mut Callback = Box::into_raw(Box::new(on_event));
    // The system reads the parameters until the subscription ends, which is when the process
    // does, so neither they nor the context are ever freed.
    let parameters = Box::into_raw(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(callback),
        Context: context.cast(),
    }));
    // SAFETY: as above: what the handle points at outlives every call the system may make.
    match unsafe { RegisterSuspendResumeNotification(HANDLE(parameters.cast()), DEVICE_NOTIFY_CALLBACK) } {
        Ok(_) => true,
        Err(err) => {
            tracing::warn!("cannot watch for sleep and wake: {err}");
            false
        }
    }
}

/// `DEVICE_NOTIFY_CALLBACK_ROUTINE`: runs on a thread of the system's.
unsafe extern "system" fn callback(context: *const c_void, notification: u32, _setting: *const c_void) -> u32 {
    // SAFETY: `context` is the `Arc` that `watch` leaked, which outlives the subscription.
    let on_event = unsafe { &*context.cast::<Callback>() };
    match notification {
        PBT_APMSUSPEND => on_event(Event::Sleep),
        PBT_APMRESUMEAUTOMATIC => on_event(Event::Wake),
        _ => {}
    }
    // ERROR_SUCCESS: whatever it does next may carry on.
    0
}
