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

    /// Pumps until `done` holds, or `limit` passes. For waits on a fade to
    /// finish, which a loaded CI machine can stretch past its nominal length.
    fn pump_until(&mut self, limit: Duration, done: impl Fn(&Self) -> bool) {
        let end = Instant::now() + limit;
        while !done(self) && Instant::now() < end {
            self.pump(Duration::from_millis(20));
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

/// Several requests within one fade: the engine still has the first track
/// fading out. Revoking it cuts it, however many requests came after.
#[test]
fn revoking_the_real_outgoing_track_after_several_requests_cuts_it() {
    for chain in [&["s1", "s2"][..], &["s1", "s2", "s3"][..]] {
        let mut l = Live::new(2.0);
        l.start("r1", &[]);
        l.core(|b| b.arm(None));
        for id in chain {
            l.core(|b| b.play_now(id));
            l.pump(Duration::from_millis(50));
        }
        assert!(l.heard("r1"), "r1 is what is fading out");
        let t = Instant::now();
        l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
        l.assert_cut("r1", chain.last().unwrap(), t);
    }
}

/// Paused during the fade: the engine loads the incoming track paused and
/// says so, which ends the fade. A revocation then has nothing to cut and
/// nothing is heard.
#[test]
fn a_pause_during_the_fade_ends_it_and_nothing_is_heard_after() {
    let mut l = Live::new(2.0);
    l.start("r1", &[]);
    l.core(|b| b.arm(None));
    l.core(|b| b.play_now("s1"));
    l.pump(Duration::from_millis(80));
    reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Pause));
    l.pump(Duration::from_millis(150));
    let t = Instant::now();
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.pump(Duration::from_millis(400));
    assert!(l.heard_since(t).is_empty(), "{:?}", l.heard_since(t));
    assert!(!l.fx.player.position().playing);
}

/// A seek during the fade, then the outgoing track revoked: cut, and the
/// incoming one starts where the seek put it.
#[test]
fn a_seek_during_the_fade_survives_a_cut() {
    let mut l = Live::new(2.0);
    l.start("r1", &[]);
    l.core(|b| b.arm(None));
    l.core(|b| b.play_now("s1"));
    l.pump(Duration::from_millis(80));
    reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Seek(1.5)));
    l.pump(Duration::from_millis(50));
    let t = Instant::now();
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.assert_cut("r1", "s1", t);
    let first = l.reports.iter().find_map(|(at, e)| match e {
        PlayerEvent::Progress { track_id: Some(id), position, .. } if *at >= t && id == "s1" => Some(*position),
        _ => None,
    });
    assert!(first.is_some_and(|p| p >= 1.5), "{first:?}");
}

/// Once the fade has finished, revoking the track that faded out changes
/// nothing: the safe track plays on, uninterrupted.
#[test]
fn revoking_a_track_that_already_faded_out_leaves_playback_alone() {
    let mut l = Live::new(0.3);
    l.start("r1", &[]);
    l.core(|b| b.arm(None));
    l.core(|b| b.play_now("s1"));
    l.pump(Duration::from_millis(600));
    assert!(l.heard("s1"));
    let gen_before = l.ui.generation;
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.pump(Duration::from_millis(200));
    assert_eq!(l.ui.generation, gen_before, "no new request to the engine");
    assert!(l.heard("s1"));
}

/// Paused, then the track's album is revoked: the next safe track is
/// loaded, and stays paused.
#[test]
fn a_revocation_while_paused_stays_paused() {
    let mut l = Live::new(2.0);
    l.start("r1", &["s1"]);
    l.core(|b| b.arm(None));
    reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Pause));
    l.pump(Duration::from_millis(100));
    let t = Instant::now();
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.pump(Duration::from_millis(400));
    assert!(l.heard_since(t).is_empty(), "nothing plays: {:?}", l.heard_since(t));
    assert!(l.fx.player.is_paused());
    assert_eq!(l.fx.with(|b| b.now_playing().map(|n| n.track.id.clone())).as_deref(), Some("s1"));
    // Play resumes the replacement.
    reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Play));
    l.pump(Duration::from_millis(300));
    assert!(l.heard("s1"));
}

