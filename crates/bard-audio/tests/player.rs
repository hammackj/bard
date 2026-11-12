//! The whole engine against the null backend: real threads, real time, no
//! device and no sound.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use bard_audio::{Backend, FFT_SIZE, Player, PlayerEvent, PlayerOptions};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn player() -> (Player, Receiver<PlayerEvent>) {
    Player::with_options(PlayerOptions { backend: Backend::Null, ..Default::default() }).unwrap()
}

/// Collects events for `d`.
fn collect(rx: &Receiver<PlayerEvent>, d: Duration) -> Vec<(Instant, PlayerEvent)> {
    let end = Instant::now() + d;
    let mut v = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(e) => v.push((Instant::now(), e)),
            Err(_) => break,
        }
    }
    v
}

fn wait_for(rx: &Receiver<PlayerEvent>, d: Duration, f: impl Fn(&PlayerEvent) -> bool) -> Option<PlayerEvent> {
    let end = Instant::now() + d;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(e) if f(&e) => return Some(e),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

fn peak(p: &Player) -> u8 {
    let mut buf = [0u8; FFT_SIZE];
    p.analyser().time_domain(&mut buf);
    buf.iter().map(|b| b.abs_diff(128)).max().unwrap()
}

#[test]
fn plays_to_the_end_with_progress_and_ended() {
    let (p, rx) = player();
    let t0 = Instant::now();
    p.play("a".into(), fixture("bursts.opus"), 1.0, 0.0);

    let first = wait_for(&rx, Duration::from_secs(1), |_| true).unwrap();
    match first {
        PlayerEvent::Progress { playing, track_id, duration, .. } => {
            assert!(playing);
            assert_eq!(track_id.as_deref(), Some("a"));
            assert!((duration - 3.0).abs() < 0.05, "{duration}");
        }
        e => panic!("{e:?}"),
    }

    // Watch the analyser for the bursts as it plays: the visualiser's view.
    let mut loud = 0;
    let mut events = Vec::new();
    while t0.elapsed() < Duration::from_secs(5) {
        if peak(&p) > 40 {
            loud += 1;
        }
        while let Ok(e) = rx.try_recv() {
            events.push((Instant::now(), e));
        }
        if events.iter().any(|(_, e)| matches!(e, PlayerEvent::Ended { .. })) {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    assert!(loud > 5, "analyser saw the bursts ({loud} loud frames)");

    let ended_at = events.iter().position(|(_, e)| matches!(e, PlayerEvent::Ended { .. })).expect("Ended");
    let took = events[ended_at].0 - t0;
    assert!(took > Duration::from_millis(2900) && took < Duration::from_millis(3700), "ended after {took:?}");

    let progress: Vec<_> = events[..ended_at]
        .iter()
        .filter_map(|(t, e)| match e {
            PlayerEvent::Progress { position, playing, track_id, .. } => {
                Some((*t, *position, *playing, track_id.clone()))
            }
            _ => None,
        })
        .collect();
    // ~4 Hz over 3 s.
    assert!((9..=16).contains(&progress.len()), "{} progress events", progress.len());
    assert!(progress.windows(2).all(|w| w[1].1 >= w[0].1), "position only moves forward");
    assert!(progress.iter().all(|p| p.3.as_deref() == Some("a")));
    // The element's order: a last progress with playing=false, then ended.
    let last = progress.last().unwrap();
    assert!(!last.2);
    assert!((last.1 - 3.0).abs() < 0.05, "final position {}", last.1);
    assert!(!p.position().playing);
    // Silence after the end.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(peak(&p), 0);
}

#[test]
fn seek_pause_resume_toggle() {
    let (p, rx) = player();
    assert!(!p.toggle(), "nothing loaded: toggle does nothing");
    p.play("a".into(), fixture("bursts.flac"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |_| true).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    p.seek(2.0);
    let e = wait_for(
        &rx,
        Duration::from_secs(1),
        |e| matches!(e, PlayerEvent::Progress { position, .. } if (*position - 2.0).abs() < 1e-6),
    );
    assert!(e.is_some(), "seek reports its position");
    std::thread::sleep(Duration::from_millis(200));
    let pos = p.position().position;
    assert!((2.1..2.4).contains(&pos), "playing on from the seek: {pos}");

    assert!(!p.toggle(), "toggle pauses");
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: false, .. })).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let held = p.position().position;
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(p.position().position, held, "paused position holds");
    assert!(!p.position().playing);
    assert_eq!(peak(&p), 0, "silence while paused");

    // Seek while paused stays paused.
    p.seek(0.5);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(p.position().position, 0.5);
    assert!(!p.position().playing);

    assert!(p.toggle(), "toggle resumes");
    std::thread::sleep(Duration::from_millis(300));
    let pos = p.position().position;
    assert!((0.7..0.9).contains(&pos), "{pos}");

    // Seek beyond the end ends the track.
    p.seek(99.0);
    assert!(wait_for(&rx, Duration::from_secs(2), |e| matches!(e, PlayerEvent::Ended { .. })).is_some());
    // play() after the end starts over, as a media element does.
    p.resume();
    std::thread::sleep(Duration::from_millis(200));
    let pos = p.position();
    assert!(pos.playing && pos.position < 0.5, "{pos:?}");
}

#[test]
fn crossfade_fades_out_swaps_then_fades_in() {
    let (p, rx) = player();
    p.set_volume(0.8);
    p.play("a".into(), fixture("bursts.flac"), 1.0, 0.0);
    std::thread::sleep(Duration::from_millis(500));
    let before = p.current_gain();
    assert!((before - 0.8).abs() < 0.01, "{before}");

    p.play("b".into(), fixture("bursts.ogg"), 0.5, 1.0);
    std::thread::sleep(Duration::from_millis(500));
    let mid = p.current_gain();
    assert!(mid < before * 0.7 && mid > 0.1, "fading out: {mid}");

    // During the fade-out the loaded track is still "a".
    let ev = collect(&rx, Duration::from_millis(300));
    assert!(ev.iter().any(|(_, e)| matches!(e, PlayerEvent::Progress { track_id: Some(t), .. } if t == "a")));

    // Swap at ~1 s: the incoming track starts from silence, not at full gain.
    let e = wait_for(
        &rx,
        Duration::from_secs(1),
        |e| matches!(e, PlayerEvent::Progress { track_id: Some(t), .. } if t == "b"),
    );
    assert!(e.is_some(), "swapped to b");
    let start = p.current_gain();
    assert!(start < 0.15, "incoming track starts near silence: {start}");
    std::thread::sleep(Duration::from_millis(450));
    let rising = p.current_gain();
    assert!(rising > start && rising < 0.4, "fading in: {rising}");
    std::thread::sleep(Duration::from_millis(800));
    // volume 0.8 x normalisation 0.5
    let settled = p.current_gain();
    assert!((settled - 0.4).abs() < 0.01, "settled at the new track's level: {settled}");
}

#[test]
fn volume_and_normalisation_ramp_into_one_gain() {
    let (p, _rx) = player();
    p.play("a".into(), fixture("bursts.wav"), 2.0, 0.0);
    std::thread::sleep(Duration::from_millis(200));
    assert!((p.current_gain() - 2.0).abs() < 0.01);
    p.set_volume(0.5);
    std::thread::sleep(Duration::from_millis(5));
    let g = p.current_gain();
    assert!(g > 1.0, "ramped, not snapped: {g}");
    std::thread::sleep(Duration::from_millis(300));
    assert!((p.current_gain() - 1.0).abs() < 0.01);
    p.set_norm_gain(10.0);
    std::thread::sleep(Duration::from_millis(300));
    assert!((p.current_gain() - 4.0).abs() < 0.01, "clamped to 4");
    p.set_volume(0.0);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(p.current_gain(), 0.0);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(peak(&p), 0, "volume 0 is exact silence");
}

#[test]
fn stop_unloads() {
    let (p, rx) = player();
    p.play("a".into(), fixture("bursts.mp3"), 1.0, 0.0);
    std::thread::sleep(Duration::from_millis(300));
    p.stop();
    let e = wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { track_id: None, .. }));
    assert!(
        matches!(e, Some(PlayerEvent::Progress { position: 0.0, duration: 0.0, playing: false, track_id: None, .. })),
        "{e:?}"
    );
    assert!(!p.toggle());
    let pos = p.position();
    assert_eq!((pos.position, pos.duration, pos.playing), (0.0, 0.0, false));
}

