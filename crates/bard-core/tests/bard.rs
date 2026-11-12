//! The facade, end to end: queue, playback reporting, config side effects,
//! streamer mode and persistence -- the orchestration ipc-handlers.ts and
//! queue.ts did, which the TypeScript had no tests for.

use bard_core::types::*;
use bard_core::{Bard, CoreEvent, Options, Transition, gain_multiplier};
use std::sync::mpsc::Receiver;
use std::time::Duration;

fn track(id: &str, album: &str, n: u32) -> Track {
    Track {
        id: id.into(),
        path: format!("/music/{album}/{n:02} - {id}.flac"),
        title: format!("Title {id}"),
        artist: "Harris Heller".into(),
        album: album.into(),
        duration_sec: 120.0,
        has_artwork: true,
        search_text: bard_core::search::search_text_for(&format!("Title {id}"), "Harris Heller", album),
        track_gain_db: None,
        album_gain_db: None,
        track_peak: None,
        album_peak: None,
    }
}

fn library() -> Vec<Track> {
    let mut v = Vec::new();
    for (album, prefix) in [("Gold", "g"), ("Rain", "r"), ("Moon", "m")] {
        for n in 1..=4 {
            v.push(track(&format!("{prefix}{n}"), album, n));
        }
    }
    v
}

struct Fixture {
    _dir: tempfile::TempDir,
    bard: Bard,
    rx: Receiver<CoreEvent>,
}

impl Fixture {
    fn drain(&self) -> Vec<CoreEvent> {
        self.rx.try_iter().collect()
    }
    fn notices(&self) -> Vec<String> {
        self.drain().into_iter().filter_map(|e| if let CoreEvent::Notice(n) = e { Some(n) } else { None }).collect()
    }
    fn dir(&self) -> &std::path::Path {
        self._dir.path()
    }
}

/// A Bard over the test library, endless off unless asked for.
fn fixture(endless: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let opts = Options {
        stats_debounce: Duration::from_secs(3600),
        albums_debounce: Duration::from_secs(3600),
        loved_debounce: Duration::from_secs(3600),
    };
    let (mut bard, rx) = Bard::open_with(dir.path().to_path_buf(), opts).unwrap();
    bard.seed_rng(7);
    bard.update_config(|c| c.playback.endless = endless);
    bard.install_library(library());
    let f = Fixture { _dir: dir, bard, rx };
    f.drain();
    f
}

fn queue_ids(b: &Bard) -> Vec<String> {
    b.queue().iter().map(|q| q.track.id.clone()).collect()
}

fn playing(b: &Bard) -> Option<String> {
    b.now_playing().map(|n| n.track.id.clone())
}

fn progress(b: &Bard, position: f64) -> PlaybackProgress {
    PlaybackProgress { position, duration: 120.0, playing: true, track_id: b.now_playing().map(|n| n.track.id.clone()) }
}

#[test]
fn bard_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Bard>();
    assert_send::<bard_core::LibraryScanner>();
}

#[test]
fn open_creates_the_data_dir_and_config() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("bard");
    let (bard, _rx) = Bard::open(data.clone()).unwrap();
    assert!(data.join("config.json").is_file());
    let p = bard.paths();
    assert_eq!(p.config, data.join("config.json").to_string_lossy());
    assert_eq!(p.artwork, data.join("artwork").to_string_lossy());
    assert_eq!(p.now_playing, data.join("nowplaying.json").to_string_lossy());
    assert_eq!(p.logs, data.join("logs").to_string_lossy());
    assert!(bard.can_reveal(&data));
    assert!(!bard.can_reveal(dir.path().join("bard-evil")));
}

#[test]
fn enqueue_starts_playback_when_idle() {
    let mut f = fixture(false);
    f.bard.enqueue("g1");
    assert_eq!(playing(&f.bard).as_deref(), Some("g1"));
    assert!(f.bard.queue().is_empty());
    let ev = f.drain();
    assert!(matches!(&ev[0], CoreEvent::QueueChanged(q) if q.len() == 1));
    assert!(ev.iter().any(|e| matches!(e, CoreEvent::PlayTrack(t, _) if t.id == "g1")));
    assert!(ev.iter().any(|e| matches!(e, CoreEvent::NowPlaying(Some(i)) if i.track.id == "g1")));

    f.bard.enqueue("g2");
    assert_eq!(queue_ids(&f.bard), ["g2"]);
    assert!(f.bard.enqueue("nope").is_none());
}

#[test]
fn enqueue_many_keeps_an_album_together_with_endless_on() {
    // The bug enqueue_many exists for: queueing an album track by track let
    // endless fill the gap after track 1 with unrelated tracks.
    let mut f = fixture(true);
    f.bard.clear_queue();
    let album = ["r1", "r2", "r3", "r4"];
    f.bard.enqueue_many(&album, true);
    assert_eq!(playing(&f.bard).as_deref(), Some("r1"));
    assert_eq!(&queue_ids(&f.bard)[..3], ["r2", "r3", "r4"]);
}

#[test]
fn enqueue_many_start_now_plays_the_batch_not_the_head() {
    let mut f = fixture(false);
    f.bard.enqueue("g1"); // playing
    f.bard.enqueue("g2"); // queued
    f.bard.enqueue_many(&["m1", "m2"], true);
    assert_eq!(playing(&f.bard).as_deref(), Some("m1"));
    assert_eq!(queue_ids(&f.bard), ["g2", "m2"]);
}

#[test]
fn play_now_interrupts_without_consuming_the_queue() {
    let mut f = fixture(false);
    f.bard.enqueue_many(&["g1", "g2", "g3"], false);
    assert_eq!(playing(&f.bard).as_deref(), Some("g1"));
    f.bard.play_now("m4");
    assert_eq!(playing(&f.bard).as_deref(), Some("m4"));
    assert_eq!(queue_ids(&f.bard), ["g2", "g3"]);
}

#[test]
fn endless_stocks_the_queue_on_install_without_playing() {
    let f = fixture(true);
    assert_eq!(f.bard.queue().len(), 10);
    assert!(f.bard.now_playing().is_none());
    let unique: std::collections::HashSet<_> = queue_ids(&f.bard).into_iter().collect();
    assert_eq!(unique.len(), 10);
}

#[test]
fn turning_endless_on_tops_up_at_once() {
    let mut f = fixture(false);
    assert!(f.bard.queue().is_empty());
    f.bard.update_config(|c| c.playback.endless = true);
    assert_eq!(f.bard.queue().len(), 10);
    f.bard.update_config(|c| c.playback.endless_lookahead = 11);
    assert_eq!(f.bard.queue().len(), 11);
    let ev = f.drain();
    assert!(ev.iter().any(|e| matches!(e, CoreEvent::ConfigChanged(c) if c.playback.endless_lookahead == 11)));
}

#[test]
fn removing_with_endless_refills() {
    let mut f = fixture(true);
    let first = f.bard.queue()[0].id.clone();
    f.bard.remove_from_queue(&first);
    assert_eq!(f.bard.queue().len(), 10);
    assert!(f.bard.queue().iter().all(|q| q.id != first));
}