/// Paused from the window (Space, the play button), the tray or a media
/// key -- none of which goes through the reducer -- during a fade onto a
/// file that cannot open: what replaces it waits, paused. The reviewer's
/// reproduction, and the same through each other way of pausing.
#[test]
fn a_pause_from_anywhere_survives_a_failed_incoming_track() {
    type Pause = fn(&mut Live);
    let ways: [(&str, Pause); 4] = [
        ("window", |l| crate::model::Backend::toggle_play(&l.fx)),
        ("core transport", |l| {
            reduce(&mut l.ui, &l.fx, AppEvent::Core(CoreEvent::Transport(bard_core::types::TransportAction::Toggle)))
        }),
        ("control toggle", |l| reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Toggle))),
        ("control pause", |l| reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Pause))),
    ];
    for (how, pause) in ways {
        let mut l = Live::new(2.0);
        l.core(|b| {
            let mut bad = track("s2", SAFE);
            bad.path = "/definitely-missing-bard-review.wav".into();
            b.install_library(vec![track("s1", SAFE), bad, track("s3", SAFE)]);
        });
        l.start("s1", &["s2", "s3"]);
        l.core(|b| b.skip());
        l.pump(Duration::from_millis(80));
        pause(&mut l);
        let t = Instant::now();
        l.pump(Duration::from_millis(500));
        assert_eq!(l.fx.with(|b| b.now_playing().map(|n| n.track.id.clone())).as_deref(), Some("s3"), "{how}");
        assert!(!l.fx.player.position().playing, "{how}: pause lost across the failed incoming track");
        assert!(l.heard_since(t).is_empty(), "{how}: {:?}", l.heard_since(t));

        // A resume lets the replacement play, and clears the pause: a later
        // failure does not hold what follows it.
        crate::model::Backend::toggle_play(&l.fx);
        l.pump(Duration::from_millis(300));
        assert!(l.heard("s3"), "{how}: resumed");
        assert_eq!(l.fx.player.held(), None, "{how}");
    }
}

impl Live {
    /// A rescan of the library, with some tracks retagged into another album
    /// at the same path (so the same id).
    fn rescan(&mut self, retagged: &[(&str, &str)]) {
        let ids = [("s1", SAFE), ("s2", SAFE), ("s3", SAFE), ("r1", RAIN), ("r2", RAIN)];
        let tracks = ids
            .iter()
            .map(|(id, album)| track(id, retagged.iter().find(|(r, _)| r == id).map_or(album, |(_, a)| a)))
            .collect();
        self.core(|b| b.install_library(tracks));
    }
}

/// The reviewer's reproduction: a rescan that moves the playing track into a
/// claimed album takes it off air at once, as flagging the album would.
#[test]
fn review_rescan_rechecks_current_album_safety() {
    let mut l = Live::new(2.0);
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.start("s1", &["s2"]);
    l.core(|b| b.arm(None));
    l.core(|b| {
        let mut retagged = track("s1", SAFE);
        retagged.album = RAIN.into(); // same track ID/path; changed embedded album tag
        b.install_library(vec![retagged, track("s2", SAFE)]);
    });
    l.pump(Duration::from_millis(400));
    assert!(!l.heard("s1"), "rescan changed the current album to Claimed but it remains audible while armed");
    assert_eq!(l.fx.with(|b| b.now_playing().map(|n| n.track.id.clone())).as_deref(), Some("s2"));
}

/// Retagged into a claimed or an unchecked album while armed: cut at once,
/// despite the crossfade, to the next safe track; and dropped from the
/// queue when it is only queued.
#[test]
fn a_retagged_track_is_judged_by_its_new_album() {
    for (album, flag) in [(RAIN, Some(ContentIdStatus::Claimed)), ("Moon", None)] {
        let mut l = Live::new(2.0);
        if let Some(f) = flag {
            l.core(|b| b.set_album_flag(album, f, None));
        }
        l.start("s1", &["s2", "s3"]);
        l.core(|b| b.arm(None));
        let t = Instant::now();
        l.rescan(&[("s1", album)]);
        l.assert_cut("s1", "s2", t);

        l.rescan(&[("s1", album), ("s3", album)]);
        let queued: Vec<String> = l.fx.with(|b| b.queue().iter().map(|i| i.track.id.clone()).collect());
        assert!(!queued.contains(&"s3".to_string()), "{album}: dropped from the queue: {queued:?}");
        assert!(l.heard("s2"), "{album}: the safe track plays on");
    }
}