#[test]
fn errors_are_events() {
    let (p, rx) = player();
    let generation = p.play("x".into(), PathBuf::from("/nonexistent/song.flac"), 1.0, 0.0);
    let e = wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Failed { .. }));
    assert!(
        matches!(&e, Some(PlayerEvent::Failed { generation: g, track_id, message })
            if *g == generation && track_id == "x" && message.contains("song.flac")),
        "{e:?}"
    );
    assert!(!p.position().playing);

    // An unknown output falls back to the default, and says so.
    assert!(p.set_output("alsa_output.gone").is_err());
    let e = wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Error(_)));
    assert!(matches!(e, Some(PlayerEvent::Error(m)) if m.contains("using system default")));
    assert!(p.set_output("default").is_ok());
}

#[test]
fn a_track_that_runs_out_mid_fade_hands_over_without_ended() {
    let (p, rx) = player();
    p.play("a".into(), fixture("bursts.flac"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |_| true).unwrap();
    p.seek(2.6);
    std::thread::sleep(Duration::from_millis(100));
    // A 2 s fade-out with 0.3 s left of the track.
    p.play("b".into(), fixture("bursts.flac"), 1.0, 2.0);
    let ev = collect(&rx, Duration::from_millis(900));
    assert!(!ev.iter().any(|(_, e)| matches!(e, PlayerEvent::Ended { .. })), "{ev:?}");
    assert!(
        ev.iter().any(|(_, e)| matches!(e, PlayerEvent::Progress { track_id: Some(t), playing: true, .. } if t == "b")),
        "b started when a ran out: {ev:?}"
    );
}

/// Pausing while a fade-out is under way: nothing may start playing when the
/// fade's deadline passes, and the incoming track waits, paused, for resume.
#[test]
fn pausing_mid_fade_stays_paused_with_the_incoming_track_loaded() {
    let (p, rx) = player();
    p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. })).unwrap();
    p.play("b".into(), fixture("bursts.wav"), 1.0, 0.4);
    std::thread::sleep(Duration::from_millis(80));
    p.pause();

    let ev = collect(&rx, Duration::from_millis(650));
    assert!(!p.position().playing, "still paused after the fade's deadline: {ev:?}");
    assert!(!ev.iter().any(|(_, e)| matches!(e, PlayerEvent::Progress { playing: true, .. })), "{ev:?}");
    assert!(
        ev.iter().any(|(_, e)| matches!(e, PlayerEvent::Progress { track_id: Some(t), playing: false, position, .. } if t == "b" && *position == 0.0)),
        "b loaded, paused at its start: {ev:?}"
    );

    // Resume plays b, at its level rather than from the fade's silence.
    p.resume();
    let e = wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. }));
    assert!(matches!(&e, Some(PlayerEvent::Progress { track_id: Some(t), .. }) if t == "b"), "{e:?}");
    std::thread::sleep(Duration::from_millis(300));
    assert!((p.current_gain() - 1.0).abs() < 0.01, "{}", p.current_gain());
}

