//! The core, the reducer and the real audio engine together (null output: no
//! device, no sound), for what only shows up end to end -- chiefly whether a
//! track streamer mode takes off the air actually stops being heard, which
//! the core alone cannot tell: it decides, the engine fades.

use super::backend::AppBackend;
use super::reduce::reduce;
use super::{AppEvent, backend::ScanState};
use crate::model::{NullBackend, Ui};
use bard_audio::{Backend, Player, PlayerEvent, PlayerOptions};
use bard_core::types::{ContentIdStatus, Track};
use bard_core::{Bard, CoreEvent, Options};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SAFE: &str = "Safe";
const RAIN: &str = "Rain";

fn track(id: &str, album: &str) -> Track {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bard-audio/tests/fixtures/bursts.wav");
    Track {
        id: id.into(),
        path: path.to_string_lossy().into_owned(),
        title: format!("Title {id}"),
        artist: "Artist".into(),
        album: album.into(),
        duration_sec: 3.0,
        has_artwork: false,
        search_text: id.into(),
        track_gain_db: None,
        album_gain_db: None,
        track_peak: None,
        album_peak: None,
    }
}

struct Live {
    _dir: tempfile::TempDir,
    fx: AppBackend,
    core_rx: Receiver<CoreEvent>,
    player_rx: Receiver<PlayerEvent>,
    ui: Ui,
    /// Every engine report so far, as it arrived.
    reports: Vec<(Instant, PlayerEvent)>,
}

impl Live {
    /// Safe: s1..s3 and Rain: r1, r2 (flagged safe too, until a test says
    /// otherwise); endless off, so only what a test queues plays.
    fn new(crossfade_sec: f64) -> Live {
        let dir = tempfile::tempdir().unwrap();
        let opts = Options {
            stats_debounce: Duration::from_secs(3600),
            albums_debounce: Duration::from_secs(3600),
            loved_debounce: Duration::from_secs(3600),
        };
        let (mut bard, core_rx) = Bard::open_with(dir.path().to_path_buf(), opts).unwrap();
        bard.update_config(|c| {
            c.playback.endless = false;
            c.playback.auto_fill = bard_core::types::AutoFillMode::Off;
            c.overlay.write_now_playing_json = false;
            c.audio.crossfade_sec = crossfade_sec;
        });
        let ids = [("s1", SAFE), ("s2", SAFE), ("s3", SAFE), ("r1", RAIN), ("r2", RAIN)];
        bard.install_library(ids.iter().map(|(id, album)| track(id, album)).collect());
        bard.set_album_flag(SAFE, ContentIdStatus::Safe, None);
        bard.set_album_flag(RAIN, ContentIdStatus::Safe, None);
        let (player, player_rx) =
            Player::with_options(PlayerOptions { backend: Backend::Null, ..Default::default() }).unwrap();
        let (events, _) = futures::channel::mpsc::unbounded();
        let fx = AppBackend {
            bard: Arc::new(Mutex::new(bard)),
            player: Arc::new(player),
            events,
            services: Arc::new(Mutex::new(None)),
            scans: Arc::new(Mutex::new(ScanState::default())),
        };
        let mut live = Live {
            _dir: dir,
            fx,
            core_rx,
            player_rx,
            ui: Ui::new(Rc::new(NullBackend), PathBuf::new()),
            reports: Vec::new(),
        };
        live.pump(Duration::from_millis(20));
        live
    }

    fn core<R>(&mut self, f: impl FnOnce(&mut Bard) -> R) -> R {
        let r = self.fx.with(f);
        self.pump(Duration::ZERO);
        r
    }

