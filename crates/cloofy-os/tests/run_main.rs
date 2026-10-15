//! `run_main` has to hand back the work's result and end the main loop, however quickly the
//! work finishes. Runs without the test harness, which would keep the main thread for itself.

use std::{panic, thread, time::Duration};

fn main() {
    // Done before the main loop even starts.
    assert_eq!(cloofy_os::run_main(|| 42), 42);

    // Done while the main loop runs.
    let late = cloofy_os::run_main(|| {
        thread::sleep(Duration::from_millis(50));
        "late"
    });
    assert_eq!(late, "late");

    // A panic ends the loop too, and reaches the caller.
    let report = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let result = panic::catch_unwind(|| cloofy_os::run_main(|| panic!("expected panic")));
    panic::set_hook(report);
    assert!(result.is_err());

    println!("run_main: ok");
}
