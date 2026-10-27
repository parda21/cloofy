//! The queue page: what plays next, from Spotify's pushes about the device that plays and from
//! cloofy's own player.

use std::{collections::HashSet, time::Instant};

use cloofy_core::catalog::Track;

use super::{App, Focus, QUEUE_GRACE};
use crate::{
    commands::Command,
    queue::{Provider, Queue, Upcoming},
    views::{QueueRow, QueueView, View},
};

impl App {
    /// Opens the queue page, or goes back from it, like Spotify's queue button.
    pub(super) fn toggle_queue(&mut self) {
        if !self.zen && matches!(self.view, View::Queue(_)) {
            return self.back();
        }
        self.zen = false;
        if !matches!(self.view, View::Queue(_)) {
            let id = self.next_id();
            self.push_view(View::Queue(QueueView::new(id)));
        }
        self.focus = Focus::Content;
        self.check_queue();
    }

    /// Takes a newer report of what plays next. One that differs also settles a shuffle
    /// cloofy's player didn't report.
    pub(super) fn set_queue(&mut self, queue: Queue) {
        if queue == self.queue {
            return;
        }
        self.queue = queue;
        self.queue_version += 1;
        self.queue_reshuffled = false;
        let listed: HashSet<&str> = self.queue.next.iter().map(|t| t.uri.as_str()).collect();
        self.queue_tracks.retain(|uri, _| listed.contains(uri.as_str()));
        self.queue_tracks_missing.retain(|uri| listed.contains(uri.as_str()));
        self.check_queue();
    }

    /// cloofy's player shuffled what plays next without saying how; the Web API tells once
    /// the player has passed it on to Spotify.
    pub(super) fn queue_reshuffled(&mut self) {
        self.queue_reshuffled = true;
        self.upcoming_asked = None;
        self.upcoming_due = Some(Instant::now() + QUEUE_GRACE);
    }

    /// Brings the open queue page up to date: loads the details it lacks, and asks the Web API
    /// what plays next when Spotify hasn't said in its own way a while after the track changed.
    pub(super) fn check_queue(&mut self) {
        if !matches!(self.view, View::Queue(_)) {
            return;
        }
        self.load_queue_tracks();
        if self.upcoming_due.is_none() && self.wants_upcoming() {
            self.upcoming_due = Some((self.track_changed_at + QUEUE_GRACE).max(Instant::now()));
        }
        self.refresh_queue_view();
    }

    /// Whether the Web API should say what plays next: Spotify hasn't said for the playing
    /// track, or cloofy's player reshuffled. It is asked once per report.
    fn wants_upcoming(&self) -> bool {
        let Some(track) = &self.now.track else { return false };
        let asked = self
            .upcoming_asked
            .as_ref()
            .is_some_and(|(uri, version)| *uri == track.uri && *version == self.queue_version);
        (self.local_active || self.remote_active) && !asked && (self.queue_reshuffled || !self.queue.knows(&track.uri))
    }

    /// Asks the Web API what plays next, once it is due.
    pub(super) fn ask_upcoming(&mut self) {
        if self.upcoming_due.is_none_or(|due| Instant::now() < due) {
            return;
        }
        self.upcoming_due = None;
        if matches!(self.view, View::Queue(_))
            && self.wants_upcoming()
            && let Some(track) = &self.now.track
        {
            self.upcoming_asked = Some((track.uri.clone(), self.queue_version));
            self.upcoming_pending = true;
            self.loader.upcoming(track.uri.clone());
        }
        self.refresh_queue_view();
    }