#[test]
fn reorder_clamps_and_ignores_bad_indices() {
    let mut f = fixture(false);
    f.bard.enqueue("g1");
    f.bard.enqueue_many(&["g2", "g3", "g4"], false);
    f.bard.reorder_queue(0, 99);
    assert_eq!(queue_ids(&f.bard), ["g3", "g4", "g2"]);
    f.bard.reorder_queue(9, 0);
    assert_eq!(queue_ids(&f.bard), ["g3", "g4", "g2"]);
    f.bard.reorder_queue(2, 0);
    assert_eq!(queue_ids(&f.bard), ["g2", "g3", "g4"]);
    f.bard.shuffle_queue();
    let mut ids = queue_ids(&f.bard);
    ids.sort();
    assert_eq!(ids, ["g2", "g3", "g4"]);
}

#[test]
fn fill_queue_respects_count_room_and_duplicates() {
    let mut f = fixture(false);
    f.bard.update_config(|c| c.playback.max_queue_length = 5);
    assert_eq!(f.bard.fill_queue(Some(3)), 3);
    // Started playback: one moved to now playing.
    assert_eq!(f.bard.queue().len(), 2);
    assert_eq!(f.bard.fill_queue(None), 3); // room for 3 more of the default 10
    assert_eq!(f.bard.fill_queue(None), 0);
    let mut all = queue_ids(&f.bard);
    all.push(playing(&f.bard).unwrap());
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(unique.len(), all.len());
}

#[test]
fn shuffle_auto_fill_keeps_playing_when_the_queue_runs_dry() {
    let mut f = fixture(false);
    f.bard.enqueue("g1");
    f.bard.report_ended();
    assert!(f.bard.now_playing().is_some());
    assert!(f.bard.queue().is_empty(), "not endless: a single track, nothing queued");

    f.bard.update_config(|c| c.playback.auto_fill = AutoFillMode::Off);
    f.bard.report_ended();
    assert!(f.bard.now_playing().is_none());
    assert!(f.drain().iter().any(|e| matches!(e, CoreEvent::NowPlaying(None))));
}

#[test]
fn album_auto_fill_walks_the_album_then_rolls_over() {
    let mut f = fixture(false);
    f.bard.update_config(|c| c.playback.auto_fill = AutoFillMode::Album);
    f.bard.enqueue("g3");
    f.bard.report_ended();
    assert_eq!(playing(&f.bard).as_deref(), Some("g4"));
    f.bard.report_ended();
    let next = playing(&f.bard).unwrap();
    assert!(next == "r1" || next == "m1", "rolled to another album's first track, got {next}");
}

#[test]
fn plays_are_counted_at_30s_and_skips_before_that() {
    let mut f = fixture(false);
    f.bard.enqueue_many(&["g1", "g2", "g3"], false);
    let p = progress(&f.bard, 29.0);
    f.bard.report_progress(p);
    assert_eq!(f.bard.track_stats("g1").plays, 0);
    let p = progress(&f.bard, 30.0);
    f.bard.report_progress(p);
    assert_eq!(f.bard.track_stats("g1").plays, 1);
    // Counted once only.
    let p = progress(&f.bard, 50.0);
    f.bard.report_progress(p);
    assert_eq!(f.bard.track_stats("g1").plays, 1);
    assert_eq!(f.drain().iter().filter(|e| matches!(e, CoreEvent::StatsChanged)).count(), 1);

    f.bard.skip(); // g1 was counted: no skip
    assert_eq!(f.bard.track_stats("g1").skips, 0);
    let p = progress(&f.bard, 5.0);
    f.bard.report_progress(p);
    f.bard.skip(); // g2 abandoned at 5s
    assert_eq!(f.bard.track_stats("g2").skips, 1);
    f.bard.skip(); // g3 at 0s is not a skip
    assert_eq!(f.bard.track_stats("g3").skips, 0);
}

#[test]
fn short_tracks_count_at_half() {
    let mut f = fixture(false);
    f.bard.enqueue("g1");
    f.bard.report_progress(PlaybackProgress {
        position: 21.0,
        duration: 40.0,
        playing: true,
        track_id: Some("g1".into()),
    });
    assert_eq!(f.bard.track_stats("g1").plays, 1);
}

#[test]
fn progress_from_the_outgoing_track_is_ignored() {
    // A crossfade keeps reporting the old track after the new one is selected.
    let mut f = fixture(false);
    f.bard.enqueue_many(&["g1", "g2"], false);
    f.bard.skip();
    f.drain();
    f.bard.report_progress(PlaybackProgress {
        position: 100.0,
        duration: 120.0,
        playing: true,
        track_id: Some("g1".into()),
    });
    assert_eq!(f.bard.track_stats("g2").plays, 0);
    assert!(f.drain().is_empty());
    // Untagged reports are accepted.
    f.bard.report_progress(PlaybackProgress { position: 31.0, duration: 0.0, playing: true, track_id: None });
    assert_eq!(f.bard.track_stats("g2").plays, 1);
}

#[test]
fn previous_restarts_or_steps_back() {
    let mut f = fixture(false);
    f.bard.enqueue_many(&["g1", "g2", "g3"], false);
    f.bard.skip(); // g2
    let p = progress(&f.bard, 10.0);
    f.bard.report_progress(p);
    f.drain();
    f.bard.previous();
    assert_eq!(playing(&f.bard).as_deref(), Some("g2"), "restart");
    assert!(matches!(&f.drain()[..], [CoreEvent::PlayTrack(t, _)] if t.id == "g2"));

    f.bard.previous(); // position reset to 0 by the restart: steps back
    assert_eq!(playing(&f.bard).as_deref(), Some("g1"));
    assert_eq!(queue_ids(&f.bard), ["g2", "g3"], "forward still works");
    f.bard.previous(); // nothing behind g1: restart
    assert_eq!(playing(&f.bard).as_deref(), Some("g1"));
    f.bard.skip();
    assert_eq!(playing(&f.bard).as_deref(), Some("g2"));
}

#[test]
fn repeat_one_and_all() {
    let mut f = fixture(false);
    f.bard.update_config(|c| c.playback.repeat = RepeatMode::One);
    f.bard.enqueue_many(&["g1", "g2"], false);
    f.bard.report_ended();
    assert_eq!(playing(&f.bard).as_deref(), Some("g1"));

    f.bard.update_config(|c| c.playback.repeat = RepeatMode::All);
    f.bard.report_ended(); // g2
    f.bard.report_ended(); // wraps: g1 again, not g2 twice
    assert_eq!(playing(&f.bard).as_deref(), Some("g1"));
    f.bard.report_ended();
    assert_eq!(playing(&f.bard).as_deref(), Some("g2"));
}

