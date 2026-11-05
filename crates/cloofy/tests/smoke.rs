//! The real binary, in a pseudo-terminal of its own: it starts, takes keys, and leaves the
//! terminal as it found it however it ends. The interface itself is tested in cloofy-tui; this
//! is what only a real terminal shows.
#![cfg(unix)]

use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthChar;

const START: Duration = Duration::from_secs(30);
const END: Duration = Duration::from_secs(10);
const CTRL_Q: &[u8] = b"\x11";

#[test]
fn version_names_the_services() {
    let output = Command::new(env!("CARGO_BIN_EXE_cloofy")).arg("--version").output().unwrap();
    let services: Vec<&str> = [(cfg!(feature = "spotify"), "spotify"), (cfg!(feature = "ytmusic"), "ytmusic")]
        .into_iter()
        .filter_map(|(built, name)| built.then_some(name))
        .collect();
    let services = if services.is_empty() { "demo only".to_owned() } else { services.join(", ") };
    let expected = format!("cloofy {} ({services})\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
}

#[test]
fn the_demo_takes_keys_and_quits_cleanly() {
    let mut tty = Tty::spawn(&["demo"]);
    tty.wait_for("Good evening");
    tty.send(b"/daft");
    tty.wait_for("One More Time");
    tty.send(b"\x1b");
    tty.send(CTRL_Q);
    assert!(tty.wait_exit().success());
    tty.assert_restored();
}

#[test]
fn a_closed_window_ends_it_cleanly() {
    for signal in [libc::SIGHUP, libc::SIGTERM] {
        let mut tty = Tty::spawn(&["demo"]);
        tty.wait_for("Good evening");
        unsafe { libc::kill(tty.child.id() as i32, signal) };
        assert!(tty.wait_exit().success(), "after signal {signal}");
        tty.assert_restored();
    }
}

/// cloofy on a pseudo-terminal of 120×36, with a home of its own.
struct Tty {
    child: Child,
    master: File,
    /// The terminal's local modes before cloofy started: echo, line editing and the like.
    modes: libc::tcflag_t,
    output: Arc<Mutex<Vec<u8>>>,
    home: PathBuf,
}

impl Tty {
    fn spawn(args: &[&str]) -> Self {
        let (mut master, mut slave) = (0, 0);
        let mut size = libc::winsize { ws_row: 36, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
        let opened = unsafe {
            libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &raw mut size)
        };
        assert_eq!(opened, 0, "openpty: {}", std::io::Error::last_os_error());
        let (master, slave) = unsafe { (File::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let modes = local_modes(&master);

        let home = std::env::temp_dir().join(format!("cloofy-smoke-{}-{}", std::process::id(), unique()));
        std::fs::create_dir_all(&home).unwrap();
        let stdio = || Stdio::from(slave.try_clone().unwrap());
        let mut command = Command::new(env!("CARGO_BIN_EXE_cloofy"));
        command
            .args(args)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("TERM", "xterm-256color")
            // No terminal answers here, so cloofy shouldn't ask one which images it draws.
            .env("CLOOFY_IMAGES", "blocks")
            .stdin(stdio())
            .stdout(stdio())
            .stderr(stdio());
        // A session of its own, with the pseudo-terminal as its terminal, as a shell gives it.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("cloofy starts");

        let output = Arc::new(Mutex::new(Vec::new()));
        let (mut reader, sink) = (master.try_clone().unwrap(), output.clone());
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            // Ends with an error once cloofy is gone, on Linux, or never: the test ends first.
            while let Ok(read @ 1..) = reader.read(&mut buffer) {
                sink.lock().unwrap().extend_from_slice(&buffer[..read]);
            }
        });
        Self { child, master, modes, output, home }
    }

    fn send(&mut self, keys: &[u8]) {
        self.master.write_all(keys).unwrap();
        self.master.flush().unwrap();
    }

    fn raw(&self) -> Vec<u8> {
        self.output.lock().unwrap().clone()
    }

    /// Waits until the screen has shown `text`.
    #[track_caller]
    fn wait_for(&self, text: &str) {
        let started = Instant::now();
        while !screen(&self.raw()).contains(text) {
            assert!(started.elapsed() < START, "{text:?} never showed:\n{}", screen(&self.raw()));
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_exit(&mut self) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(started.elapsed() < END, "cloofy didn't end");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The terminal is back as cloofy found it: main screen, no mouse or focus reports, and
    /// typing echoes and edits lines again.
    #[track_caller]
    fn assert_restored(&self) {
        let raw = String::from_utf8_lossy(&self.raw()).into_owned();
        let last = |sequence: &str| raw.rfind(sequence);
        let modes = [
            ("\x1b[?1049h", "\x1b[?1049l", "the alternate screen"),
            ("\x1b[?1000h", "\x1b[?1000l", "mouse reports"),
            ("\x1b[?1004h", "\x1b[?1004l", "focus reports"),
        ];
        for (on, off, what) in modes {
            assert!(last(on).is_some(), "{what} never went on");
            assert!(last(off) > last(on), "{what} stayed on");
        }
        assert_eq!(local_modes(&self.master), self.modes, "the terminal stayed in raw mode");
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Read through our end: once cloofy is gone, macOS takes its end away.
fn local_modes(fd: &impl AsRawFd) -> libc::tcflag_t {
    let mut settings = unsafe { std::mem::zeroed::<libc::termios>() };
    assert_eq!(unsafe { libc::tcgetattr(fd.as_raw_fd(), &mut settings) }, 0);
    settings.c_lflag
}

fn unique() -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// What a terminal of 120×36 shows after `raw`. Enough of one for ratatui, which only moves the
/// cursor, sets colors and clears: text goes where the cursor is, and the rest is skipped.
fn screen(raw: &[u8]) -> String {
    const ROWS: usize = 36;
    const COLUMNS: usize = 120;
    let mut cells = vec![vec![" ".to_owned(); COLUMNS]; ROWS];
    let (mut row, mut column) = (0, 0);
    let raw = String::from_utf8_lossy(raw);
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    let mut end = ' ';
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            end = c;
                            break;
                        }
                        params.push(c);
                    }
                    let numbers: Vec<usize> = params.split(';').map(|n| n.parse().unwrap_or(1)).collect();
                    match end {
                        'H' => {
                            row = numbers[0].clamp(1, ROWS) - 1;
                            column = numbers.get(1).copied().unwrap_or(1).clamp(1, COLUMNS) - 1;
                        }
                        'J' if params == "2" => cells.iter_mut().flatten().for_each(|cell| *cell = " ".into()),
                        'h' | 'l' if params == "?1049" => {
                            cells.iter_mut().flatten().for_each(|cell| *cell = " ".into())
                        }
                        _ => {}
                    }
                }
                // Strings (OSC, DCS, APC, PM) end with BEL or ESC \.
                Some(']' | 'P' | '_' | '^') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => column = 0,
            '\n' => row = (row + 1).min(ROWS - 1),
            c if c.is_control() => {}
            c => {
                let width = c.width().unwrap_or(0);
                if width == 0 || column >= COLUMNS {
                    continue;
                }
                cells[row][column] = c.to_string();
                for skipped in 1..width {
                    if let Some(cell) = cells[row].get_mut(column + skipped) {
                        cell.clear();
                    }
                }
                column += width;
            }
        }
    }
    cells.iter().map(|row| row.concat().trim_end().to_owned()).collect::<Vec<_>>().join("\n")
}
