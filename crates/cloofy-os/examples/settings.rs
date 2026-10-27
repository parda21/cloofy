//! Prints the system settings cloofy follows each time they change. Turn on Reduce Motion or
//! power saving to try it:
//!
//! ```sh
//! cargo run -p cloofy-os --example settings
//! ```

use std::{thread, time::Duration};

fn main() {
    cloofy_os::run_main(|| {
        println!("Watching the settings; Ctrl+C quits.");
        let mut shown = None;
        loop {
            let settings = cloofy_os::settings::current();
            if shown != Some(settings) {
                println!("{settings:?}");
                shown = Some(settings);
            }
            thread::sleep(Duration::from_millis(500));
        }
    });
}