/// Nothing safe to follow: silence. Paused: the replacement waits paused.
/// Fading out under a safe track: the fade ends. A rescan that changes
/// nothing interrupts nothing.
#[test]
fn a_retag_while_paused_fading_or_alone_is_handled_like_a_flag() {
    // Alone.
    let mut l = Live::new(2.0);
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.start("s1", &[]);
    l.core(|b| b.arm(None));
    let t = Instant::now();
    l.rescan(&[("s1", RAIN)]);
    l.pump(Duration::from_millis(300));
    assert!(l.heard_since(t).is_empty(), "silence: {:?}", l.heard_since(t));
    assert!(l.fx.with(|b| b.now_playing().is_none()));

    // Paused.
    let mut l = Live::new(2.0);
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.start("s1", &["s2"]);
    l.core(|b| b.arm(None));
    reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Pause));
    l.pump(Duration::from_millis(100));
    let t = Instant::now();
    l.rescan(&[("s1", RAIN)]);
    l.pump(Duration::from_millis(300));
    assert!(l.heard_since(t).is_empty(), "stays paused: {:?}", l.heard_since(t));
    assert_eq!(l.fx.with(|b| b.now_playing().map(|n| n.track.id.clone())).as_deref(), Some("s2"));
    assert!(l.fx.player.is_paused());

    // Fading out under a safe track.
    let mut l = Live::new(2.0);
    l.core(|b| b.set_album_flag(RAIN, ContentIdStatus::Claimed, None));
    l.start("s1", &["s2"]);
    l.core(|b| b.arm(None));
    l.core(|b| b.skip());
    l.pump(Duration::from_millis(100));
    assert!(l.heard("s1"), "s1 fading out");
    let t = Instant::now();
    l.rescan(&[("s1", RAIN)]);
    l.assert_cut("s1", "s2", t);

    // Unchanged.
    let mut l = Live::new(2.0);
    l.start("s1", &["s2"]);
    l.core(|b| b.arm(None));
    let generation = l.ui.generation;
    let t = Instant::now();
    l.rescan(&[]);
    l.pump(Duration::from_millis(300));
    assert_eq!(l.heard_since(t), ["s1"], "uninterrupted");
    assert_eq!(l.ui.generation, generation, "no new request");
}

impl Live {
    /// s1 and s2 with these ReplayGain track and album gains (dB).
    fn gains(&mut self, s1: f64, s2: f64) {
        self.core(|b| {
            let with = |id: &str, db: f64| {
                let mut t = track(id, SAFE);
                t.track_gain_db = Some(db);
                t.album_gain_db = Some(db);
                t
            };
            b.install_library(vec![with("s1", s1), with("s2", s2)]);
        });
    }

    fn normalize(&mut self, mode: bard_core::types::NormalizeMode) {
        self.core(|b| b.update_config(|c| c.audio.normalize = mode));
    }

    /// The loudest gain over `d`: what the fade-out reaches.
    fn peak_gain(&mut self, d: Duration) -> f32 {
        let end = Instant::now() + d;
        let mut most = 0f32;
        while Instant::now() < end {
            self.pump(Duration::from_millis(10));
            most = most.max(self.fx.player.current_gain());
        }
        most
    }
}

/// The reviewer's reproduction: normalisation turned off during a fade
/// reaches the incoming track, which used to start at the multiplier
/// captured when it was asked for.
#[test]
fn review_normalization_change_survives_pending_fade() {
    let mut l = Live::new(0.3);
    l.core(|b| {
        let mut incoming = track("s2", SAFE);
        incoming.track_gain_db = Some(-20.0);
        b.install_library(vec![track("s1", SAFE), incoming]);
    });
    l.start("s1", &["s2"]);
    l.core(|b| b.skip());
    l.pump(Duration::from_millis(80));
    l.core(|b| {
        b.update_config(|c| c.audio.normalize = bard_core::types::NormalizeMode::Off);
    });
    l.pump(Duration::from_millis(850));
    assert!(l.heard("s2"));
    let gain = l.fx.player.current_gain();
    assert!(gain > 0.9, "normalization is Off but pending track restored old multiplier: {gain}");
}

/// Changed during a fade, the setting reaches the incoming track -- off to
/// on and on to off, track and album -- and leaves the outgoing one at its
/// own level; through a pause mid-fade and a cut too.
#[test]
fn a_normalisation_change_during_a_fade_belongs_to_the_incoming_track() {
    use bard_core::types::NormalizeMode::{Album, Off, Track};
    // (from, to, s1 dB, s2 dB, s2's level after)
    for (from, to, s1, s2, want) in
        [(Track, Off, -20., -20., 1.0), (Off, Track, 0., -20., 0.1), (Off, Album, 0., -20., 0.1)]
    {
        let mut l = Live::new(1.0);
        l.normalize(from);
        l.gains(s1, s2);
        l.start("s1", &["s2"]);
        let outgoing = l.fx.player.current_gain();
        l.core(|b| b.skip());
        l.pump(Duration::from_millis(80));
        l.normalize(to);
        let during = l.peak_gain(Duration::from_millis(300));
        assert!(
            during <= outgoing + 0.01,
            "{from:?}->{to:?}: the outgoing track keeps its level: {during} > {outgoing}"
        );
        // The fade takes 2 s from the skip: 1 s out, 1 s in.
        l.pump(Duration::from_millis(1700));
        l.pump_until(Duration::from_secs(3), |l| (l.fx.player.current_gain() - want).abs() < 0.02);
        assert!(l.heard("s2"));
        let g = l.fx.player.current_gain();
        assert!((g - want).abs() < 0.02, "{from:?}->{to:?}: s2 at {g}, want {want}");
    }

    // Paused mid-fade after the change: resumed at the new level.
    let mut l = Live::new(1.0);
    l.normalize(Off);
    l.gains(0., -20.);
    l.start("s1", &["s2"]);
    l.core(|b| b.skip());
    l.pump(Duration::from_millis(80));
    l.normalize(Track);
    reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Pause));
    l.pump(Duration::from_millis(100));
    reduce(&mut l.ui, &l.fx, AppEvent::Control(super::Control::Play));
    l.pump(Duration::from_millis(400));
    assert!(l.heard("s2"));
    assert!((l.fx.player.current_gain() - 0.1).abs() < 0.02, "paused: {}", l.fx.player.current_gain());

    // Cut short (streamer mode) after the change: straight in at the new level.
    let mut l = Live::new(1.0);
    l.normalize(Off);
    l.gains(0., -20.);
    l.start("s1", &["s2"]);
    l.core(|b| b.skip());
    l.pump(Duration::from_millis(80));
    l.normalize(Track);
    l.fx.player.cut_fade();
    l.pump(Duration::from_millis(300));
    assert!(l.heard("s2"));
    assert!((l.fx.player.current_gain() - 0.1).abs() < 0.02, "cut: {}", l.fx.player.current_gain());
}