#[test]
fn claiming_an_album_drops_it_from_the_queue_and_auto_play() {
    let mut f = fixture(false);
    f.bard.enqueue_many(&["g1", "g2", "r1", "g3"], false);
    f.drain();
    f.bard.set_album_flag("Gold", ContentIdStatus::Claimed, Some(" Content ID "));
    assert_eq!(queue_ids(&f.bard), ["r1"]);
    assert_eq!(f.bard.album_flags()["Gold"].note.as_deref(), Some("Content ID"));
    assert!(f.drain().iter().any(|e| matches!(e, CoreEvent::AlbumFlagsChanged)));
    assert!(f.bard.playable_tracks().iter().all(|t| t.album != "Gold"));
    // Deliberate play is still allowed while not armed.
    assert!(f.bard.play_now("g4").is_some());
}

#[test]
fn every_album_claimed_falls_back_to_everything() {
    let mut f = fixture(false);
    for a in ["Gold", "Rain", "Moon"] {
        f.bard.set_album_flag(a, ContentIdStatus::Claimed, None);
    }
    assert_eq!(f.bard.playable_tracks().len(), 12);
}

#[test]
fn streamer_mode_is_an_allow_list_enforced_on_the_present() {
    let mut f = fixture(false);
    f.bard.set_album_flag("Rain", ContentIdStatus::Safe, None);
    f.bard.set_album_flag("Gold", ContentIdStatus::Claimed, None);
    f.bard.enqueue_many(&["g1", "m1", "r1", "r2"], false);
    assert_eq!(playing(&f.bard).as_deref(), Some("g1"));
    f.drain();

    f.bard.arm(None);
    assert!(f.bard.is_streamer_armed());
    assert_eq!(playing(&f.bard).as_deref(), Some("r1"), "the unsafe track on air was skipped");
    assert_eq!(queue_ids(&f.bard), ["r2"]);
    let notices = f.notices();
    assert_eq!(
        notices,
        [
            "Streamer mode: removed 1 queued track not marked safe",
            "Streamer mode: \"Gold\" is flagged as claimed \u{2014} skipping",
        ]
    );

    // Deliberate play is refused, with a reason.
    assert!(f.bard.play_now("m2").is_none());
    assert_eq!(f.notices(), ["Streamer mode: \"Moon\" has not been checked for Content ID"]);
    assert!(f.bard.enqueue_many(&["m1", "m2", "g1"], false).is_empty());
    assert_eq!(f.notices(), ["Streamer mode: nothing queued \u{2014} 3 tracks not marked safe"]);
    assert_eq!(f.bard.enqueue_many(&["m1", "r3"], false).len(), 1);
    assert_eq!(f.notices(), ["Streamer mode: skipped 1 track not marked safe"]);

    // Auto-play only reaches safe albums.
    for _ in 0..10 {
        f.bard.report_ended();
        if let Some(id) = playing(&f.bard) {
            assert!(id.starts_with('r'), "{id}");
        }
    }
    f.bard.disarm();
    assert!(!f.bard.is_streamer_armed());
}

#[test]
fn armed_with_nothing_safe_stays_silent() {
    // The TypeScript's shuffle fallback reached past the allow-list here.
    let mut f = fixture(false);
    f.bard.arm(None);
    f.bard.fill_queue(Some(5));
    assert!(f.bard.queue().is_empty());
    f.bard.report_ended();
    assert!(f.bard.now_playing().is_none());
    f.bard.update_config(|c| c.playback.endless = true);
    assert!(f.bard.queue().is_empty());
}

#[test]
fn previous_skips_history_that_is_no_longer_permitted() {
    let mut f = fixture(false);
    f.bard.set_album_flag("Rain", ContentIdStatus::Safe, None);
    f.bard.enqueue_many(&["r1", "g1", "r2"], false);
    f.bard.skip(); // g1
    f.bard.arm(None); // g1 is skipped -> r2
    assert_eq!(playing(&f.bard).as_deref(), Some("r2"));
    f.bard.previous();
    assert_eq!(playing(&f.bard).as_deref(), Some("r1"));
}

#[test]
fn obs_arming_names_its_source_and_tells_the_user() {
    let mut f = fixture(false);
    f.bard.arm(Some(StreamerSource::Record));
    assert!(f.bard.is_streamer_armed());
    assert_eq!(f.bard.streamer_source(), Some(StreamerSource::Record));
    let ev = f.drain();
    assert!(matches!(ev[0], CoreEvent::StreamerSource(Some(StreamerSource::Record))));
    assert!(ev.iter().any(|e| matches!(e, CoreEvent::ConfigChanged(c) if c.streamer.enabled)));
    assert!(ev.iter().any(|e| matches!(e, CoreEvent::Notice(n) if n.starts_with("OBS went live"))));

    // Already armed: the source is updated, nothing else said.
    f.bard.arm(Some(StreamerSource::Stream));
    assert_eq!(f.drain(), [CoreEvent::StreamerSource(Some(StreamerSource::Stream))]);

    // Toggling by hand clears the source.
    f.bard.disarm();
    assert_eq!(f.bard.streamer_source(), None);
    f.bard.arm(Some(StreamerSource::Stream));
    f.bard.update_config(|c| c.streamer.enabled = false);
    assert_eq!(f.bard.streamer_source(), None);
}

#[test]
fn loved_tracks_and_stats_persist() {
    let dir = tempfile::tempdir().unwrap();
    {
        let (mut bard, rx) = Bard::open(dir.path().to_path_buf()).unwrap();
        bard.install_library(library());
        assert_eq!(bard.toggle_loved("g1"), ["g1"]);
        assert!(bard.is_loved("g1"));
        assert!(rx.try_iter().any(|e| matches!(e, CoreEvent::LovedChanged)));
        bard.play_now("g2");
        let p = progress(&bard, 31.0);
        bard.report_progress(p);
        bard.shutdown();
    }
    let (bard, _rx) = Bard::open(dir.path().to_path_buf()).unwrap();
    assert_eq!(bard.loved(), ["g1"]);
    assert_eq!(bard.stats()["g2"].plays, 1);

    let mut bard = bard;
    bard.clear_stats();
    assert!(bard.stats().is_empty());
}

#[test]
fn nowplaying_json_is_written_on_track_and_play_state_changes() {
    let mut f = fixture(false);
    f.bard.set_overlay_port(Some(4748));
    f.bard.enqueue("g1");
    let path = f.dir().join("nowplaying.json");
    let read = || -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap() };
    let v = read();
    assert_eq!(v["title"], "Title g1");
    assert_eq!(v["artUrl"], "http://127.0.0.1:4748/art/g1");
    assert_eq!(v["isPlaying"], false);
    assert_eq!(v["durationSec"], 120);
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.starts_with("{\n  \"title\": \"Title g1\",\n  \"artist\""), "{body}");

    // A progress tick that changes play state writes; one that does not, does not.
    let p = progress(&f.bard, 1.0);
    f.bard.report_progress(p);
    assert_eq!(read()["isPlaying"], true);
    std::fs::remove_file(&path).unwrap();
    let p = progress(&f.bard, 2.0);
    f.bard.report_progress(p);
    assert!(!path.exists());

    // Every report reaches the overlay.
    let overlay = f.drain().into_iter().filter(|e| matches!(e, CoreEvent::Overlay(_))).count();
    assert!(overlay >= 2);

    f.bard.shutdown();
    let v = read();
    assert_eq!(v["title"], "");
    assert_eq!(v["isPlaying"], false);
    assert_eq!(v["queueLength"], 0);
    assert_eq!(v["artUrl"], serde_json::Value::Null);
}

