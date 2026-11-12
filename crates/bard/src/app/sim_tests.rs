//! Generated playback sequences against the real core and reducer, with
//! invariants checked after every step.
//!
//! The reviews kept finding stale state that only a *sequence* exposes --
//! A -> B -> C inside one fade, remove -> finish -> Previous, volume during a
//! fade -- each passing its own focused test. Here seeded random sequences of
//! legal actions (transport, queue, repeat, crossfade, arm and revoke,
//! folder removal and rescan, open failures, stop) run against `Bard` and
//! the reducer, and after each step the state is checked against what the
//! test itself knows -- which albums are safe, which folders exist, whether
//! the user paused -- not against core's own answers.
//!
//! The engine is simulated, on virtual time: its documented contract (one
//! source; a fade-out keeps the old source audible with the newest request
//! pending; pause mid-fade loads the incoming one paused; cut and stop end a
//! fade; every report carries its request's generation; a track ends when
//! its time is up) and nothing of core's decisions. Time, and when the
//! engine's reports reach the reducer, are steps of the sequence, so late
//! acknowledgements are generated too, and nothing sleeps.
//!
//! A failure prints the seed and a minimised sequence. A small seed set runs
//! with the ordinary tests; `BARD_SIM_SEEDS=5000 cargo test -p bard sweep --
//! --ignored` runs a larger sweep. Audio-level invariants (zero volume is
//! silent) are checked against the real engine at the end of this file.

use super::AppEvent;
use super::Control;
use super::reduce::{Effects, reduce};
use crate::model::{NullBackend, Ui};
use bard_audio::PlayerEvent;
use bard_core::types::*;
use bard_core::{Bard, CoreEvent, Options};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

// --- the library -------------------------------------------------------------

/// (id, album, root). `x1` cannot be opened.
const TRACKS: &[(&str, &str, &str)] = &[
    ("s1", "Safe", "a"),
    ("s2", "Safe", "a"),
    ("r1", "Rain", "a"),
    ("s3", "Safe", "b"),
    ("r2", "Rain", "b"),
    ("r3", "Rain", "b"),
    ("m1", "Moon", "b"),
    ("m2", "Moon", "c"),
    ("x1", "Moon", "c"),
];
const ALBUMS: &[&str] = &["Safe", "Rain", "Moon"];
const ROOTS: &[&str] = &["a", "b", "c"];

fn track(i: usize) -> Track {
    let (id, album, root) = TRACKS[i];
    Track {
        id: id.into(),
        path: format!("/music/{root}/{id}.flac"),
        title: id.into(),
        artist: "Artist".into(),
        album: album.into(),
        // Distinct, so a report applied to the wrong track shows.
        duration_sec: 20.0 + 7.0 * i as f64,
        has_artwork: false,
        search_text: id.into(),
        track_gain_db: None,
        album_gain_db: None,
        track_peak: None,
        album_peak: None,
    }
}

fn root_of(path: &str) -> &str {
    path.trim_start_matches("/music/").split('/').next().unwrap_or("")
}

// --- the simulated engine ----------------------------------------------------

#[derive(Clone, Debug)]
struct Source {
    track: Track,
    generation: u64,
    pos_ms: u64,
    ended: bool,
}

#[derive(Default)]
struct Engine {
    now_ms: u64,
    next_generation: u64,
    source: Option<Source>,
    playing: bool,
    /// The newest request waiting on a fade-out: (track, generation, at, seek).
    pending: Option<(Track, u64, u64, u64)>,
    /// Reports not yet delivered to the reducer.
    out: VecDeque<PlayerEvent>,
    next_report_ms: u64,
    /// Every play request: (track, started) -- for the invariants.
    requests: Vec<(Track, bool)>,
    /// Request generation, and its track (None for a stop).
    latest: (u64, Option<String>),
    /// As `Player::held`.
    held: u64,
}

impl Engine {
    fn report(&mut self) {
        let (track_id, position, duration, generation) = match &self.source {
            Some(s) => (Some(s.track.id.clone()), s.pos_ms as f64 / 1000., s.track.duration_sec, s.generation),
            None => (None, 0., 0., self.latest.0),
        };
        self.out.push_back(PlayerEvent::Progress { position, duration, playing: self.playing, track_id, generation });
    }

