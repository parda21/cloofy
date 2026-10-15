//! Prints the system's sleep and wake as cloofy sees them. Close the lid to try it:
//!
//! ```sh
//! cargo run -p cloofy-os --example sleep_wake
//! ```

fn main() {
    cloofy_os::run_main(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        let watching = cloofy_os::power::watch(move |event| {
            let _ = tx.send(event);
        });
        if !watching {
            eprintln!("This system doesn't report sleep and wake.");
            return;
        }
        println!("Watching for sleep and wake; Ctrl+C quits.");
        for event in rx {
            println!("{event:?}");
        }
    });
}