/// Stop during a fade-out drops the incoming track; nothing starts later.
#[test]
fn stopping_mid_fade_starts_nothing() {
    let (p, rx) = player();
    p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. })).unwrap();
    p.play("b".into(), fixture("bursts.wav"), 1.0, 0.4);
    std::thread::sleep(Duration::from_millis(80));
    p.stop();
    let ev = collect(&rx, Duration::from_millis(650));
    assert!(!p.position().playing);
    assert!(!ev.iter().any(|(_, e)| matches!(e, PlayerEvent::Progress { playing: true, .. })), "{ev:?}");
}

/// Every report says which request it belongs to: during a fade-out the
/// outgoing track's reports carry the old generation, and restarting the same
/// track is a new generation even though the track id is the same.
#[test]
fn reports_carry_the_generation_of_their_request() {
    let (p, rx) = player();
    let first = p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    let e = wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. }));
    assert!(matches!(e, Some(PlayerEvent::Progress { generation, .. }) if generation == first));

    let again = p.play("a".into(), fixture("bursts.wav"), 1.0, 0.4);
    assert!(again > first);
    let ev = collect(&rx, Duration::from_millis(700));
    let gens: Vec<u64> = ev
        .iter()
        .filter_map(|(_, e)| if let PlayerEvent::Progress { generation, .. } = e { Some(*generation) } else { None })
        .collect();
    assert!(gens.contains(&first), "the fade-out reports as the first play: {gens:?}");
    assert_eq!(gens.last(), Some(&again), "then the restart takes over: {gens:?}");
    assert!(gens.windows(2).all(|w| w[0] <= w[1]), "never back to the old one: {gens:?}");

    let stopped = p.stop();
    let e = wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { track_id: None, .. }));
    assert!(matches!(e, Some(PlayerEvent::Progress { generation, .. }) if generation == stopped));
}