#[test]
fn nowplaying_json_can_be_turned_off() {
    let mut f = fixture(false);
    f.bard.update_config(|c| c.overlay.write_now_playing_json = false);
    f.bard.enqueue("g1");
    f.bard.shutdown();
    assert!(!f.dir().join("nowplaying.json").exists());
}

#[test]
fn add_library_path_dedupes_and_persists() {
    let mut f = fixture(false);
    assert_eq!(f.bard.add_library_path("/mnt/music/"), ["/mnt/music"]);
    assert_eq!(f.bard.add_library_path("/mnt/music"), ["/mnt/music"]);
    assert_eq!(f.bard.add_library_paths(["/a", "/mnt/./music"]), ["/mnt/music", "/a"]);
    let raw = std::fs::read_to_string(f.dir().join("config.json")).unwrap();
    assert!(raw.contains("\"libraryPaths\": [\n\t\t\"/mnt/music\",\n\t\t\"/a\"\n\t]"));
}

#[test]
fn search_filters_and_matches() {
    let f = fixture(false);
    assert_eq!(f.bard.search("").len(), 12);
    assert_eq!(f.bard.search("RAIN").len(), 4);
    assert_eq!(f.bard.best_match("title g3").map(|t| t.id.as_str()), Some("g3"));
    assert!(f.bard.best_match("bohemian rhapsody").is_none());
}

#[test]
fn transport_and_notice_relay() {
    let f = fixture(false);
    f.bard.transport(TransportAction::Toggle);
    f.bard.notice("hi");
    assert_eq!(f.drain(), [CoreEvent::Transport(TransportAction::Toggle), CoreEvent::Notice("hi".into())]);
}

#[test]
fn scanning_an_empty_folder_reports_and_installs() {
    let mut f = fixture(false);
    let music = f.dir().join("music");
    std::fs::create_dir_all(&music).unwrap();
    std::fs::write(music.join("x.flac"), b"junk").unwrap();
    f.bard.update_config(|c| c.library_paths = vec![music.to_string_lossy().into_owned()]);
    let tracks = f.bard.scan_library();
    assert_eq!(tracks.len(), 1);
    assert_eq!(f.bard.tracks().len(), 1);
    let ev = f.drain();
    assert!(ev.iter().any(|e| matches!(e, CoreEvent::ScanProgress(p) if p.scanned == 1 && p.total == 1)));
    assert!(f.bard.artwork_path(&tracks[0].id).is_none());
}

#[test]
fn gain_multiplier_is_exported() {
    let mut t = track("x", "A", 1);
    t.track_gain_db = Some(-6.0206);
    assert!((gain_multiplier(&t, NormalizeMode::Track) - 0.5).abs() < 1e-4);
    assert_eq!(gain_multiplier(&t, NormalizeMode::Off), 1.0);
}

/// A setting that cannot be saved is not applied: the live config, the
/// returned config and the events all agree with the file, and the same
/// change made again once saving works goes through.
#[test]
fn a_setting_that_fails_to_save_is_not_applied() {
    let mut f = fixture(false);
    let before = f.bard.config().streamer.enabled;
    // The atomic write goes through config.json.tmp; a directory there makes
    // it fail.
    let blocker = f.dir().join("config.json.tmp");
    std::fs::create_dir(&blocker).unwrap();

    let returned = f.bard.update_config(|c| c.streamer.enabled = !before);
    assert_eq!(returned.streamer.enabled, before, "returns the config in force");
    assert_eq!(f.bard.config().streamer.enabled, before, "the live config is unchanged");
    let events = f.drain();
    assert!(!events.iter().any(|e| matches!(e, CoreEvent::ConfigChanged(_))), "{events:?}");
    assert!(
        events.iter().any(|e| matches!(e, CoreEvent::Notice(n) if n.starts_with("Could not save settings"))),
        "{events:?}"
    );
    let on_disk = std::fs::read_to_string(f.dir().join("config.json")).unwrap();
    assert_eq!(on_disk.contains("\"enabled\": true"), before);

    std::fs::remove_dir(&blocker).unwrap();
    let returned = f.bard.update_config(|c| c.streamer.enabled = !before);
    assert_eq!(returned.streamer.enabled, !before, "the retry is a change");
    assert_eq!(f.bard.config().streamer.enabled, !before);
    assert!(f.drain().iter().any(|e| matches!(e, CoreEvent::ConfigChanged(c) if c.streamer.enabled == !before)));
}

/// Armed, with Rain safe and playing r1, and r2 and a second r1 queued.
fn armed_on_rain(exclude_claimed: bool) -> Fixture {
    let mut f = fixture(false);
    f.bard.update_config(|c| c.playback.exclude_claimed = exclude_claimed);
    f.bard.set_album_flag("Rain", ContentIdStatus::Safe, None);
    f.bard.set_album_flag("Moon", ContentIdStatus::Safe, None);
    f.bard.arm(None);
    f.bard.enqueue_many(&["r1", "r2", "m1", "r1"], false);
    assert_eq!(playing(&f.bard).as_deref(), Some("r1"));
    assert_eq!(queue_ids(&f.bard), ["r2", "m1", "r1"]);
    f.drain();
    f
}

#[test]
fn an_album_that_stops_being_safe_while_armed_leaves_the_air_and_the_queue() {
    for exclude_claimed in [true, false] {
        for status in [ContentIdStatus::Claimed, ContentIdStatus::Untested] {
            let mut f = armed_on_rain(exclude_claimed);
            f.bard.set_album_flag("Rain", status, None);
            assert_eq!(
                playing(&f.bard).as_deref(),
                Some("m1"),
                "the track on air is skipped ({status:?}, exclude_claimed {exclude_claimed})"
            );
            assert!(queue_ids(&f.bard).iter().all(|id| !id.starts_with('r')), "{:?}", queue_ids(&f.bard));
            let events = f.drain();
            assert!(
                !events.iter().any(|e| matches!(e, CoreEvent::PlayTrack(t, _) if t.album == "Rain")),
                "nothing from Rain is sent to the engine: {events:?}"
            );
            // Nor later.
            for _ in 0..6 {
                f.bard.report_ended();
                assert!(playing(&f.bard).is_none_or(|id| !id.starts_with('r')));
            }
        }
    }
}

