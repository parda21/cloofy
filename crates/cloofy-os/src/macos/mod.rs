pub mod audio;
pub mod media;
pub mod power;
pub mod secrets;
pub mod settings;

use std::{panic, thread};

use dispatch2::DispatchQueue;
use objc2_core_foundation::CFRunLoop;

/// Same as the main thread's, which the work would otherwise have had.
const WORKER_STACK: usize = 8 << 20;

pub fn run_main<R: Send + 'static>(work: impl FnOnce() -> R + Send + 'static) -> R {
    let worker = thread::Builder::new()
        .name("cloofy".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _stop = StopMainLoop;
            work()
        })
        .expect("cannot start the worker thread");
    // Serves the main dispatch queue until stopped; it counts as a source, so this never
    // returns early for lack of work.
    CFRunLoop::run();
    worker.join().unwrap_or_else(|payload| panic::resume_unwind(payload))
}

/// Ends the main run loop once the work is done, also when it panicked.
struct StopMainLoop;

impl Drop for StopMainLoop {
    fn drop(&mut self) {
        // Queued for the main thread, the stop runs inside the loop, so it can't come too early.
        DispatchQueue::main().exec_async(|| {
            if let Some(main) = CFRunLoop::main() {
                main.stop();
            }
        });
    }
}