/// A track that cannot be opened after one that played: the failure names
/// its request, and the engine reports itself stopped rather than playing on.
#[test]
fn a_missing_file_after_a_playing_track_fails_and_stops() {
    let (p, rx) = player();
    p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. })).unwrap();
    let generation = p.play("gone".into(), PathBuf::from("/nonexistent/gone.wav"), 1.0, 0.0);
    let ev = collect(&rx, Duration::from_millis(500));
    assert!(
        ev.iter()
            .any(|(_, e)| matches!(e, PlayerEvent::Progress { playing: false, generation: g, .. } if *g == generation)),
        "{ev:?}"
    );
    assert!(
        ev.iter().any(|(_, e)| matches!(e, PlayerEvent::Failed { generation: g, track_id, .. } if *g == generation && track_id == "gone")),
        "{ev:?}"
    );
    assert!(!p.position().playing);
    assert!(!p.toggle(), "nothing loaded to resume");
}

fn progress_of(ev: &[(Instant, PlayerEvent)], generation: u64) -> Vec<f64> {
    ev.iter()
        .filter_map(|(_, e)| match e {
            PlayerEvent::Progress { generation: g, position, .. } if *g == generation => Some(*position),
            _ => None,
        })
        .collect()
}

/// A seek during the fade-out is about the track the caller moved on to: the
/// incoming track starts where it asked, and the outgoing one is not jumped.
#[test]
fn a_seek_during_a_fade_moves_the_incoming_track() {
    let (p, rx) = player();
    let a = p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. })).unwrap();
    let b = p.play("b".into(), fixture("bursts.wav"), 1.0, 0.5);
    std::thread::sleep(Duration::from_millis(80));
    p.seek(1.5);

    let ev = collect(&rx, Duration::from_millis(900));
    let outgoing = progress_of(&ev, a);
    assert!(outgoing.iter().all(|&pos| pos < 1.0), "the outgoing track kept its place: {outgoing:?}");
    let incoming = progress_of(&ev, b);
    assert!(incoming.first().is_some_and(|&pos| pos >= 1.5), "b starts at the seek: {incoming:?}");
}