    /// The Web API's list of what plays after `playing`. A report that came in meanwhile, or
    /// another track, makes it outdated.
    pub(super) fn on_upcoming(&mut self, playing: String, result: Result<Vec<Track>, String>) {
        self.upcoming_pending = false;
        let tracks = match result {
            Ok(tracks) => tracks,
            Err(err) => {
                tracing::warn!("what plays next: {err}");
                return self.refresh_queue_view();
            }
        };
        let asked = self
            .upcoming_asked
            .as_ref()
            .is_some_and(|(uri, version)| *uri == playing && *version == self.queue_version);
        if !asked || self.now.track.as_ref().is_none_or(|t| t.uri != playing) {
            return self.refresh_queue_view();
        }
        let context = self.last_context.clone().unwrap_or_else(|| self.queue.context.clone());
        let queue = self.queue.with_listing(&playing, context, tracks.iter().map(|t| t.uri.clone()).collect());
        // The list comes with the details, so none need loading.
        self.queue_tracks.extend(tracks.into_iter().map(|t| (t.uri.clone(), Some(t))));
        self.set_queue(queue);
        self.refresh_queue_view();
    }

    /// Takes in details of tracks in the queue.
    pub(super) fn on_queue_tracks(&mut self, asked: Vec<String>, tracks: Vec<Track>, reached: bool) {
        let found: HashSet<String> = tracks.iter().map(|t| t.uri.clone()).collect();
        for track in tracks {
            if let Some(slot) = self.queue_tracks.get_mut(&track.uri) {
                *slot = Some(track);
            }
        }
        // Unanswered tracks stop loading: ones Spotify has nothing for stay off the page, the
        // rest are asked for again with the next report of what plays next.
        for uri in asked.into_iter().filter(|uri| !found.contains(uri)) {
            if matches!(self.queue_tracks.get(&uri), Some(None)) {
                self.queue_tracks.remove(&uri);
                if reached {
                    self.queue_tracks_missing.insert(uri);
                }
            }
        }
        self.refresh_queue_view();
    }

    /// Loads the details of listed tracks that lack them, taking what open lists already have,
    /// and whether they are liked.
    fn load_queue_tracks(&mut self) {
        let (mut missing, mut unchecked) = (Vec::new(), Vec::new());
        for entry in &self.queue.next {
            if self.queue_tracks.contains_key(&entry.uri) || self.queue_tracks_missing.contains(&entry.uri) {
                continue;
            }
            let known = self.listed_tracks().find(|t| t.uri == entry.uri).cloned();
            if known.is_none() {
                missing.push(entry.uri.clone());
            }
            if !self.saved.contains_key(&entry.uri) {
                unchecked.push(entry.uri.clone());
            }
            self.queue_tracks.insert(entry.uri.clone(), known);
        }
        if !missing.is_empty() {
            self.loader.queue_tracks(missing);
        }
        if !unchecked.is_empty() {
            self.loader.check_saved(unchecked);
        }
    }

    /// Songs of the track lists in the history, the open one first.
    fn listed_tracks(&self) -> impl Iterator<Item = &Track> {
        std::iter::once(&self.view).chain(self.history.iter().rev()).flat_map(|view| match view {
            View::Tracks(v) => &v.tracks[..],
            _ => &[],
        })
    }

    /// Rebuilds the queue page from what is known, if it is open.
    pub(super) fn refresh_queue_view(&mut self) {
        if !matches!(self.view, View::Queue(_)) {
            return;
        }
        let rows = self.queue_rows();
        let has_next = rows.iter().any(|row| matches!(row, QueueRow::Next { .. }));
        let playing = self.now.track.as_ref().map(|t| t.uri.as_str());
        let lacking = self.queue.after(playing).iter().any(|t| matches!(self.queue_tracks.get(&t.uri), Some(None)));
        let asking = self.upcoming_due.is_some() || self.upcoming_pending;
        let unknown = playing.is_some_and(|uri| !self.queue.knows(uri)) && asking;
        if let View::Queue(v) = &mut self.view {
            v.set_rows(rows);
            v.loading = !has_next && (lacking || unknown);
        }
    }

