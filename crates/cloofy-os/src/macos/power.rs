//! Sleep and wake through IOKit's power notifications, which reach processes without a
//! Cocoa application too.

use std::{ffi::c_void, ptr, sync::Arc, time::Duration};

use dispatch2::{DispatchQueue, DispatchTime};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopCommonModes};
use objc2_io_kit::{
    IOAllowPowerChange, IONotificationPort, IONotificationPortRef, IORegisterForSystemPower, io_connect_t, io_object_t,
    io_service_t, kIOMessageCanSystemSleep, kIOMessageSystemHasPoweredOn, kIOMessageSystemWillSleep,
};

use crate::power::Event;

/// How long the system waits after announcing sleep, so playback can pause first.
const SLEEP_GRACE: Duration = Duration::from_secs(1);

type Callback = Arc<dyn Fn(Event) + Send + Sync>;

/// Lives for the rest of the process, as the listener's data.
struct Watch {
    on_event: Callback,
    root_port: io_connect_t,
}

pub fn watch(on_event: Callback) -> bool {
    let watch = Box::into_raw(Box::new(Watch { on_event, root_port: 0 }));
    let mut port: IONotificationPortRef = ptr::null_mut();
    let mut notifier: io_object_t = 0;
    // SAFETY: the out pointers are valid, and `watch` stays alive for the whole process.
    let root_port = unsafe { IORegisterForSystemPower(watch.cast(), &mut port, Some(changed), &mut notifier) };
    if root_port == 0 {
        tracing::warn!("cannot watch for sleep and wake");
        // SAFETY: registration failed, so nothing else holds the pointer.
        drop(unsafe { Box::from_raw(watch) });
        return false;
    }
    // SAFETY: no notification can arrive before its run loop source is added below.
    unsafe { (*watch).root_port = root_port };
    // SAFETY: `port` was just created by the registration.
    let source = unsafe { IONotificationPort::run_loop_source(port) };
    match (source, CFRunLoop::main()) {
        // SAFETY: reading an immutable constant of the framework.
        (Some(source), Some(main)) => main.add_source(Some(&source), unsafe { kCFRunLoopCommonModes }),
        _ => tracing::warn!("cannot receive sleep and wake notifications"),
    }
    true
}

unsafe extern "C-unwind" fn changed(data: *mut c_void, _: io_service_t, message: u32, argument: *mut c_void) {
    // SAFETY: `data` is the `Watch` registered above, which is never freed.
    let watch = unsafe { &*data.cast::<Watch>() };
    // The system waits for an answer to these, the argument identifies the request.
    let (root_port, request) = (watch.root_port, argument as isize);
    if message == kIOMessageCanSystemSleep {
        IOAllowPowerChange(root_port, request);
    } else if message == kIOMessageSystemWillSleep {
        (watch.on_event)(Event::Sleep);
        let allow = move || {
            IOAllowPowerChange(root_port, request);
        };
        match DispatchTime::try_from(SLEEP_GRACE) {
            Ok(when) => {
                let _ = DispatchQueue::main().after(when, allow);
            }
            Err(()) => allow(),
        }
    } else if message == kIOMessageSystemHasPoweredOn {
        (watch.on_event)(Event::Wake);
    }
}