/// The same when paused mid-fade: the incoming track waits at the seek.
#[test]
fn a_seek_then_pause_during_a_fade_loads_the_incoming_track_there() {
    let (p, rx) = player();
    p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. })).unwrap();
    let b = p.play("b".into(), fixture("bursts.wav"), 1.0, 0.5);
    std::thread::sleep(Duration::from_millis(80));
    p.seek(2.0);
    p.pause();
    let ev = collect(&rx, Duration::from_millis(300));
    let incoming = progress_of(&ev, b);
    assert!(incoming.first().is_some_and(|&pos| (pos - 2.0).abs() < 0.01), "{incoming:?}");
    assert!(!p.position().playing);
}

/// Skipping again during a fade starts the newest track from its beginning,
/// not from a seek meant for the one it replaced.
#[test]
fn a_newer_request_during_a_fade_drops_the_earlier_seek() {
    let (p, rx) = player();
    p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. })).unwrap();
    p.play("b".into(), fixture("bursts.wav"), 1.0, 0.4);
    std::thread::sleep(Duration::from_millis(50));
    p.seek(1.5);
    let c = p.play("c".into(), fixture("bursts.wav"), 1.0, 0.4);
    let ev = collect(&rx, Duration::from_millis(700));
    let incoming = progress_of(&ev, c);
    assert!(incoming.first().is_some_and(|&pos| pos < 0.1), "{incoming:?}");
}

/// cut_fade ends a fade-out at once: the outgoing track is gone and the
/// incoming one plays at its level, not from the fade's silence.
#[test]
fn cutting_a_fade_starts_the_incoming_track_at_once() {
    let (p, rx) = player();
    let a = p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. })).unwrap();
    let b = p.play("b".into(), fixture("bursts.wav"), 1.0, 2.0);
    std::thread::sleep(Duration::from_millis(100));
    let cut_at = Instant::now();
    p.cut_fade();
    let e = wait_for(
        &rx,
        Duration::from_millis(300),
        |e| matches!(e, PlayerEvent::Progress { generation, .. } if *generation == b),
    );
    assert!(e.is_some(), "b started well before the 2 s fade would have ended");
    assert!(cut_at.elapsed() < Duration::from_millis(300));
    let ev = collect(&rx, Duration::from_millis(300));
    assert!(progress_of(&ev, a).is_empty(), "nothing more from a: {ev:?}");
    assert!((p.current_gain() - 1.0).abs() < 0.01, "at its level: {}", p.current_gain());

    // With no fade under way it does nothing.
    p.cut_fade();
    let ev = collect(&rx, Duration::from_millis(300));
    assert!(
        ev.iter()
            .all(|(_, e)| matches!(e, PlayerEvent::Progress { generation, playing: true, .. } if *generation == b))
    );
}

/// A play request with no crossfade during a fade-out replaces the fade:
/// the outgoing track stops at once.
#[test]
fn an_immediate_play_during_a_fade_cuts_the_outgoing_track() {
    let (p, rx) = player();
    let a = p.play("a".into(), fixture("bursts.wav"), 1.0, 0.0);
    wait_for(&rx, Duration::from_secs(1), |e| matches!(e, PlayerEvent::Progress { playing: true, .. })).unwrap();
    let b = p.play("b".into(), fixture("bursts.wav"), 1.0, 2.0);
    std::thread::sleep(Duration::from_millis(100));
    let c = p.play("c".into(), fixture("bursts.wav"), 1.0, 0.0);
    let ev = collect(&rx, Duration::from_millis(500));
    let first_c =
        ev.iter().position(|(_, e)| matches!(e, PlayerEvent::Progress { generation, .. } if *generation == c));
    let first_c = first_c.expect("c started at once");
    assert!(progress_of(&ev[first_c..], a).is_empty(), "{ev:?}");
    assert!(progress_of(&ev, b).is_empty(), "b never played: {ev:?}");
    assert!((p.current_gain() - 1.0).abs() < 0.01, "{}", p.current_gain());
}