    /// The queue page's rows: the playing track, what the user queued, then the rest.
    fn queue_rows(&self) -> Vec<QueueRow> {
        let Some(now) = &self.now.track else { return Vec::new() };
        let mut rows = vec![QueueRow::Header("Now playing".into()), QueueRow::Playing(now.to_track())];
        let upcoming = self.queue.after(Some(&now.uri));
        let mut section = |title: String, provider: Provider| {
            let entries: Vec<QueueRow> = upcoming
                .iter()
                .filter(|entry| entry.provider == provider)
                .filter_map(|entry| {
                    let track = self.queue_tracks.get(&entry.uri)?.clone()?;
                    Some(QueueRow::Next { track, entry: entry.clone() })
                })
                .collect();
            if !entries.is_empty() {
                rows.push(QueueRow::Header(title));
                rows.extend(entries);
            }
        };
        section("Next in queue".into(), Provider::Queue);
        let from = self.queue_context_name().map_or_else(|| "Next up".to_owned(), |name| format!("Next from: {name}"));
        section(from, Provider::Context);
        section("Next up".into(), Provider::Unknown);
        rows
    }

    /// The name of what the rest of the queue comes from, like “Late Night Coding”.
    fn queue_context_name(&self) -> Option<String> {
        let uri = Some(self.queue.context.as_str()).filter(|uri| !uri.is_empty()).or(self.last_context.as_deref())?;
        let label = self.context_label(uri);
        let name = label.as_deref().map(|label| label.split_once(" · ").map_or(label, |(_, name)| name));
        name.map(str::to_owned).or_else(|| self.queue.context_name.clone())
    }

    /// Plays the selected row of the queue page: skips ahead to it, or resumes the playing song.
    pub(super) fn play_queue_row(&mut self, row: QueueRow) {
        match row {
            QueueRow::Playing(_) if !self.now.playing => self.run(Command::PlayPause),
            QueueRow::Next { track, entry } => self.skip_to(track, entry),
            QueueRow::Playing(_) | QueueRow::Header(_) => {}
        }
    }

    /// Skips ahead to a track that plays later, as a click in Spotify's queue does: what lies
    /// between is passed over, the rest of the queue stays.
    pub(super) fn skip_to(&mut self, track: Track, entry: Upcoming) {
        if !self.is_connected() {
            return self.not_connected();
        }
        if self.local_active && !self.remote_active {
            return self.skip_here(track, entry);
        }
        let device = match (self.remote_active, self.local_active) {
            (true, _) => self.active_device.clone(),
            (false, true) => self.device_id.clone(),
            (false, false) => None,
        };
        let Some(device) = device else {
            return self.notice("Nothing is playing here yet · press space to start".into());
        };
        // Polls from before the skip would show the track it skips.
        self.last_command = Instant::now();
        self.loader.skip_to(device, entry);
        self.now.start(Some(track.uri.clone()));
        self.show_early(&track, track.image_url.clone());
    }

    /// Skips ahead on cloofy's own player, with its own commands rather than a command sent
    /// through Spotify to the device it comes from.
    fn skip_here(&mut self, track: Track, entry: Upcoming) {
        if entry.provider == Provider::Queue {
            // The queued songs before it are passed over; the ones after it stay queued.
            let playing = self.now.track.as_ref().map(|t| t.uri.as_str());
            let queued: Vec<String> = self
                .queue
                .after(playing)
                .iter()
                .filter(|t| t.provider == Provider::Queue)
                .map(|t| t.uri.clone())
                .collect();
            let Some(index) = queued.iter().position(|uri| *uri == entry.uri) else {
                return self.error("This song isn't in the queue any more.".into());
            };
            let sent = self.control(|p| {
                p.clear_queue()?;
                queued[index..].iter().try_for_each(|uri| p.add_to_queue(uri))?;
                p.next()
            });
            if !sent {
                return;
            }
            self.now.start(Some(track.uri.clone()));
        } else {
            // A song of the playlist or album starts there, as a click on its page does. The
            // queued songs stay queued.
            let Some(context) = self.queue_context() else {
                return self.error("Spotify hasn't said yet what plays next.".into());
            };
            self.play(context, Some(track.uri.clone()));
        }
        self.show_early(&track, track.image_url.clone());
    }

