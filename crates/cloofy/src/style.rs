//! Minimal ANSI styling for the CLI commands; plain text when piped or with `NO_COLOR`.

use std::{io::IsTerminal, sync::LazyLock};

static ENABLED: LazyLock<bool> =
    LazyLock::new(|| std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none());

fn paint(code: &str, text: &str) -> String {
    if *ENABLED { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_owned() }
}

pub fn bold(text: &str) -> String {
    paint("1", text)
}

pub fn dim(text: &str) -> String {
    paint("2", text)
}

pub fn green(text: &str) -> String {
    paint("38;2;30;215;96", text)
}

pub fn red(text: &str) -> String {
    paint("38;2;255;92;92", text)
}

pub fn cyan(text: &str) -> String {
    paint("38;2;94;200;255", text)
}