    fn swap(&mut self, track: Track, generation: u64, start: bool, seek_ms: u64) {
        if track.id == "x1" {
            self.source = None;
            self.playing = false;
            self.out.push_back(PlayerEvent::Progress {
                position: 0.,
                duration: 0.,
                playing: false,
                track_id: Some(track.id.clone()),
                generation,
            });
            self.out.push_back(PlayerEvent::Failed { generation, track_id: track.id, message: "no such file".into() });
            return;
        }
        let pos_ms = seek_ms.min((track.duration_sec * 1000.) as u64);
        self.source = Some(Source { track, generation, pos_ms, ended: false });
        self.playing = start;
        self.next_report_ms = self.now_ms + 250;
        self.report();
    }

    fn audible(&self) -> bool {
        self.playing && self.source.as_ref().is_some_and(|s| !s.ended)
    }

    fn play(&mut self, track: &Track, crossfade_sec: f32, start: bool) -> u64 {
        self.next_generation += 1;
        let g = self.next_generation;
        self.latest = (g, Some(track.id.clone()));
        self.held = if start { 0 } else { g };
        self.requests.push((track.clone(), start));
        let fade_ms = (crossfade_sec * 1000.) as u64;
        if start && fade_ms > 0 {
            if let Some(p) = &mut self.pending {
                *p = (track.clone(), g, p.2, 0);
                return g;
            }
            if self.audible() {
                self.pending = Some((track.clone(), g, self.now_ms + fade_ms, 0));
                return g;
            }
        }
        self.pending = None;
        self.swap(track.clone(), g, start, 0);
        g
    }

    fn stop(&mut self) -> u64 {
        self.next_generation += 1;
        self.latest = (self.next_generation, None);
        self.held = 0;
        self.pending = None;
        self.source = None;
        self.playing = false;
        self.report();
        self.next_generation
    }

    fn cut_fade(&mut self) {
        if let Some((t, g, _, seek)) = self.pending.take() {
            self.swap(t, g, true, seek);
        }
    }

    fn pause(&mut self) {
        self.held = self.latest.0;
        if let Some((t, g, _, seek)) = self.pending.take() {
            self.swap(t, g, false, seek);
            return;
        }
        self.playing = false;
        if self.source.is_some() {
            self.report();
        }
    }

    fn resume(&mut self) {
        self.held = 0;
        let Some(s) = &mut self.source else { return };
        if s.ended {
            s.pos_ms = 0;
            s.ended = false;
        }
        self.playing = true;
        self.next_report_ms = self.now_ms + 250;
        self.report();
    }

    fn seek(&mut self, secs: f64) {
        let ms = (secs.max(0.) * 1000.) as u64;
        if let Some(p) = &mut self.pending {
            p.3 = ms;
            return;
        }
        if let Some(s) = &mut self.source {
            s.pos_ms = ms.min((s.track.duration_sec * 1000.) as u64);
            s.ended = false;
            self.report();
        }
    }

    fn advance(&mut self, ms: u64) {
        let end = self.now_ms + ms;
        while self.now_ms < end {
            let step = 50.min(end - self.now_ms);
            self.now_ms += step;
            if self.playing
                && let Some(s) = &mut self.source
                && !s.ended
            {
                s.pos_ms += step;
                if s.pos_ms >= (s.track.duration_sec * 1000.) as u64 {
                    s.pos_ms = (s.track.duration_sec * 1000.) as u64;
                    s.ended = true;
                }
            }
            let ran_out = self.source.as_ref().is_some_and(|s| s.ended) && self.playing;
            if let Some((_, _, at, _)) = &self.pending
                && (self.now_ms >= *at || ran_out)
            {
                let (t, g, _, seek) = self.pending.take().expect("checked");
                self.swap(t, g, true, seek);
                continue;
            }
            if ran_out {
                self.playing = false;
                self.report();
                let generation = self.source.as_ref().expect("ran out").generation;
                self.out.push_back(PlayerEvent::Ended { generation });
            } else if self.playing && self.now_ms >= self.next_report_ms {
                self.next_report_ms = self.now_ms + 250;
                self.report();
            }
        }
    }
}

// --- the effects: real core, simulated engine --------------------------------

struct Fx {
    bard: RefCell<Bard>,
    engine: RefCell<Engine>,
}