#[test]
fn repeat_one_does_not_replay_a_track_no_longer_safe() {
    let mut f = armed_on_rain(true);
    f.bard.update_config(|c| c.playback.repeat = RepeatMode::One);
    f.bard.report_ended();
    assert_eq!(playing(&f.bard).as_deref(), Some("r1"), "repeats while safe");
    f.bard.set_album_flag("Rain", ContentIdStatus::Untested, None);
    f.drain();
    f.bard.report_ended();
    assert!(playing(&f.bard).is_none_or(|id| !id.starts_with('r')));
}

#[test]
fn a_track_that_fails_to_open_is_skipped_or_playback_stops() {
    let mut f = fixture(false);
    f.bard.enqueue_many(&["g1", "g2", "g3"], false);
    assert_eq!(playing(&f.bard).as_deref(), Some("g1"));
    f.drain();

    f.bard.report_failed(false);
    assert_eq!(playing(&f.bard).as_deref(), Some("g2"), "moves on");
    assert!(f.drain().iter().any(|e| matches!(e, CoreEvent::PlayTrack(t, _) if t.id == "g2")));
    assert_eq!(f.bard.track_stats("g1").skips, 0, "a track that never played is not a skip");

    f.bard.report_failed(true);
    assert!(f.bard.now_playing().is_none(), "gives up");
    assert_eq!(queue_ids(&f.bard), ["g3"], "the queue is kept");
    let events = f.drain();
    assert!(events.iter().any(|e| matches!(e, CoreEvent::NowPlaying(None))), "{events:?}");
    assert!(!events.iter().any(|e| matches!(e, CoreEvent::PlayTrack(..))), "{events:?}");
}

#[test]
fn an_older_scan_finishing_last_is_not_installed() {
    let mut f = fixture(false);
    let slow = f.bard.scanner();
    let fast = f.bard.scanner();
    assert!(!slow.is_current() && fast.is_current());

    let newer = vec![track("n1", "New", 1)];
    assert!(f.bard.install_scan(&fast, newer));
    assert!(!f.bard.install_scan(&slow, library()), "the older result is dropped");
    let ids: Vec<&str> = f.bard.tracks().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["n1"]);
}

#[test]
fn changing_the_folders_makes_a_scan_in_flight_obsolete() {
    let mut f = fixture(false);
    let in_flight = f.bard.scanner();
    f.bard.update_config(|c| c.library_paths = vec!["/elsewhere".into()]);
    assert!(!in_flight.is_current());
    assert!(!f.bard.install_scan(&in_flight, library()));
}

#[test]
fn removing_a_folder_drops_its_tracks_from_the_library_and_queue() {
    let mut f = fixture(false);
    let mut tracks = library();
    for t in &mut tracks {
        // Gold lives in one folder, everything else in another.
        let root = if t.album == "Gold" { "/music/a" } else { "/music/b" };
        t.path = format!("{root}/{}", t.path.trim_start_matches("/music/"));
    }
    f.bard.update_config(|c| c.library_paths = vec!["/music/a".into(), "/music/b".into()]);
    f.bard.install_library(tracks);
    f.bard.enqueue_many(&["r1", "g1", "m1", "g2"], false);
    assert_eq!(playing(&f.bard).as_deref(), Some("r1"));
    f.drain();

    f.bard.update_config(|c| c.library_paths.retain(|p| p != "/music/b"));
    assert!(f.bard.tracks().iter().all(|t| t.album == "Gold"), "only the kept folder's tracks remain");
    assert_eq!(queue_ids(&f.bard), ["g1", "g2"]);
    assert_eq!(playing(&f.bard).as_deref(), Some("r1"), "the track on air finishes");
    let events = f.drain();
    assert!(events.iter().any(|e| matches!(e, CoreEvent::LibraryChanged)), "{events:?}");

    // Nothing from the removed folder is picked automatically.
    f.bard.update_config(|c| c.playback.auto_fill = AutoFillMode::Shuffle);
    for _ in 0..12 {
        f.bard.report_ended();
        if let Some(id) = playing(&f.bard) {
            assert!(id.starts_with('g'), "{id}");
        }
    }
}

fn transitions(events: &[CoreEvent]) -> Vec<(String, Transition)> {
    events
        .iter()
        .filter_map(|e| if let CoreEvent::PlayTrack(t, tr) = e { Some((t.id.clone(), *tr)) } else { None })
        .collect()
}

/// Streamer mode taking a track off the air does it with a cut, so the
/// engine does not fade the forbidden track out over the crossfade; the
/// user's own moves keep the normal transition.
#[test]
fn streamer_enforcement_cuts_and_everything_else_does_not() {
    let mut f = armed_on_rain(true);
    f.bard.skip();
    assert_eq!(transitions(&f.drain()), [("r2".to_string(), Transition::Normal)]);

    f.bard.set_album_flag("Rain", ContentIdStatus::Claimed, None);
    let events = f.drain();
    assert_eq!(transitions(&events), [("m1".to_string(), Transition::Cut)]);
    // The track fading out (r1) is gone with the one on air; no separate cut.
    assert!(!events.iter().any(|e| matches!(e, CoreEvent::CutFade)), "{events:?}");

    f.bard.report_ended();
    assert!(transitions(&f.drain()).iter().all(|(_, t)| *t == Transition::Normal));

    // Arming over a track that is not safe.
    let mut f = fixture(false);
    f.bard.set_album_flag("Moon", ContentIdStatus::Safe, None);
    f.bard.enqueue_many(&["r1", "m1"], false);
    f.drain();
    f.bard.arm(None);
    assert_eq!(transitions(&f.drain()), [("m1".to_string(), Transition::Cut)]);
}

/// The track before the current one may still be fading out. When it stops
/// being safe the fade is cut, and only then.
#[test]
fn a_forbidden_track_fading_out_is_cut() {
    let mut f = armed_on_rain(true);
    f.bard.set_album_flag("Gold", ContentIdStatus::Safe, None);
    f.bard.enqueue("g1");
    f.bard.clear_queue();
    f.bard.enqueue("m1");
    f.bard.skip(); // r1 -> m1, r1 fading out
    f.drain();

    f.bard.set_album_flag("Gold", ContentIdStatus::Claimed, None);
    assert!(!f.drain().iter().any(|e| matches!(e, CoreEvent::CutFade)), "nothing of Gold is fading out");

    f.bard.set_album_flag("Rain", ContentIdStatus::Claimed, None);
    let events = f.drain();
    assert_eq!(events.iter().filter(|e| matches!(e, CoreEvent::CutFade)).count(), 1, "{events:?}");
    assert!(transitions(&events).is_empty(), "m1 plays on: {events:?}");
    assert_eq!(playing(&f.bard).as_deref(), Some("m1"));

    // Once cut, not again.
    f.bard.set_album_flag("Rain", ContentIdStatus::Untested, None);
    assert!(!f.drain().iter().any(|e| matches!(e, CoreEvent::CutFade)));

    // After a cut there is nothing left fading.
    let mut f = armed_on_rain(true);
    f.bard.set_album_flag("Rain", ContentIdStatus::Claimed, None); // cut r1 -> m1
    f.drain();
    f.bard.set_album_flag("Rain", ContentIdStatus::Untested, None);
    assert!(!f.drain().iter().any(|e| matches!(e, CoreEvent::CutFade)));
}

