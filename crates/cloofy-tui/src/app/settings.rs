//! The Settings page: moving through it, and what each change does right away.

use std::time::Duration;

use cloofy_spotify::config;
use crossterm::event::{KeyCode, KeyEvent};

use super::{App, COVER_SIZE, Focus, ImageSlot};
use crate::{
    preferences::{AccentChoice, Setting},
    theme::DEFAULT_ACCENT,
    views::{SettingsView, View},
};

impl App {
    pub fn is_demo(&self) -> bool {
        self.demo
    }

    /// Whether this terminal can show desktop notifications.
    pub fn can_notify(&self) -> bool {
        self.notifier.is_some()
    }

    /// Where settings are saved, with the home directory as `~`.
    pub fn config_path(&self) -> String {
        let path = self.paths.config_file();
        match std::env::var_os("HOME").and_then(|home| path.strip_prefix(home).ok().map(|p| p.to_owned())) {
            Some(relative) => format!("~/{}", relative.display()),
            None => path.display().to_string(),
        }
    }

    pub(super) fn open_settings(&mut self) {
        self.zen = false;
        if !matches!(self.view, View::Settings(_)) {
            let id = self.next_id();
            self.push_view(View::Settings(SettingsView::new(id)));
        }
        self.focus = Focus::Content;
    }

    pub(super) fn on_settings_key(&mut self, key: KeyEvent) {
        let View::Settings(view) = &mut self.view else { return };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => view.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => view.move_by(1),
            KeyCode::Home | KeyCode::Char('g') => view.move_by(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => view.move_by(isize::MAX / 2),
            KeyCode::Left | KeyCode::Char('h') => {
                let setting = view.setting();
                self.change_setting(setting, false);
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => {
                let setting = view.setting();
                self.change_setting(setting, true);
            }
            KeyCode::Esc => self.back(),
            _ => {}
        }
    }

    /// Moves `setting` to its next or previous choice, applies it and saves it.
    pub(super) fn change_setting(&mut self, setting: Setting, forward: bool) {
        self.prefs.step(setting, forward);
        match setting {
            Setting::Crossfade => {
                if let Some(controller) = &self.controller {
                    controller.set_crossfade(Duration::from_secs(self.prefs.crossfade));
                }
            }
            Setting::Covers if self.prefs.covers => {
                if let Some(url) = self.now.track.as_ref().and_then(|t| t.cover_url.clone()) {
                    self.load_cover(url);
                }
            }
            Setting::Covers => self.drop_covers(),
            Setting::Images => self.set_image_protocol(self.prefs.images),
            Setting::Accent => {
                let color = self.prefs.accent.fixed().or(self.cover_accent).unwrap_or(DEFAULT_ACCENT);
                if self.settings.low_power {
                    self.accent.jump(color);
                } else {
                    self.accent.set(color);
                }
            }
            Setting::Icons => crate::icons::init(self.prefs.icons.config()),
            Setting::PowerSaving => self.settings = self.prefs.apply(self.system_settings),
            Setting::Notifications => {}
            Setting::UpdateCheck => {
                if self.prefs.update_check && !self.demo {
                    self.check_for_update(false);
                }
            }
        }
        self.save_setting(setting);
    }

    /// Writes `setting` to the config file. The demo keeps changes until it closes.
    pub(super) fn save_setting(&mut self, setting: Setting) {
        if self.demo {
            return;
        }
        let (key, value) = self.prefs.entry(setting);
        if let Err(err) = config::store(&self.paths, key, value) {
            self.error(format!("{err:#}"));
        }
    }

    /// Loads the player's cover, unless cover art is off.
    pub(super) fn load_cover(&mut self, url: String) {
        if self.prefs.covers {
            self.loader.cover(url, COVER_SIZE);
        }
    }

    /// With cover art turned off: forgets every cover, here and in the terminal, and hands the
    /// memory back.
    fn drop_covers(&mut self) {
        for (_, slot) in self.images.drain() {
            if let ImageSlot::Ready(picture) = slot {
                self.terminal_out.extend(picture.terminal_delete());
            }
        }
        self.image_used.clear();
        self.prefetch.clear();
        if let Some(cover) = self.now.cover.take() {
            self.terminal_out.extend(cover.picture.terminal_delete());
        }
        if let Some(cover) = self.zen_cover.take() {
            self.terminal_out.extend(cover.picture.terminal_delete());
        }
        self.zen_cover_pending = None;
        self.cover_accent = None;
        if self.prefs.accent == AccentChoice::Cover {
            self.accent.set(DEFAULT_ACCENT);
        }
        crate::release_freed_memory();
    }
}