impl Effects for Fx {
    fn play(&self, t: &Track, _: f32, crossfade_sec: f32) -> u64 {
        self.engine.borrow_mut().play(t, crossfade_sec, true)
    }
    fn stop(&self) -> u64 {
        self.engine.borrow_mut().stop()
    }
    fn cut_fade(&self) {
        self.engine.borrow_mut().cut_fade()
    }
    fn load_paused(&self, t: &Track, _: f32) -> u64 {
        self.engine.borrow_mut().play(t, 0., false)
    }
    fn playing(&self) -> bool {
        let e = self.engine.borrow();
        e.source.is_some() && e.playing
    }
    fn held(&self) -> Option<u64> {
        Some(self.engine.borrow().held).filter(|g| *g != 0)
    }
    fn pause(&self) {
        self.engine.borrow_mut().pause()
    }
    fn seek(&self, s: f64) {
        self.engine.borrow_mut().seek(s)
    }
    fn set_norm_gain(&self, _: f32) {}
    fn set_output(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    /// As the app's: toggle what is loaded, or start the queue. As the
    /// Player's toggle, nothing at all when core has a track but the engine
    /// has nothing loaded (it failed to open, and the report is on its way).
    fn toggle_play(&self) {
        let loaded = self.engine.borrow().source.is_some();
        if loaded || self.bard.borrow().now_playing().is_some() {
            let mut e = self.engine.borrow_mut();
            if !loaded {
            } else if e.playing {
                e.pause()
            } else {
                e.resume()
            }
        } else {
            self.bard.borrow_mut().skip();
        }
    }
    fn skip(&self) {
        self.bard.borrow_mut().skip()
    }
    fn previous(&self) {
        self.bard.borrow_mut().previous()
    }
    fn set_volume(&self, v: f64) {
        self.bard.borrow_mut().adjust_config(|c| c.audio.volume = v);
    }
    fn update_config(&self, f: Box<dyn FnOnce(&mut Config)>) {
        self.bard.borrow_mut().update_config(f);
    }
    fn scan(&self) {}
    fn report_progress(&self, p: PlaybackProgress) {
        self.bard.borrow_mut().report_progress(p)
    }
    fn report_ended(&self) {
        self.bard.borrow_mut().report_ended()
    }
    fn report_failed(&self, give_up: bool) {
        self.bard.borrow_mut().report_failed(give_up)
    }
    fn stats(&self) -> StatsMap {
        self.bard.borrow().stats()
    }
    fn album_flags(&self) -> AlbumFlags {
        self.bard.borrow().album_flags()
    }
    fn loved(&self) -> Vec<String> {
        Vec::new()
    }
    fn tracks(&self) -> Vec<Track> {
        self.bard.borrow().tracks().to_vec()
    }
    fn queue(&self) -> Vec<QueueItem> {
        self.bard.borrow().queue().to_vec()
    }
}

// --- actions -----------------------------------------------------------------

#[derive(Clone, Debug)]
enum Action {
    PlayNow(usize),
    Enqueue(usize),
    Next,
    Previous,
    Toggle,
    /// The sleep timer runs out.
    Sleep,
    /// Space or the play button: straight to the backend, not through the
    /// reducer.
    WindowToggle,
    Pause,
    Play,
    Seek(f64),
    Volume(f64),
    Repeat(RepeatMode),
    Crossfade(f64),
    Arm,
    Disarm,
    Flag(usize, ContentIdStatus),
    RemoveFolder(usize),
    /// Puts a folder back and installs what a scan of the folders finds.
    Rescan(usize),
    ClearQueue,
    /// Virtual milliseconds pass.
    Wait(u64),
    /// The engine's next `n` reports reach the reducer.
    Deliver(usize),
    DeliverAll,
}

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn generate(seed: u64, steps: usize) -> Vec<Action> {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let statuses = [ContentIdStatus::Safe, ContentIdStatus::Claimed, ContentIdStatus::Untested];
    let repeats = [RepeatMode::Off, RepeatMode::One, RepeatMode::All];
    (0..steps)
        .map(|_| match r.below(24) {
            0 | 1 => Action::PlayNow(r.below(TRACKS.len())),
            2 | 3 => Action::Enqueue(r.below(TRACKS.len())),
            4 => Action::Next,
            5 => Action::Previous,
            6 => Action::Toggle,
            7 => Action::Pause,
            8 => Action::Play,
            9 => Action::Seek(r.below(60) as f64),
            10 => Action::Volume([0., 0.4, 1.][r.below(3)]),
            11 => Action::Repeat(repeats[r.below(3)]),
            12 => Action::Crossfade([0., 1., 3.][r.below(3)]),
            13 => Action::Arm,
            14 => Action::Disarm,
            15 | 16 => Action::Flag(r.below(ALBUMS.len()), statuses[r.below(3)]),
            17 => Action::RemoveFolder(r.below(ROOTS.len())),
            18 => Action::Rescan(r.below(ROOTS.len())),
            19 => Action::ClearQueue,
            20 => Action::Wait([50, 400, 1500, 30_000][r.below(4)]),
            21 => Action::WindowToggle,
            22 => Action::Sleep,
            _ => {
                if r.below(2) == 0 {
                    Action::Deliver(1 + r.below(3))
                } else {
                    Action::DeliverAll
                }
            }
        })
        .collect()
}

// --- the world and its invariants --------------------------------------------

/// What the test knows on its own.
struct Oracle {
    armed: bool,
    status: HashMap<&'static str, ContentIdStatus>,
    roots: HashSet<&'static str>,
    /// The user paused, and has not asked to play since.
    paused: bool,
}

struct World {
    _dir: tempfile::TempDir,
    fx: Fx,
    rx: Receiver<CoreEvent>,
    ui: Ui,
    oracle: Oracle,
    /// Requests already checked.
    checked: usize,
}

impl World {
    fn new(seed: u64) -> World {
        let dir = tempfile::tempdir().unwrap();
        let opts = Options {
            stats_debounce: Duration::from_secs(3600),
            albums_debounce: Duration::from_secs(3600),
            loved_debounce: Duration::from_secs(3600),
        };
        let (mut bard, rx) = Bard::open_with(dir.path().to_path_buf(), opts).unwrap();
        bard.seed_rng(seed);
        bard.update_config(|c| {
            c.library_paths = ROOTS.iter().map(|r| format!("/music/{r}")).collect();
            c.overlay.write_now_playing_json = false;
            c.playback.endless = true;
            c.playback.endless_lookahead = 3;
            c.playback.exclude_claimed = false;
        });
        bard.install_library((0..TRACKS.len()).map(track).collect());
        let fx = Fx { bard: RefCell::new(bard), engine: RefCell::new(Engine::default()) };
        let mut w = World {
            _dir: dir,
            fx,
            rx,
            ui: Ui::new(Rc::new(NullBackend), PathBuf::new()),
            oracle: Oracle {
                armed: false,
                status: HashMap::new(),
                roots: ROOTS.iter().copied().collect(),
                paused: false,
            },
            checked: 0,
        };
        w.core();
        w
    }