/// Two folders, a's track played, then b's; a is removed.
fn played_a_then_b(repeat: RepeatMode) -> Fixture {
    let mut f = fixture(false);
    let mut a = track("a1", "A", 1);
    a.path = "/music/a/a1.flac".into();
    let mut b = track("b1", "B", 1);
    b.path = "/music/b/b1.flac".into();
    let mut b2 = track("b2", "B", 2);
    b2.path = "/music/b/b2.flac".into();
    f.bard.update_config(|c| {
        c.library_paths = vec!["/music/a".into(), "/music/b".into()];
        c.playback.auto_fill = AutoFillMode::Off;
        c.playback.repeat = repeat;
    });
    f.bard.install_library(vec![a, b, b2]);
    f.bard.enqueue_many(&["a1", "b1"], false);
    f.bard.report_ended();
    assert_eq!(playing(&f.bard).as_deref(), Some("b1"));
    f.bard.update_config(|c| c.library_paths.retain(|p| p != "/music/a"));
    assert!(f.bard.track("a1").is_none());
    f.drain();
    f
}

#[test]
fn repeat_all_does_not_bring_back_a_removed_folder() {
    let mut f = played_a_then_b(RepeatMode::All);
    f.bard.enqueue("b2");
    f.bard.report_ended();
    assert_eq!(playing(&f.bard).as_deref(), Some("b2"));
    f.drain();
    let mut heard = Vec::new();
    for _ in 0..4 {
        f.bard.report_ended();
        heard.push(playing(&f.bard).unwrap_or_default());
    }
    assert_eq!(heard, ["b1", "b2", "b1", "b2"], "laps b's tracks only");
    assert!(!transitions(&f.drain()).iter().any(|(id, _)| id == "a1"));
}

#[test]
fn previous_does_not_step_back_into_a_removed_folder() {
    let mut f = played_a_then_b(RepeatMode::Off);
    f.bard.previous();
    assert_ne!(playing(&f.bard).as_deref(), Some("a1"));
    assert!(!transitions(&f.drain()).iter().any(|(id, _)| id == "a1"));
}

/// The track on air from a removed folder finishes; it is not replayed.
#[test]
fn a_removed_track_on_air_finishes_but_does_not_come_back() {
    // Repeat One.
    let mut f = played_a_then_b(RepeatMode::Off);
    f.bard.enqueue("a1"); // gone: nothing
    f.bard.play_now("b2");
    f.bard.update_config(|c| c.library_paths = vec!["/music/a".into()]);
    // b2 is on air and now outside the library, as is all of B.
    f.bard.update_config(|c| c.playback.repeat = RepeatMode::One);
    f.drain();
    f.bard.report_ended();
    assert!(!transitions(&f.drain()).iter().any(|(id, _)| id == "b2"), "not repeated");

    // Previous mid-track does not restart it, nor put it back in the queue.
    let mut f = played_a_then_b(RepeatMode::Off);
    f.bard.update_config(|c| c.library_paths = vec!["/music/a".into()]);
    f.drain();
    let p = progress(&f.bard, 60.0);
    f.bard.report_progress(p);
    f.bard.previous();
    assert!(!transitions(&f.drain()).iter().any(|(id, _)| id == "b1"), "not restarted");
    assert!(!queue_ids(&f.bard).contains(&"b1".to_string()));
}

/// A scan that started before the folder was removed cannot put it back.
#[test]
fn a_scan_from_before_the_removal_cannot_bring_the_folder_back() {
    let mut f = fixture(false);
    let mut a = track("a1", "A", 1);
    a.path = "/music/a/a1.flac".into();
    let mut b = track("b1", "B", 1);
    b.path = "/music/b/b1.flac".into();
    f.bard.update_config(|c| {
        c.library_paths = vec!["/music/a".into(), "/music/b".into()];
        c.playback.auto_fill = AutoFillMode::Off;
        c.playback.repeat = RepeatMode::All;
    });
    f.bard.install_library(vec![a.clone(), b.clone()]);
    let before = f.bard.scanner();
    f.bard.enqueue_many(&["a1", "b1"], false);
    f.bard.report_ended();
    f.bard.update_config(|c| c.library_paths.retain(|p| p != "/music/a"));
    assert!(!f.bard.install_scan(&before, vec![a, b]));
    f.drain();
    for _ in 0..3 {
        f.bard.report_ended();
    }
    assert!(!transitions(&f.drain()).iter().any(|(id, _)| id == "a1"));
}

fn read_np(f: &Fixture) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(f.dir().join("nowplaying.json")).unwrap()).unwrap()
}

fn report(b: &mut Bard, id: &str, position: f64, duration: f64, playing: bool) {
    b.report_progress(PlaybackProgress { position, duration, playing, track_id: Some(id.into()) });
}

/// nowplaying.json pairs a track with its own timing: a switch resets it,
/// the new track's first report is written, later ticks are not.
#[test]
fn nowplaying_json_timing_follows_the_track() {
    let mut f = fixture(false);
    let mut long = track("long", "Gold", 9);
    long.duration_sec = 200.0;
    let mut lib = library();
    lib.push(long);
    f.bard.install_library(lib);
    f.bard.enqueue_many(&["g1", "long", "g2"], false);
    report(&mut f.bard, "g1", 90.0, 120.0, true);
    assert_eq!(read_np(&f)["positionSec"], 90.0);

    // Playing to playing.
    f.bard.skip();
    let v = read_np(&f);
    assert_eq!(v["title"], "Title long");
    assert_eq!(v["durationSec"], 200, "the tag duration until the engine reports");
    assert_eq!(v["positionSec"], 0);
    assert_eq!(v["isPlaying"], true);

    // The outgoing track's reports (a crossfade) change nothing.
    report(&mut f.bard, "g1", 91.0, 120.0, true);
    assert_eq!(read_np(&f)["positionSec"], 0);

    // The first report of the new track is written, with the decoder's duration.
    report(&mut f.bard, "long", 0.25, 199.5, true);
    let v = read_np(&f);
    assert_eq!((v["positionSec"].as_f64(), v["durationSec"].as_f64()), (Some(0.25), Some(199.5)));
    // Later ticks are not.
    report(&mut f.bard, "long", 10.0, 199.5, true);
    assert_eq!(read_np(&f)["positionSec"], 0.25);

    // Paused, then switched: still paused, timing reset.
    report(&mut f.bard, "long", 50.0, 199.5, false);
    f.bard.skip();
    let v = read_np(&f);
    assert_eq!(v["title"], "Title g2");
    assert_eq!((v["positionSec"].as_f64(), v["durationSec"].as_f64()), (Some(0.0), Some(120.0)));
    assert_eq!(v["isPlaying"], false);
    report(&mut f.bard, "g2", 0.0, 119.0, false);
    assert_eq!(read_np(&f)["durationSec"], 119.0, "the paused track's first report is written too");
}

