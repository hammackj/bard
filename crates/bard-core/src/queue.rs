//! The queue lives here and nowhere else. A port of `src/main/queue.ts`.
//!
//! The UI sends intents and re-renders whatever comes back
//! ([`CoreEvent::QueueChanged`], [`CoreEvent::NowPlaying`],
//! [`CoreEvent::PlayTrack`]), so there is one source of truth for what plays
//! next.

use crate::CoreEvent;
use crate::bard::Bard;
use crate::bard::Transition;
use crate::deck::draw;
use crate::history::{BackStep, push_played, remember_recent, step_back, wrap_for_repeat_all};
use crate::playlist::{next_in_album_rolling, pick_random, shuffled};
use crate::stores::{PLAY_THRESHOLD_RATIO, PLAY_THRESHOLD_SEC};
use crate::types::{AutoFillMode, ContentIdStatus, QueueItem, RepeatMode, Track};
use crate::util::now_ms;
use std::collections::HashSet;

fn new_item(track: Track) -> QueueItem {
    QueueItem { id: uuid::Uuid::new_v4().to_string(), track, added_at: now_ms() }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

impl Bard {
    pub fn queue(&self) -> &[QueueItem] {
        &self.queue
    }

    pub fn now_playing(&self) -> Option<&QueueItem> {
        self.now_playing.as_ref()
    }

    /// Tracks eligible for automatic play.
    ///
    /// Albums flagged as Content ID claimed are withheld from shuffle, fill and
    /// endless -- the whole point of testing them is to keep them from turning
    /// up unattended mid-stream. Playing one deliberately is still allowed:
    /// that is an explicit choice, not the player deciding for you.
    pub fn playable_tracks(&self) -> Vec<Track> {
        let all = self.library.tracks();

        // Streamer mode is an allow-list and takes precedence: only albums
        // actually checked and found safe may play. Deliberately no "fall back
        // to everything if the result is empty" escape here, unlike the claimed
        // filter below -- falling back is exactly the failure this mode exists
        // to prevent, and an empty pool while armed is a real answer that the
        // UI has to explain.
        if self.is_streamer_armed() {
            return all.iter().filter(|t| self.albums.status(&t.album) == ContentIdStatus::Safe).cloned().collect();
        }

        if !self.config.get().playback.exclude_claimed {
            return all.to_vec();
        }
        let blocked = self.albums.claimed();
        if blocked.is_empty() {
            return all.to_vec();
        }
        let left: Vec<Track> = all.iter().filter(|t| !blocked.contains(&t.album)).cloned().collect();
        // If every album is flagged, fall back to the whole library rather than
        // falling silent -- silence would look like a bug, not a policy.
        if left.is_empty() { all.to_vec() } else { left }
    }

    /// Why a track may not play right now, or None when it may.
    ///
    /// Returns a reason rather than a boolean because refusing silently is the
    /// worst outcome available here: on a stream, a player that quietly does
    /// nothing reads as broken, and the user cannot tell a blocked track from a
    /// bug. Every refusal is surfaced as a [`CoreEvent::Notice`].
    pub fn blocked_reason(&self, track: &Track) -> Option<String> {
        if !self.is_streamer_armed() {
            return None;
        }
        match self.albums.status(&track.album) {
            ContentIdStatus::Safe => None,
            ContentIdStatus::Claimed => Some(format!("Streamer mode: \"{}\" is flagged as claimed", track.album)),
            ContentIdStatus::Untested => {
                Some(format!("Streamer mode: \"{}\" has not been checked for Content ID", track.album))
            }
        }
    }

    /// Draws the next track from the deck, rebuilding it when exhausted.
    fn draw_from_deck(&mut self) -> Option<Track> {
        let ids: Vec<String> = self.playable_tracks().into_iter().map(|t| t.id).collect();
        let weight = self.config.get().playback.loved_weight.max(1);
        let loved: HashSet<String> = self.loved.all().into_iter().collect();
        let weight_of = |id: &str| if weight > 1 && loved.contains(id) { weight as f64 } else { 1.0 };
        let avoid = self.now_playing.as_ref().map(|n| n.track.id.clone());
        let rng = &mut self.rng;
        let (id, deck) = draw(self.deck.as_ref(), &ids, &mut || rng.f64(), avoid.as_deref(), Some(&weight_of));
        self.deck = Some(deck);
        id.and_then(|id| self.library.get(&id).cloned())
    }

    /// Tops the queue up so playback never runs out.
    ///
    /// Deliberately fills the visible queue rather than picking a track at the
    /// last moment: you can see what is coming, and reorder or drop it, instead
    /// of being handed whatever the player decided a second ago.
    fn top_up_endless(&mut self) {
        let cfg = self.config.get().playback.clone();
        if std::env::var_os("BARD_ENDLESS_DEBUG").is_some() {
            eprintln!(
                "[endless] on={} look={} queue={} library={}",
                cfg.endless,
                cfg.endless_lookahead,
                self.queue.len(),
                self.library.tracks().len()
            );
        }
        if !cfg.endless {
            return;
        }
        let want = cfg.endless_lookahead.max(1) as usize;
        if self.queue.len() >= want {
            return;
        }

        let mut taken: HashSet<String> = self.queue.iter().map(|q| q.track.id.clone()).collect();
        let mut added: Vec<QueueItem> = Vec::new();
        // The anchor advances with every pick. Album mode walks forward from the
        // last track that will be queued, not from what is playing now.
        let mut anchor: Option<Track> =
            self.queue.last().map(|q| q.track.clone()).or_else(|| self.now_playing.as_ref().map(|n| n.track.clone()));

        let mut guard = 0;
        while self.queue.len() + added.len() < want && guard < want * 6 {
            guard += 1;
            let next = if cfg.auto_fill == AutoFillMode::Album {
                let pool = self.playable_tracks();
                let rng = &mut self.rng;
                next_in_album_rolling(&pool, anchor.as_ref(), &mut || rng.f64())
            } else {
                self.draw_from_deck()
            };
            let Some(next) = next else { break };
            anchor = Some(next.clone());
            // A duplicate is skipped, not treated as failure: the deck will deal
            // something else next time round.
            if !taken.insert(next.id.clone()) {
                continue;
            }
            added.push(new_item(next));
        }
        if added.is_empty() {
            return;
        }
        self.queue.extend(added);
        self.emit_queue();
    }

    fn remember(&mut self, track: &Track) {
        let limit = i64::from(self.config.get().playback.avoid_repeat);
        self.history = remember_recent(&self.history, &track.id, limit);
    }

    /// Counts a play once the track has actually been listened to.
    ///
    /// Counting on start would make skipping through the library inflate every
    /// number; 30 seconds, or half of a short track, is the point at which it
    /// was genuinely played rather than sampled.
    pub(crate) fn report_position(&mut self, seconds: f64, duration: f64) {
        self.position = seconds;
        let Some(np) = &self.now_playing else { return };
        if self.counted {
            return;
        }
        let enough = seconds >= PLAY_THRESHOLD_SEC || (duration > 0.0 && seconds >= duration * PLAY_THRESHOLD_RATIO);
        if !enough {
            return;
        }
        self.counted = true;
        self.stats.record_play(&np.track.id, now_ms());
        self.emit(CoreEvent::StatsChanged);
    }

    /// Called when playback leaves a track before it counted as played.
    fn credit_skip_if_unplayed(&mut self) {
        let Some(np) = &self.now_playing else { return };
        if self.counted || self.position <= 1.0 {
            return;
        }
        self.stats.record_skip(&np.track.id, now_ms());
        self.emit(CoreEvent::StatsChanged);
    }

    fn emit_queue(&mut self) {
        self.emit(CoreEvent::QueueChanged(self.queue.clone()));
        self.np.set_queue_length(self.queue.len());
    }

    /// Whether a progress report describes the track that is actually current.
    ///
    /// A crossfade keeps the outgoing track playing -- and reporting -- for the
    /// length of the fade, by which time the next one is already selected.
    /// Untagged reports (None) are accepted: there is nothing to disambiguate
    /// before a track has been loaded.
    pub fn is_current_track(&self, track_id: Option<&str>) -> bool {
        match track_id {
            None => true,
            Some(id) => self.now_playing.as_ref().is_some_and(|n| n.track.id == id),
        }
    }

    /// Picks the next track when the queue has run dry.
    ///
    /// Album mode carries on through the current album and rolls to another
    /// when it ends, so playback does not stop dead between records.
    fn auto_fill_track(&mut self) -> Option<Track> {
        match self.config.get().playback.auto_fill {
            AutoFillMode::Off => None,
            AutoFillMode::Album => {
                let pool = self.playable_tracks();
                let anchor = self.now_playing.as_ref().map(|n| n.track.clone());
                let rng = &mut self.rng;
                next_in_album_rolling(&pool, anchor.as_ref(), &mut || rng.f64())
            }
            AutoFillMode::Shuffle => {
                if let Some(t) = self.draw_from_deck() {
                    return Some(t);
                }
                // The fallback has to stay inside what is playable, not just
                // avoid recent history. The TypeScript passed the unplayable
                // ids as an exclusion to a picker that ignores its exclusion
                // once it covers everything -- so with nothing safe while
                // armed, it reached straight past the filter and played an
                // unchecked track. Picking from the playable pool cannot.
                let pool = self.playable_tracks();
                let recent: HashSet<String> = self.history.iter().cloned().collect();
                let rng = &mut self.rng;
                pick_random(&pool, &recent, &mut || rng.f64()).cloned()
            }
        }
    }

    fn notify(&self, message: String) {
        self.emit(CoreEvent::Notice(message));
    }

    /// Appends one track. Starts playback when nothing is playing.
    pub fn enqueue(&mut self, track_id: &str) -> Option<QueueItem> {
        let track = self.library.get(track_id)?.clone();
        if let Some(reason) = self.blocked_reason(&track) {
            self.notify(reason);
            return None;
        }
        let item = new_item(track);
        self.queue.push(item.clone());
        self.emit_queue();
        if self.now_playing.is_none() {
            self.play_next();
        }
        Some(item)
    }

    /// Appends `count` tracks (default: the fill-count setting) chosen at
    /// random from what is playable, avoiding recent history and what is
    /// already queued. Returns how many were added.
    pub fn fill_queue(&mut self, count: Option<u32>) -> usize {
        let cfg = self.config.get().playback.clone();
        let want = count.unwrap_or(cfg.fill_count).max(1) as usize;
        let room = (cfg.max_queue_length as usize).saturating_sub(self.queue.len());
        let n = want.min(room);
        if n == 0 {
            return 0;
        }

        // Exclude what is already queued as well as recent history, so a fill
        // does not stack duplicates. Picks stay inside the playable pool (see
        // auto_fill_track for why that is not done with an exclusion list).
        let pool = self.playable_tracks();
        let mut taken: HashSet<String> = self.history.iter().cloned().collect();
        taken.extend(self.queue.iter().map(|q| q.track.id.clone()));
        let mut added = Vec::new();
        for _ in 0..n {
            let rng = &mut self.rng;
            let Some(t) = pick_random(&pool, &taken, &mut || rng.f64()).cloned() else { break };
            taken.insert(t.id.clone());
            added.push(new_item(t));
        }
        if added.is_empty() {
            return 0;
        }
        let count = added.len();
        self.queue.extend(added);
        self.emit_queue();
        if self.now_playing.is_none() {
            self.play_next();
        }
        count
    }

    /// Reorders the pending queue at random; what is playing is untouched.
    pub fn shuffle_queue(&mut self) {
        if self.queue.len() < 2 {
            return;
        }
        let rng = &mut self.rng;
        self.queue = shuffled(&self.queue, &mut || rng.f64());
        self.emit_queue();
    }

    pub fn remove_from_queue(&mut self, queue_item_id: &str) {
        let before = self.queue.len();
        self.queue.retain(|i| i.id != queue_item_id);
        if self.queue.len() != before {
            self.emit_queue();
            self.top_up_endless();
        }
    }

    /// Removes newly-claimed albums from the pending queue.
    ///
    /// Excluding them from future picks is not enough: anything already queued
    /// would still play, which on a stream is exactly the moment it matters.
    pub(crate) fn drop_claimed_from_queue(&mut self) {
        if !self.config.get().playback.exclude_claimed {
            return;
        }
        let blocked = self.albums.claimed();
        if blocked.is_empty() {
            return;
        }
        let before = self.queue.len();
        self.queue.retain(|i| !blocked.contains(&i.track.album));
        if self.queue.len() != before {
            self.emit_queue();
            self.top_up_endless();
        }
    }

    /// Called when the endless setting changes, so it takes effect immediately.
    pub(crate) fn refresh_endless(&mut self) {
        self.top_up_endless();
    }

    /// Applies streamer mode to what is already in flight.
    ///
    /// Arming has to act on the present, not just on future picks. Anything
    /// queued would still play in a few minutes, and whatever is playing right
    /// now is on air already -- which is the moment the user armed the mode to
    /// stop.
    ///
    /// Called on every change so disarming re-stocks the queue too, rather than
    /// leaving it short until something else happens to trigger a top-up.
    pub(crate) fn refresh_streamer_mode(&mut self) {
        if !self.is_streamer_armed() {
            self.top_up_endless();
            return;
        }

        let before = self.queue.len();
        let queue = std::mem::take(&mut self.queue);
        self.queue = queue.into_iter().filter(|i| self.blocked_reason(&i.track).is_none()).collect();
        let dropped = before - self.queue.len();
        if dropped > 0 {
            self.emit_queue();
            self.notify(format!("Streamer mode: removed {dropped} queued track{} not marked safe", plural(dropped)));
        }

        // The track on air matters most. Skipping is not enough on its own: if
        // nothing safe can follow, play_next lands on None and stops, which is
        // the correct outcome -- silence beats an unchecked track going out.
        //
        // And it goes at once. With a crossfade set, a normal skip keeps the
        // outgoing track audible for the whole fade-out, up to twelve seconds
        // of exactly the track that must not be heard.
        let playing_reason = self.now_playing.as_ref().and_then(|n| self.blocked_reason(&n.track));
        if let Some(reason) = playing_reason {
            self.notify(format!("{reason} \u{2014} skipping"));
            self.cut_to_next();
            return;
        }
        // A track before it may still be fading out under a permitted one.
        let current = self.now_playing.as_ref().map(|n| n.track.id.clone());
        let fading_blocked =
            self.maybe_audible.iter().any(|t| Some(&t.id) != current.as_ref() && self.blocked_reason(t).is_some());
        if fading_blocked {
            self.maybe_audible.retain(|t| Some(&t.id) == current.as_ref());
            self.emit(CoreEvent::CutFade);
        }
        self.top_up_endless();
    }

    /// Plays the next track with no fade: the current one may not be heard.
    fn cut_to_next(&mut self) {
        self.cutting = true;
        self.play_next();
        // Stopping instead also cuts; the flag must not outlive this.
        self.cutting = false;
    }

    /// Tells the engine to play `track`. The one place PlayTrack is sent, so
    /// what may still be fading out is always known.
    fn emit_play(&mut self, track: Track) {
        let cut = std::mem::take(&mut self.cutting);
        if cut {
            self.maybe_audible.clear();
        }
        self.maybe_audible.retain(|t| t.id != track.id);
        self.maybe_audible.push(track.clone());
        self.emit(CoreEvent::PlayTrack(track, if cut { Transition::Cut } else { Transition::Normal }));
    }

    /// Playback stopped; the engine's stop ends any fade too.
    fn went_off_air(&mut self) {
        self.maybe_audible.clear();
    }

    /// Whether a track may play again from history (Repeat One, Repeat All,
    /// Previous): still in the library -- a removed folder's track finishes,
    /// but does not come back -- and permitted by streamer mode.
    fn may_replay(&self, track: &Track) -> bool {
        self.library.get(&track.id).is_some() && self.blocked_reason(track).is_none()
    }

    /// Moves the item at `from` to `to` (clamped into range).
    pub fn reorder_queue(&mut self, from: usize, to: usize) {
        if from == to || from >= self.queue.len() {
            return;
        }
        let to = to.min(self.queue.len() - 1);
        let moved = self.queue.remove(from);
        self.queue.insert(to, moved);
        self.emit_queue();
    }

    pub fn clear_queue(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        self.queue.clear();
        self.emit_queue();
    }

    /// Pulls the head of the queue and tells the audio engine to play it.
    pub(crate) fn play_next(&mut self) -> Option<QueueItem> {
        self.credit_skip_if_unplayed();
        self.counted = false;
        self.position = 0.0;

        // Stock the queue before drawing from it.
        //
        // Order matters: auto-fill returns a track whenever the library is
        // non-empty, so doing it first meant the "queue is empty" branch always
        // won and played directly, leaving the queue permanently empty. Endless
        // would then be invisible -- music kept playing but nothing was ever
        // queued to look at.
        self.top_up_endless();

        // The last check before anything goes on air. The queue is pruned when
        // the mode is armed and when a flag changes, but this is the boundary
        // that must hold whatever got the queue into its state.
        self.drop_blocked_from_head();

        let write = self.config.get().overlay.write_now_playing_json;
        if self.queue.is_empty() {
            // Not endless: fall back to a single auto-filled track, or stop.
            if let Some(filled) = self.auto_fill_track() {
                let item = new_item(filled.clone());
                self.now_playing = Some(item.clone());
                self.remember(&filled);
                self.played = push_played(&self.played, item.clone());
                self.np.set_now_playing(Some(item.clone()), write);
                self.emit(CoreEvent::NowPlaying(Some(item.clone())));
                self.emit_play(filled);
                return Some(item);
            }
            self.now_playing = None;
            self.went_off_air();
            self.np.set_now_playing(None, write);
            self.emit(CoreEvent::NowPlaying(None));
            return None;
        }
        let next = self.queue.remove(0);
        self.now_playing = Some(next.clone());
        self.remember(&next.track);
        self.played = push_played(&self.played, next.clone());
        self.emit_queue();
        self.top_up_endless();
        self.np.set_now_playing(Some(next.clone()), write);
        self.emit(CoreEvent::NowPlaying(Some(next.clone())));
        self.emit_play(next.track.clone());
        Some(next)
    }

    /// Drops tracks streamer mode does not permit from the head of the queue,
    /// saying so, until the head may play.
    fn drop_blocked_from_head(&mut self) {
        let mut dropped = false;
        while let Some(reason) = self.queue.first().and_then(|i| self.blocked_reason(&i.track)) {
            self.queue.remove(0);
            self.notify(format!("{reason} \u{2014} skipping"));
            dropped = true;
        }
        if dropped {
            self.emit_queue();
            self.top_up_endless();
        }
    }

    pub fn skip(&mut self) {
        self.play_next();
    }

    /// Stops playback: nothing is current, the queue is left as it is.
    pub fn stop(&mut self) {
        self.credit_skip_if_unplayed();
        self.counted = false;
        self.position = 0.0;
        self.now_playing = None;
        self.went_off_air();
        let write = self.config.get().overlay.write_now_playing_json;
        self.np.set_now_playing(None, write);
        self.emit(CoreEvent::NowPlaying(None));
    }

    /// Back one track, or restart the current one.
    ///
    /// Restarting when a few seconds in matches every other player: pressing
    /// back mid-track almost always means "start this again", and only a quick
    /// double-press means "the one before".
    pub fn previous(&mut self) {
        // Stepping back reaches into history, which can hold tracks that were
        // allowed when they played but are not now -- anything queued before
        // the mode was armed. Prune before stepping rather than refusing at the
        // end, so Previous walks to the most recent *permitted* track instead
        // of dead-ending.
        //
        // The same for tracks no longer in the library (a removed folder),
        // including the entry kept for a removed track while it finished: once
        // something else is playing it is just history. The current entry
        // stays, as the point Previous steps back from.
        let current = self.now_playing.as_ref().map(|n| n.id.clone());
        let played = std::mem::take(&mut self.played);
        self.played =
            played.into_iter().filter(|i| Some(&i.id) == current.as_ref() || self.may_replay(&i.track)).collect();

        // A track whose folder was removed is let finish, not restarted: back
        // goes to the one before it instead, wherever it is in the track.
        let removed = self.now_playing.as_ref().is_some_and(|n| self.library.get(&n.track.id).is_none());
        let position = if removed { 0.0 } else { self.position };
        match step_back(&self.played, position) {
            BackStep::Restart => {
                let Some(np) = &self.now_playing else { return };
                let track = np.track.clone();
                if removed {
                    return;
                }
                if let Some(reason) = self.blocked_reason(&track) {
                    self.notify(format!("{reason} \u{2014} skipping"));
                    self.cut_to_next();
                    return;
                }
                self.counted = false;
                self.position = 0.0;
                self.emit_play(track);
            }
            BackStep::Back { prior, played } => {
                self.credit_skip_if_unplayed();
                self.played = played;
                // Put the current track back at the head so forward still works
                // -- unless it may not play again.
                if let Some(np) = self.now_playing.take()
                    && self.may_replay(&np.track)
                {
                    self.queue.insert(0, np);
                }
                self.now_playing = Some(prior.clone());
                self.counted = false;
                self.position = 0.0;
                self.emit_queue();
                let write = self.config.get().overlay.write_now_playing_json;
                self.np.set_now_playing(Some(prior.clone()), write);
                self.emit(CoreEvent::NowPlaying(Some(prior.clone())));
                self.emit_play(prior.track);
            }
        }
    }

    /// The audio engine reports the current track finished.
    pub(crate) fn on_track_ended(&mut self) {
        let mode = self.config.get().playback.repeat;
        if mode == RepeatMode::One
            && let Some(np) = &self.now_playing
            && self.may_replay(&np.track)
        {
            let track = np.track.clone();
            self.counted = false;
            self.position = 0.0;
            self.emit_play(track);
            return;
        }
        if mode == RepeatMode::All && self.queue.is_empty() && !self.played.is_empty() {
            // Re-queue what has been played, in order, and carry on.
            let (queue, played) = wrap_for_repeat_all(&self.played, self.now_playing.as_ref());
            // Re-queuing a lap replays what was played earlier in the session,
            // which may predate the mode being armed.
            // Removed folders' tracks stay in history until then, too.
            self.queue = queue.into_iter().filter(|i| self.may_replay(&i.track)).map(|i| new_item(i.track)).collect();
            self.played = played;
            self.emit_queue();
        }
        self.play_next();
    }

    /// Makes `item` the current track and tells everyone.
    ///
    /// The bookkeeping play_next does around a selection, without the
    /// queue-shifting: credit a skip for whatever is being abandoned, reset the
    /// play counter, remember the track so shuffle avoids it, and push it onto
    /// the back stack.
    fn start_playing(&mut self, item: QueueItem) {
        self.credit_skip_if_unplayed();
        self.counted = false;
        self.position = 0.0;
        self.now_playing = Some(item.clone());
        self.remember(&item.track);
        self.played = push_played(&self.played, item.clone());
        let write = self.config.get().overlay.write_now_playing_json;
        self.np.set_now_playing(Some(item.clone()), write);
        self.emit(CoreEvent::NowPlaying(Some(item.clone())));
        self.emit_play(item.track);
    }

    /// Plays a specific track immediately, leaving the pending queue intact.
    ///
    /// The Electron renderer once expressed this as enqueue-then-skip, which
    /// cannot work: enqueue appends to the tail and skip shifts the head, so
    /// the user got whatever was already queued and their choice was buried at
    /// the back. With endless on and an empty queue it was worse -- the
    /// enqueue started the right track, the top-up stuffed the queue in front
    /// of it, and the skip immediately abandoned it for filler.
    ///
    /// Deliberately does not consume the queue: "play this" is an
    /// interruption, and what was lined up should still be lined up afterwards.
    pub fn play_now(&mut self, track_id: &str) -> Option<QueueItem> {
        let track = self.library.get(track_id)?.clone();
        if let Some(reason) = self.blocked_reason(&track) {
            self.notify(reason);
            return None;
        }
        let item = new_item(track);
        self.start_playing(item.clone());
        self.top_up_endless();
        Some(item)
    }

    /// Lifts an item out of the pending queue and plays it.
    fn play_queue_item(&mut self, item_id: &str) {
        let Some(idx) = self.queue.iter().position(|i| i.id == item_id) else { return };
        let item = self.queue.remove(idx);
        self.start_playing(item);
        self.emit_queue();
        self.top_up_endless();
    }

    /// Appends several tracks as one operation; `start_now` begins playback on
    /// the batch's first track.
    ///
    /// One enqueue per track is not equivalent. enqueue starts playback as
    /// soon as the first one lands, and play_next tops the queue up before
    /// drawing from it -- so queueing a twelve-track album appended track 1,
    /// started it, saw a queue of one against a lookahead of ten, and filled
    /// the gap with nine unrelated tracks. The rest of the album arrived behind
    /// them, and the user heard track 1, nine tracks from elsewhere in the
    /// library, then track 2.
    ///
    /// Building the whole batch before anything plays leaves the top-up nothing
    /// to do. That matters more than tidiness here: the point of a curated
    /// library is knowing what goes out on stream, and this put unvetted tracks
    /// on air without anyone choosing them.
    pub fn enqueue_many<S: AsRef<str>>(&mut self, track_ids: &[S], start_now: bool) -> Vec<QueueItem> {
        let mut items = Vec::new();
        let mut blocked = 0;
        for id in track_ids {
            let Some(track) = self.library.get(id.as_ref()).cloned() else { continue };
            if self.blocked_reason(&track).is_some() {
                blocked += 1;
                continue;
            }
            items.push(new_item(track));
        }
        // One message for the batch, not one per track: queueing a blocked
        // album would otherwise fire a notice for every track on it.
        if blocked > 0 {
            self.notify(if items.is_empty() {
                format!("Streamer mode: nothing queued \u{2014} {blocked} track{} not marked safe", plural(blocked))
            } else {
                format!("Streamer mode: skipped {blocked} track{} not marked safe", plural(blocked))
            });
        }
        if items.is_empty() {
            return items;
        }

        self.queue.extend(items.iter().cloned());
        self.emit_queue();

        if start_now {
            // Start on the batch's own first track, not on whatever was already
            // queued ahead of it -- "play album" means play this album.
            self.play_queue_item(&items[0].id);
        } else if self.now_playing.is_none() {
            self.play_next();
        }
        items
    }
}