    /// Core events reach the reducer (they are synchronous in the app).
    fn core(&mut self) {
        while let Ok(e) = self.rx.try_recv() {
            reduce(&mut self.ui, &self.fx, AppEvent::Core(e));
        }
    }

    fn deliver(&mut self, n: usize) {
        for _ in 0..n {
            let Some(e) = self.fx.engine.borrow_mut().out.pop_front() else { return };
            reduce(&mut self.ui, &self.fx, AppEvent::Player(e));
            self.core();
        }
    }

    fn control(&mut self, c: Control) {
        reduce(&mut self.ui, &self.fx, AppEvent::Control(c));
        self.core();
    }

    fn apply(&mut self, a: &Action) {
        // Asking for another track ends a pause -- if one is started: Next
        // with nowhere to go, or a track that is no longer there, leaves
        // the pause as it was.
        let user_plays = matches!(a, Action::PlayNow(_) | Action::Next | Action::Previous);
        let before = self.fx.engine.borrow().requests.len();
        // Nothing loaded: a pause has nothing left to hold, and enqueueing
        // into an idle player starts it.
        let idle = self.fx.bard.borrow().now_playing().is_none() && self.fx.engine.borrow().source.is_none();
        if idle {
            self.oracle.paused = false;
        }
        match *a {
            Action::PlayNow(i) => {
                self.fx.bard.borrow_mut().play_now(TRACKS[i].0);
            }
            Action::Enqueue(i) => {
                self.fx.bard.borrow_mut().enqueue(TRACKS[i].0);
            }
            Action::Next => self.control(Control::Next),
            Action::Previous => self.control(Control::Previous),
            Action::Toggle => {
                self.toggled();
                self.control(Control::Toggle);
            }
            Action::WindowToggle => {
                self.toggled();
                self.fx.toggle_play();
            }
            // A pause, whatever is playing: what follows waits for Play.
            Action::Pause => {
                self.oracle.paused = true;
                self.control(Control::Pause);
            }
            Action::Sleep => {
                self.oracle.paused = true;
                let at = std::time::Instant::now();
                self.ui.sleep_ends_at = Some(at);
                reduce(&mut self.ui, &self.fx, AppEvent::SleepDue(at));
            }
            Action::Play => {
                if !self.fx.playing() {
                    self.toggled();
                }
                self.control(Control::Play)
            }
            Action::Seek(s) => self.control(Control::Seek(s)),
            Action::Volume(v) => self.control(Control::SetVolume(v)),
            Action::Repeat(m) => {
                self.fx.bard.borrow_mut().update_config(|c| c.playback.repeat = m);
            }
            Action::Crossfade(s) => {
                self.fx.bard.borrow_mut().adjust_config(|c| c.audio.crossfade_sec = s);
            }
            Action::Arm => {
                self.oracle.armed = true;
                self.fx.bard.borrow_mut().arm(None);
            }
            Action::Disarm => {
                self.oracle.armed = false;
                self.fx.bard.borrow_mut().disarm();
            }
            Action::Flag(i, st) => {
                self.oracle.status.insert(ALBUMS[i], st);
                self.fx.bard.borrow_mut().set_album_flag(ALBUMS[i], st, None);
            }
            Action::RemoveFolder(i) => {
                self.oracle.roots.remove(ROOTS[i]);
                let keep: Vec<String> =
                    ROOTS.iter().filter(|r| self.oracle.roots.contains(*r)).map(|r| format!("/music/{r}")).collect();
                self.fx.bard.borrow_mut().update_config(|c| c.library_paths = keep);
            }
            Action::Rescan(i) => {
                self.oracle.roots.insert(ROOTS[i]);
                let roots: Vec<String> =
                    ROOTS.iter().filter(|r| self.oracle.roots.contains(*r)).map(|r| format!("/music/{r}")).collect();
                let found: Vec<Track> =
                    (0..TRACKS.len()).map(track).filter(|t| self.oracle.roots.contains(root_of(&t.path))).collect();
                let mut b = self.fx.bard.borrow_mut();
                b.update_config(|c| c.library_paths = roots);
                let scanner = b.scanner();
                b.install_scan(&scanner, found);
            }
            Action::ClearQueue => self.fx.bard.borrow_mut().clear_queue(),
            Action::Wait(ms) => self.fx.engine.borrow_mut().advance(ms),
            Action::Deliver(n) => self.deliver(n),
            Action::DeliverAll => self.deliver(usize::MAX),
        }
        self.core();
        if user_plays && self.fx.engine.borrow().requests[before..].iter().any(|(_, start)| *start) {
            self.oracle.paused = false;
        }
    }