#[test]
fn review_rescan_updates_persisted_metadata_without_art_change() {
    let mut f = fixture(false);
    f.bard.play_now("g1");
    let p = progress(&f.bard, 5.0);
    f.bard.report_progress(p);
    let mut lib = library();
    lib[0].title = "Retagged current title".into();
    f.bard.install_library(lib);
    let p = progress(&f.bard, 6.0);
    f.bard.report_progress(p);
    assert_eq!(f.bard.now_playing_snapshot().title, "Retagged current title");
    assert_eq!(read_np(&f)["title"], "Retagged current title", "file consumers still see the old title");
}

fn overlay_frames(f: &Fixture) -> Vec<NowPlaying> {
    f.drain().into_iter().filter_map(|e| if let CoreEvent::Overlay(n) = e { Some(n) } else { None }).collect()
}

/// A rescan that retags the playing track -- title, artist or album, playing
/// or paused -- rewrites nowplaying.json and sends the overlay a frame at
/// once, keeping the track's timing and play state. One that changes
/// nothing a viewer sees does neither.
#[test]
fn a_retag_of_the_current_track_reaches_the_file_and_overlay_at_once() {
    let mut f = fixture(false);
    f.bard.play_now("g1");
    report(&mut f.bard, "g1", 40.0, 119.5, true);
    let path = f.dir().join("nowplaying.json");
    type Edit = fn(&mut Track);
    let edits: [(&str, Edit, &str, &str); 3] = [
        ("title", |t| t.title = "New title".into(), "title", "New title"),
        ("artist", |t| t.artist = "New artist".into(), "artist", "New artist"),
        ("album", |t| t.album = "New album".into(), "album", "New album"),
    ];
    for playing in [true, false] {
        report(&mut f.bard, "g1", 40.0, 119.5, playing);
        for (what, edit, key, want) in edits {
            let mut lib = library();
            edit(&mut lib[0]);
            f.drain();
            f.bard.install_library(lib);
            let v = read_np(&f);
            assert_eq!(v[key], want, "{what} while playing={playing}");
            assert_eq!(v["isPlaying"], playing, "{what}: play state kept");
            assert_eq!((v["positionSec"].as_f64(), v["durationSec"].as_f64()), (Some(40.0), Some(119.5)), "{what}: timing kept");
            let frames = overlay_frames(&f);
            assert!(frames.iter().any(|n| n.title == v["title"] && n.artist == v["artist"] && n.album == v["album"]), "{what}: overlay frame");
            assert_eq!(self::playing(&f.bard).as_deref(), Some("g1"), "{what}: still the same track");

            // Put it back, then an unchanged rescan touches nothing.
            f.bard.install_library(library());
            assert_eq!(read_np(&f)["title"], "Title g1");
            std::fs::remove_file(&path).unwrap();
            f.drain();
            f.bard.install_library(library());
            assert!(!path.exists(), "{what}: an unchanged rescan rewrote the file");
            assert!(overlay_frames(&f).is_empty(), "{what}: an unchanged rescan sent a frame");
            report(&mut f.bard, "g1", 40.0, 119.5, !playing);
            report(&mut f.bard, "g1", 40.0, 119.5, playing);
        }
    }
}

#[test]
fn a_retag_with_the_file_turned_off_still_reaches_the_overlay() {
    let mut f = fixture(false);
    f.bard.update_config(|c| c.overlay.write_now_playing_json = false);
    f.bard.play_now("g1");
    report(&mut f.bard, "g1", 3.0, 120.0, true);
    let mut lib = library();
    lib[0].artist = "Someone else".into();
    f.drain();
    f.bard.install_library(lib);
    assert!(overlay_frames(&f).iter().any(|n| n.artist == "Someone else"));
    assert!(!f.dir().join("nowplaying.json").exists());
}

/// Two folders: a's tracks a1/a2, b's b1/b2.
fn two_folders(armed: bool) -> Fixture {
    let mut f = fixture(false);
    let mut v = Vec::new();
    for (root, id, n) in [("a", "a1", 1), ("a", "a2", 2), ("b", "b1", 1), ("b", "b2", 2)] {
        let mut t = track(id, &root.to_uppercase(), n);
        t.path = format!("/music/{root}/{id}.flac");
        v.push(t);
    }
    f.bard.update_config(|c| {
        c.library_paths = vec!["/music/a".into(), "/music/b".into()];
        c.playback.auto_fill = AutoFillMode::Off;
    });
    f.bard.install_library(v);
    if armed {
        f.bard.set_album_flag("A", ContentIdStatus::Safe, None);
        f.bard.set_album_flag("B", ContentIdStatus::Safe, None);
        f.bard.arm(None);
    }
    f.drain();
    f
}

/// The entry kept for a removed track while it finished is not a way back
/// to it: after the next track starts -- by finishing, a skip or Play now --
/// Previous goes to valid history, or restarts what is playing if there is
/// none. Armed and not.
#[test]
fn a_removed_track_that_finished_is_not_reached_by_previous() {
    enum Leave {
        Finish,
        Skip,
        PlayNow,
    }
    for armed in [false, true] {
        for leave in [Leave::Finish, Leave::Skip, Leave::PlayNow] {
            for earlier in [false, true] {
                let mut f = two_folders(armed);
                if earlier {
                    f.bard.play_now("b2"); // valid history before a1
                }
                f.bard.play_now("a1");
                f.bard.enqueue("b1");
                f.bard.update_config(|c| c.library_paths.retain(|p| p != "/music/a"));
                assert!(f.bard.track("a1").is_none());
                match leave {
                    Leave::Finish => f.bard.report_ended(),
                    Leave::Skip => f.bard.skip(),
                    Leave::PlayNow => {
                        f.bard.play_now("b1");
                    }
                }
                assert_eq!(playing(&f.bard).as_deref(), Some("b1"));
                f.drain();
                f.bard.previous();
                let now = playing(&f.bard);
                assert_ne!(now.as_deref(), Some("a1"), "armed {armed}, earlier history {earlier}");
                assert_eq!(now.as_deref(), Some(if earlier { "b2" } else { "b1" }));
                assert!(!transitions(&f.drain()).iter().any(|(id, _)| id == "a1"));
                assert!(!queue_ids(&f.bard).contains(&"a1".to_string()));
            }
        }
    }
}

/// Previous walks back over several stale entries to the newest valid one.
#[test]
fn previous_skips_any_number_of_stale_entries() {
    let mut f = two_folders(false);
    for id in ["b1", "a1", "a2", "b2"] {
        f.bard.play_now(id);
    }
    f.bard.update_config(|c| c.library_paths.retain(|p| p != "/music/a"));
    f.drain();
    f.bard.previous();
    assert_eq!(playing(&f.bard).as_deref(), Some("b1"));
}

fn cut_fades(events: &[CoreEvent]) -> usize {
    events.iter().filter(|e| matches!(e, CoreEvent::CutFade)).count()
}