    /// Delivers events as the app's event loop does, for `d`.
    fn pump(&mut self, d: Duration) {
        let end = Instant::now() + d;
        loop {
            while let Ok(e) = self.core_rx.try_recv() {
                reduce(&mut self.ui, &self.fx, AppEvent::Core(e));
            }
            while let Ok(e) = self.player_rx.try_recv() {
                self.reports.push((Instant::now(), e.clone()));
                reduce(&mut self.ui, &self.fx, AppEvent::Player(e));
            }
            if Instant::now() >= end {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Plays `first` with `next` queued behind it, and waits until it is heard.
    fn start(&mut self, first: &str, next: &[&str]) {
        self.core(|b| {
            b.enqueue(first);
            for id in next {
                b.enqueue(id);
            }
        });
        self.pump(Duration::from_millis(250));
        assert!(self.heard(first), "{first} playing: {:?}", self.reports);
    }

    /// Whether the engine's latest report has `id` playing.
    fn heard(&self, id: &str) -> bool {
        self.reports.iter().rev().find_map(|(_, e)| match e {
            PlayerEvent::Progress { track_id, playing, .. } => Some(*playing && track_id.as_deref() == Some(id)),
            _ => None,
        }) == Some(true)
    }

    /// The tracks the engine reported playing after `since`.
    fn heard_since(&self, since: Instant) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for (at, e) in &self.reports {
            if let PlayerEvent::Progress { track_id: Some(t), playing: true, .. } = e
                && *at >= since
                && v.last() != Some(t)
            {
                v.push(t.clone());
            }
        }
        v
    }

    /// The forbidden track is gone from the engine within `within`, and
    /// `then` is what plays.
    fn assert_cut(&mut self, forbidden: &str, then: &str, since: Instant) {
        self.pump(Duration::from_millis(150));
        let after = Instant::now();
        self.pump(Duration::from_millis(250));
        let late = self.heard_since(after);
        assert!(!late.iter().any(|t| t == forbidden), "{forbidden} still heard 150 ms on: {late:?}");
        assert_eq!(late, [then], "then {then} plays: {:?}", self.heard_since(since));
        assert_eq!(self.fx.with(|b| b.now_playing().map(|n| n.track.id.clone())).as_deref(), Some(then));
        let gain = self.fx.player.current_gain();
        assert!(gain > 0.5, "{then} at its level, not fading in: {gain}");
    }
}

#[test]
fn a_track_that_stops_being_safe_goes_off_air_at_once_despite_a_crossfade() {
    for status in [ContentIdStatus::Claimed, ContentIdStatus::Untested] {
        let mut l = Live::new(2.0);
        l.start("r1", &["s1"]);
        l.core(|b| b.arm(None));
        let t = Instant::now();
        l.core(|b| b.set_album_flag(RAIN, status, None));
        l.assert_cut("r1", "s1", t);
    }
}

#[test]
fn arming_over_an_unsafe_track_takes_it_off_air_at_once() {
    let mut l = Live::new(2.0);
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Untested, None));
    l.start("r1", &["s1"]);
    let t = Instant::now();
    l.core(|b| b.arm(None));
    l.assert_cut("r1", "s1", t);
}

/// The outgoing track of a crossfade under way becomes forbidden while the
/// incoming one is safe: the fade-out ends there.
#[test]
fn a_track_fading_out_that_stops_being_safe_is_cut_off() {
    let mut l = Live::new(2.0);
    l.start("r1", &["s1"]);
    l.core(|b| b.arm(None));
    l.core(|b| b.skip());
    l.pump(Duration::from_millis(100));
    assert!(l.heard("r1"), "r1 fading out: {:?}", l.heard_since(Instant::now() - Duration::from_secs(1)));
    let t = Instant::now();
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.assert_cut("r1", "s1", t);
}

/// The incoming track of a crossfade under way becomes forbidden: neither it
/// nor the outgoing one is heard; the next safe track is.
#[test]
fn a_track_fading_in_that_stops_being_safe_never_plays() {
    let mut l = Live::new(2.0);
    l.start("s1", &["r1", "s2"]);
    l.core(|b| b.arm(None));
    l.core(|b| b.skip());
    l.pump(Duration::from_millis(100));
    let t = Instant::now();
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.assert_cut("s1", "s2", t);
    assert!(!l.heard_since(t).iter().any(|id| id == "r1"));
}

/// Nothing safe to follow: silence, at once.
#[test]
fn with_nothing_safe_to_follow_playback_stops_at_once() {
    let mut l = Live::new(2.0);
    l.start("r1", &["r2"]);
    l.core(|b| b.arm(None));
    let t = Instant::now();
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.pump(Duration::from_millis(150));
    let after = Instant::now();
    l.pump(Duration::from_millis(250));
    assert!(l.heard_since(after).is_empty(), "{:?}", l.heard_since(t));
    assert!(!l.fx.player.position().playing);
}

/// The user's own skips still crossfade.
#[test]
fn an_ordinary_skip_still_crossfades() {
    let mut l = Live::new(2.0);
    l.start("s1", &["s2"]);
    l.core(|b| b.arm(None));
    l.core(|b| b.skip());
    l.pump(Duration::from_millis(300));
    assert!(l.heard("s1"), "s1 still fading out");
}