/// The reviewer's reproduction: a rescan that retitles the playing and the
/// queued track reaches the window's now-playing and queue.
#[test]
fn review_rescan_updates_current_and_queued_ui_metadata() {
    let mut l = Live::new(0.0);
    l.start("s1", &["s2"]);
    l.core(|b| {
        let mut current = track("s1", SAFE);
        current.title = "Updated current title".into();
        let mut queued = track("s2", SAFE);
        queued.title = "Updated queued title".into();
        b.install_library(vec![current, queued]);
    });
    l.pump(Duration::from_millis(350));
    assert_eq!(l.fx.with(|b| b.now_playing().unwrap().track.title.clone()), "Updated current title");
    assert_eq!(l.fx.with(|b| b.queue()[0].track.title.clone()), "Updated queued title");
    assert_eq!(
        (l.ui.track.as_ref().unwrap().title.as_str(), l.ui.queue[0].track.title.as_str()),
        ("Updated current title", "Updated queued title"),
        "current and queued UI snapshots stayed stale"
    );
}

/// Title, album, duration and art all reach the window, without a new
/// request, a restart, a moved position or a reordered queue; an unchanged
/// rescan sends nothing at all.
#[test]
fn a_rescan_refreshes_the_window_without_touching_playback() {
    let mut l = Live::new(0.0);
    l.start("s1", &["s2", "s3"]);
    l.pump(Duration::from_millis(300));
    let (generation, queue_ids): (u64, Vec<String>) =
        (l.ui.generation, l.ui.queue.iter().map(|q| q.id.clone()).collect());
    let position = l.ui.position;
    assert!(position > 0.2, "{position}");

    // Unchanged: no NowPlaying or QueueChanged. (Installed directly, not
    // through `core`, which would hand the events to the reducer first.)
    let same = ["s1", "s2", "s3", "r1", "r2"].iter().zip([SAFE, SAFE, SAFE, RAIN, RAIN]);
    let same: Vec<Track> = same.map(|(id, album)| track(id, album)).collect();
    l.fx.with(|b| b.install_library(same));
    let quiet: Vec<_> = l.core_rx.try_iter().collect();
    assert!(quiet.iter().any(|e| matches!(e, CoreEvent::LibraryChanged)), "the events are here: {quiet:?}");
    assert!(
        !quiet.iter().any(|e| matches!(e, CoreEvent::NowPlaying(_) | CoreEvent::QueueChanged(_))),
        "an unchanged rescan sends nothing: {quiet:?}"
    );

    let mut s1 = track("s1", SAFE);
    s1.title = "Retitled".into();
    s1.album = "Renamed".into();
    s1.duration_sec = 999.;
    s1.has_artwork = !s1.has_artwork;
    let mut s3 = track("s3", SAFE);
    s3.title = "Queued retitled".into();
    l.core(|b| b.set_album_flag("Renamed", ContentIdStatus::Safe, None));
    l.core(|b| b.install_library(vec![s1.clone(), track("s2", SAFE), s3]));
    l.pump(Duration::from_millis(200));
    let t = l.ui.track.clone().unwrap();
    assert_eq!(
        (t.title.as_str(), t.album.as_str(), t.duration_sec, t.has_artwork),
        ("Retitled", "Renamed", 999., s1.has_artwork)
    );
    assert_eq!(l.ui.queue[1].track.title, "Queued retitled");
    assert_eq!(l.ui.queue.iter().map(|q| q.id.clone()).collect::<Vec<_>>(), queue_ids, "same items, same order");
    assert_eq!(l.ui.generation, generation, "no new request");
    assert!(l.heard("s1") && l.ui.position >= position, "playing on from where it was: {}", l.ui.position);
}