/// The engine fades out the source it has, which after several requests in
/// one fade is the first, not the one before last. Every track that may
/// still be heard is checked until the engine reports on the current request.
#[test]
fn every_track_that_may_still_be_fading_is_checked() {
    let mut f = armed_on_rain(true);
    f.bard.set_album_flag("Gold", ContentIdStatus::Safe, None);
    f.drain();
    // r1 on air; m1 then g1 requested within one fade.
    f.bard.play_now("m1");
    f.bard.play_now("g1");
    f.drain();
    f.bard.set_album_flag("Rain", ContentIdStatus::Claimed, None);
    let events = f.drain();
    assert_eq!(cut_fades(&events), 1, "r1, two requests back, is cut: {events:?}");
    assert!(transitions(&events).is_empty(), "g1 plays on");

    // Longer chains, and the middle one revoked.
    let mut f = armed_on_rain(true);
    f.bard.set_album_flag("Gold", ContentIdStatus::Safe, None);
    for id in ["m1", "r2", "g1", "g2"] {
        f.bard.play_now(id);
    }
    f.drain();
    f.bard.set_album_flag("Moon", ContentIdStatus::Claimed, None);
    assert_eq!(cut_fades(&f.drain()), 1);

    // Once the engine reports on the current request nothing else is heard,
    // so a revocation then cuts nothing.
    let mut f = armed_on_rain(true);
    f.bard.set_album_flag("Gold", ContentIdStatus::Safe, None);
    f.bard.play_now("m1");
    f.bard.play_now("g1");
    report(&mut f.bard, "g1", 0.1, 120.0, true);
    f.drain();
    f.bard.set_album_flag("Rain", ContentIdStatus::Claimed, None);
    assert_eq!(cut_fades(&f.drain()), 0);

    // A report from a track no longer current acknowledges nothing.
    let mut f = armed_on_rain(true);
    f.bard.set_album_flag("Gold", ContentIdStatus::Safe, None);
    f.bard.play_now("m1");
    f.bard.play_now("g1");
    report(&mut f.bard, "r1", 2.0, 120.0, true);
    f.drain();
    f.bard.set_album_flag("Rain", ContentIdStatus::Claimed, None);
    assert_eq!(cut_fades(&f.drain()), 1);
}

/// A damaged store at startup: Bard opens, says so, keeps the bytes, and
/// plays on.
#[test]
fn a_damaged_store_at_startup_is_reported_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("albums.json"), r#"{"version":1,"albums":{"Rain":{"status":"cla"#).unwrap();
    let (mut bard, rx) = Bard::open(dir.path().to_path_buf()).unwrap();
    let notices: Vec<String> =
        rx.try_iter().filter_map(|e| if let CoreEvent::Notice(n) = e { Some(n) } else { None }).collect();
    assert!(notices.iter().any(|n| n.contains("albums.json") && n.contains("set aside")), "{notices:?}");
    let states = bard.store_states();
    assert!(matches!(&states[1], ("albums.json", bard_core::LoadState::Recovered { .. })), "{states:?}");
    assert!(matches!(&states[0], ("stats.json", bard_core::LoadState::Ok)));
    let kept: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("albums.json.corrupt-"))
        .collect();
    assert_eq!(kept.len(), 1);
    bard.update_config(|c| c.playback.endless = false);
    bard.install_library(library());
    bard.enqueue("g1");
    assert_eq!(playing(&bard).as_deref(), Some("g1"));
    assert!(bard.flush());
}

fn config_on_disk(f: &Fixture) -> Config {
    serde_json::from_str(&std::fs::read_to_string(f.dir().join("config.json")).unwrap()).unwrap()
}

/// Slider changes are in effect and announced at once, and written behind;
/// anything else asked of adjust_config is written before it applies.
#[test]
fn slider_settings_apply_at_once_and_reach_the_disk_by_shutdown() {
    let mut f = fixture(false);
    let c = f.bard.adjust_config(|c| c.audio.volume = 0.37);
    assert_eq!(c.audio.volume, 0.37);
    assert!(f.drain().iter().any(|e| matches!(e, CoreEvent::ConfigChanged(c) if c.audio.volume == 0.37)));

    // Not a slider: transactional, on disk on return.
    f.bard.adjust_config(|c| c.playback.repeat = RepeatMode::All);
    let disk = config_on_disk(&f);
    assert_eq!(disk.playback.repeat, RepeatMode::All);
    assert_eq!(disk.audio.volume, 0.37, "the pending slider value went with it");

    f.bard.adjust_config(|c| c.audio.crossfade_sec = 7.);
    f.bard.shutdown();
    assert_eq!(config_on_disk(&f).audio.crossfade_sec, 7.);
}

/// The overlay's art URL names the cached cover's version: a rescan that
/// replaces the playing track's cover sends a new URL at once (the overlay
/// only reloads on a new URL), one that leaves it alone sends nothing, and
/// art removed or added follows too.
#[test]
fn a_rescan_that_replaces_the_current_cover_gives_the_overlay_a_new_url() {
    let mut f = fixture(false);
    f.bard.update_config(|c| c.overlay.write_now_playing_json = false);
    f.bard.set_overlay_port(Some(4321));
    let art = f.dir().join("artwork");
    std::fs::create_dir_all(&art).unwrap();
    std::fs::write(art.join("g1.jpg"), b"\xff\xd8old\xff\xd9").unwrap();
    f.bard.play_now("g1");
    let url = |b: &Bard| b.now_playing_snapshot().art_url;
    let first = url(&f.bard).unwrap();
    assert!(first.starts_with("http://127.0.0.1:4321/art/g1?v="), "{first}");
    f.drain();

    let overlay_urls = |f: &Fixture| -> Vec<Option<String>> {
        f.drain()
            .into_iter()
            .filter_map(|e| if let CoreEvent::Overlay(n) = e { Some(n.art_url) } else { None })
            .collect()
    };

    // Unchanged: no new frame.
    f.bard.install_library(library());
    assert!(overlay_urls(&f).is_empty());
    assert_eq!(url(&f.bard).as_ref(), Some(&first));

    // Replaced by a scan: a new URL, pushed straight away.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(art.join("g1.jpg"), b"\xff\xd8the new cover\xff\xd9").unwrap();
    f.bard.install_library(library());
    let pushed = overlay_urls(&f);
    let second = url(&f.bard).unwrap();
    assert_ne!(second, first);
    assert_eq!(pushed, [Some(second.clone())]);

    // Removed from the file: the scan says no art, and the overlay hides it.
    std::fs::remove_file(art.join("g1.jpg")).unwrap();
    let mut lib = library();
    lib[0].has_artwork = false;
    f.bard.install_library(lib.clone());
    assert_eq!(overlay_urls(&f), [None]);

    // Added back.
    std::fs::write(art.join("g1.jpg"), b"\xff\xd8again\xff\xd9").unwrap();
    lib[0].has_artwork = true;
    f.bard.install_library(lib);
    let third = url(&f.bard).unwrap();
    assert!(third.contains("?v="));
    assert_eq!(overlay_urls(&f), [Some(third)]);
}