    /// What a toggle means, as the user would see it: pauses what plays,
    /// resumes what is loaded, starts the queue when nothing is -- and does
    /// nothing while the engine has nothing loaded but core has a track.
    fn toggled(&mut self) {
        let e = self.fx.engine.borrow();
        if e.source.is_some() {
            self.oracle.paused = e.playing;
        } else if self.fx.bard.borrow().now_playing().is_none() {
            self.oracle.paused = false;
        }
    }

    fn safe(&self, t: &Track) -> bool {
        self.oracle.status.get(t.album.as_str()) == Some(&ContentIdStatus::Safe)
    }

    fn check(&mut self) -> Result<(), String> {
        let e = self.fx.engine.borrow();
        // Every request made since the last check.
        for (t, start) in &e.requests[self.checked..] {
            if self.oracle.armed && !self.safe(t) {
                return Err(format!("armed, and {} ({}) was sent to the engine", t.id, t.album));
            }
            if !self.oracle.roots.contains(root_of(&t.path)) {
                return Err(format!("{} from removed folder {} was sent to the engine", t.id, root_of(&t.path)));
            }
            if self.oracle.paused && *start {
                return Err(format!("paused by the user, and {} was started", t.id));
            }
        }
        self.checked = e.requests.len();

        // Forbidden audio is never heard, outgoing or not.
        if self.oracle.armed
            && e.audible()
            && let Some(s) = &e.source
            && !self.safe(&s.track)
        {
            return Err(format!("armed, and {} ({}) is audible", s.track.id, s.track.album));
        }
        if self.oracle.paused && e.playing {
            return Err("paused by the user, and the engine is playing".into());
        }
        // And the other way: nothing waits paused unless the user paused.
        if !self.oracle.paused
            && e.out.is_empty()
            && e.pending.is_none()
            && !e.playing
            && let Some(s) = e.source.as_ref().filter(|s| !s.ended)
        {
            return Err(format!("nothing paused it, and {} waits paused", s.track.id));
        }

        // Once every report is in, the model reflects the newest request
        // and nothing older.
        if e.out.is_empty() {
            if self.ui.generation != e.latest.0 {
                return Err(format!("model on generation {}, engine on {}", self.ui.generation, e.latest.0));
            }
            if self.ui.track.as_ref().map(|t| t.id.clone()) != e.latest.1 {
                return Err(format!("model shows {:?}, the newest request is {:?}", self.ui.track, e.latest.1));
            }
            if self.ui.duration != 0. {
                let current = e.source.as_ref().filter(|s| s.generation == self.ui.generation);
                if current.is_none_or(|s| s.track.duration_sec != self.ui.duration) {
                    return Err(format!("model duration {} is not the current track's", self.ui.duration));
                }
            }
        }
        Ok(())
    }
}

/// Runs a sequence; the step that broke an invariant, and how.
fn run(seed: u64, actions: &[Action]) -> Result<(), (usize, String)> {
    let mut w = World::new(seed);
    for (i, a) in actions.iter().enumerate() {
        w.apply(a);
        w.check().map_err(|e| (i, e))?;
    }
    // Drain, so late reports are checked too.
    w.apply(&Action::DeliverAll);
    w.check().map_err(|e| (actions.len(), e))
}

/// Drops actions while it still fails, to the shortest sequence that does.
fn minimise(seed: u64, mut actions: Vec<Action>) -> Vec<Action> {
    if let Err((at, _)) = run(seed, &actions) {
        actions.truncate(at + 1);
    }
    let mut i = 0;
    while i < actions.len() {
        let mut fewer = actions.clone();
        fewer.remove(i);
        if run(seed, &fewer).is_err() {
            actions = fewer;
        } else {
            i += 1;
        }
    }
    actions
}

fn sweep(seeds: std::ops::Range<u64>, steps: usize) {
    for seed in seeds {
        let actions = generate(seed, steps);
        if let Err((at, why)) = run(seed, &actions) {
            let min = minimise(seed, actions);
            let listing: String = min.iter().map(|a| format!("\n  {a:?}")).collect();
            panic!(
                "seed {seed}: step {at}: {why}\nminimised ({} steps):{listing}\n(rerun: run({seed}, &[...]))",
                min.len()
            );
        }
    }
}

#[test]
fn generated_sequences_keep_the_invariants() {
    sweep(0..150, 80);
}

#[test]
#[ignore]
fn sweep_many_seeds() {
    let n = std::env::var("BARD_SIM_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(5000);
    sweep(0..n, 150);
}

/// The sequences the reviews found, as the harness sees them.
#[test]
fn the_review_findings_are_caught_as_sequences() {
    use Action::*;
    use ContentIdStatus::*;
    let fixed = [
        // A -> B -> C inside one fade, then A revoked.
        vec![
            Flag(0, Safe),
            Flag(1, Safe),
            Flag(2, Safe),
            Arm,
            PlayNow(2),
            DeliverAll,
            Crossfade(3.),
            PlayNow(0),
            PlayNow(1),
            Flag(1, Claimed),
            Wait(50),
        ],
        // remove the current track's folder, let it finish, then Previous.
        vec![PlayNow(7), DeliverAll, Enqueue(0), RemoveFolder(2), Wait(30_000), DeliverAll, Previous, DeliverAll],
        // paused, then the current album revoked.
        vec![
            Flag(0, Safe),
            Flag(1, Safe),
            Arm,
            PlayNow(2),
            Enqueue(0),
            DeliverAll,
            Pause,
            Flag(1, Claimed),
            Wait(400),
            DeliverAll,
        ],
        // the track runs out, and is revoked before its end is reported:
        // nobody paused, so the replacement plays.
        vec![
            Flag(0, Safe),
            Flag(1, Safe),
            Arm,
            PlayNow(2),
            ClearQueue,
            Enqueue(0),
            DeliverAll,
            Wait(30_000),
            Wait(30_000),
            Flag(1, Claimed),
            DeliverAll,
            Wait(1500),
            DeliverAll,
        ],
        // paused, revoked onto a file that cannot open.
        vec![
            Flag(0, Safe),
            Flag(1, Safe),
            Flag(2, Safe),
            Arm,
            PlayNow(2),
            ClearQueue,
            Enqueue(8),
            Enqueue(0),
            DeliverAll,
            Pause,
            Flag(1, Claimed),
            DeliverAll,
        ],
    ];
    for (i, seq) in fixed.iter().enumerate() {
        if let Err((at, why)) = run(1, seq) {
            panic!("finding {i}: step {at}: {why}");
        }
    }
}
