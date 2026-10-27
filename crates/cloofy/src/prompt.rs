//! Asking in the terminal: one of a list with the arrow keys or its number, yes or no, and a
//! secret pasted without showing it. Keys are read one by one, so the terminal is in raw mode
//! while asking, and back to normal afterwards whatever happens.

use std::io::{IsTerminal, Write};

use anyhow::Result;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};

use crate::style;

/// Whether there is someone at a terminal to ask.
pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Raw mode until dropped.
struct Raw;

impl Raw {
    fn on() -> Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Raw)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// A key press, or text pasted at once.
enum Input {
    Key(KeyEvent),
    Paste(String),
}

fn next_input() -> Result<Input> {
    loop {
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => return Ok(Input::Key(key)),
            Event::Paste(text) => return Ok(Input::Paste(text)),
            _ => {}
        }
    }
}

fn cancels(key: &KeyEvent) -> bool {
    key.code == KeyCode::Esc || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// Asks `question` with `items` below it; returns the index picked, `None` when cancelled. The
/// list goes away afterwards, and the question stays with the answer.
pub fn pick(question: &str, items: &[String]) -> Result<Option<usize>> {
    let mut out = std::io::stdout();
    let mut selected = 0;
    let draw = |out: &mut std::io::Stdout, selected: usize| -> Result<()> {
        write!(out, "\r\x1b[2K  {}\r\n", style::bold(question))?;
        for (index, item) in items.iter().enumerate() {
            let number = style::dim(&(index + 1).to_string());
            let line = if index == selected {
                format!("{} {number}  {}", style::green("›"), style::bold(item))
            } else {
                format!("  {number}  {item}")
            };
            write!(out, "\r\x1b[2K  {line}\r\n")?;
        }
        let keys = format!("↑ ↓ or 1–{} · enter picks · esc cancels", items.len());
        write!(out, "\r\x1b[2K  {}", style::dim(&keys))?;
        out.flush()?;
        Ok(())
    };
    let raw = Raw::on()?;
    draw(&mut out, selected)?;
    let picked = loop {
        let Input::Key(key) = next_input()? else { continue };
        match key.code {
            _ if cancels(&key) => break None,
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(items.len() - 1),
            KeyCode::Enter => break Some(selected),
            KeyCode::Char(digit @ '1'..='9') => {
                let index = digit as usize - '1' as usize;
                if index < items.len() {
                    break Some(index);
                }
                continue;
            }
            _ => continue,
        }
        // Back up past the items to the question, to draw the list again.
        write!(out, "\x1b[{}A", items.len() + 1)?;
        draw(&mut out, selected)?;
    };
    drop(raw);
    // The list and the keys make way for the answer.
    write!(out, "\x1b[{}A\r\x1b[J", items.len() + 1)?;
    let answer = picked.map_or_else(|| style::dim("cancelled"), |index| items[index].clone());
    writeln!(out, "  {} {answer}", style::bold(question))?;
    Ok(picked)
}

/// Asks a yes-or-no `question`; enter takes `default`, esc says no.
pub fn confirm(question: &str, default: bool) -> Result<bool> {
    let mut out = std::io::stdout();
    let choices = if default { "Y/n" } else { "y/N" };
    write!(out, "  {question} {} ", style::dim(&format!("[{choices}]")))?;
    out.flush()?;
    let raw = Raw::on()?;
    let answer = loop {
        let Input::Key(key) = next_input()? else { continue };
        match key.code {
            _ if cancels(&key) => break false,
            KeyCode::Enter => break default,
            KeyCode::Char('y' | 'Y') => break true,
            KeyCode::Char('n' | 'N') => break false,
            _ => {}
        }
    };
    drop(raw);
    writeln!(out, "{}", if answer { "yes" } else { "no" })?;
    Ok(answer)
}

/// Reads a secret, pasted or typed, until enter, without showing it: only how long it is.
/// `None` when cancelled.
pub fn secret(prompt: &str) -> Result<Option<String>> {
    let mut out = std::io::stdout();
    writeln!(out, "  {prompt}")?;
    let raw = Raw::on()?;
    crossterm::execute!(out, EnableBracketedPaste)?;
    let mut text = String::new();
    let show = |out: &mut std::io::Stdout, text: &str| -> Result<()> {
        let length =
            if text.is_empty() { "waiting…".to_owned() } else { format!("{} characters", text.chars().count()) };
        write!(out, "\r\x1b[2K  {} {}", style::green("›"), style::dim(&length))?;
        out.flush()?;
        Ok(())
    };
    show(&mut out, &text)?;
    let entered = loop {
        match next_input()? {
            Input::Paste(pasted) => text.push_str(&pasted),
            Input::Key(key) if cancels(&key) => break false,
            Input::Key(KeyEvent { code: KeyCode::Enter, .. }) => break true,
            Input::Key(KeyEvent { code: KeyCode::Backspace, .. }) => {
                text.pop();
            }
            Input::Key(KeyEvent { code: KeyCode::Char(c), .. }) => text.push(c),
            Input::Key(_) => {}
        }
        show(&mut out, &text)?;
    };
    crossterm::execute!(out, DisableBracketedPaste)?;
    drop(raw);
    writeln!(out)?;
    Ok(entered.then_some(text).filter(|text| !text.trim().is_empty()))
}