    /// Skips ahead to an entry of the queue page.
    pub(super) fn skip_to_entry(&mut self, entry: Upcoming) {
        match self.queue_tracks.get(&entry.uri).cloned().flatten() {
            Some(track) => self.skip_to(track, entry),
            None => self.error("This song isn't in the queue any more.".into()),
        }
    }

    /// The device took the skip, or turned it down.
    pub(super) fn on_skipped_to(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => self.next_poll = self.next_poll.min(Instant::now() + super::CONFIRM_DELAY),
            Err(err) => {
                self.cancel_start();
                self.error(format!("Queue: {err}"));
            }
        }
    }

    /// The queue entry the cursor of the queue page is on, if it is one.
    pub(super) fn selected_upcoming(&self) -> Option<(Track, Upcoming)> {
        let View::Queue(v) = &self.view else { return None };
        match v.selected()? {
            QueueRow::Next { track, entry } => Some((track.clone(), entry.clone())),
            _ => None,
        }
    }

    /// Takes a song the user queued out of the queue again. The device's next report shows it
    /// gone.
    pub(super) fn remove_from_queue(&mut self, name: String, entry: Upcoming) {
        if entry.provider != Provider::Queue {
            return self.error("Only songs added to the queue can be removed.".into());
        }
        if self.remote_active {
            match (self.queue.without(&entry), self.active_device.clone()) {
                (Some(listed), Some(device)) => self.loader.set_queue(self.active, device, listed),
                _ => return self.error("Spotify hasn't said yet what plays next there.".into()),
            }
        } else {
            // cloofy's player takes no new list from here: it empties its queue and gets the
            // rest back in order.
            let playing = self.now.track.as_ref().map(|t| t.uri.as_str());
            let queued: Vec<&Upcoming> =
                self.queue.after(playing).iter().filter(|t| t.provider == Provider::Queue).collect();
            let Some(index) = queued.iter().position(|t| **t == entry) else { return };
            let keep: Vec<String> =
                queued.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, t)| t.uri.clone()).collect();
            let sent = self.control(|p| {
                p.clear_queue()?;
                keep.iter().try_for_each(|uri| p.add_to_queue(uri))
            });
            if !sent {
                return;
            }
        }
        self.notice(format!("“{name}” removed from queue"));
    }

    /// Delete on the queue page: removes the selected song, if it was queued.
    pub(super) fn remove_selected_from_queue(&mut self) {
        match self.selected_upcoming() {
            Some((track, entry)) if entry.provider == Provider::Queue => self.remove_from_queue(track.name, entry),
            Some(_) => self.error("Only songs added to the queue can be removed.".into()),
            None => {}
        }
    }

    /// The demo's queue: its queued songs, then the rest of the playlist.
    pub(super) fn start_demo_queue(&mut self, queued: Vec<Track>, rest: Vec<Track>) {
        let entry = |track: &Track, provider| Upcoming { uri: track.uri.clone(), uid: String::new(), provider };
        let next =
            queued.iter().map(|t| entry(t, Provider::Queue)).chain(rest.iter().map(|t| entry(t, Provider::Context)));
        self.queue = Queue {
            current: self.now.track.as_ref().map(|t| t.uri.clone()),
            next: next.collect(),
            context: self.last_context.clone().unwrap_or_default(),
            ..Queue::default()
        };
        self.queue_tracks = queued.into_iter().chain(rest).map(|t| (t.uri.clone(), Some(t))).collect();
        self.loader.check_saved(self.queue.next.iter().map(|t| t.uri.clone()).collect());
    }

    /// The context a queue row's track plays in, for its menu.
    pub(super) fn queue_context(&self) -> Option<String> {
        Some(self.queue.context.clone()).filter(|uri| !uri.is_empty()).or_else(|| self.last_context.clone())
    }
}
