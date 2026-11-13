//! MPRIS read back over D-Bus with busctl, on a private session bus so
//! nothing appears in the real desktop's media controls.
//!
//! Skipped (passes vacuously, with a note) when dbus-daemon or busctl is
//! missing.

#![cfg(target_os = "linux")]

use bard_core::types::{NowPlaying, QueueItem, Track};
use bard_core::{Bard, CoreEvent};
use bard_services::{AppControls, Services, ServicesOptions, secrets::ObsPassword};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);
impl Recorder {
    fn push(&self, s: impl Into<String>) {
        self.0.lock().unwrap().push(s.into());
    }
}
impl AppControls for Recorder {
    fn play(&self) {
        self.push("play")
    }
    fn pause(&self) {
        self.push("pause")
    }
    fn toggle(&self) {
        self.push("toggle")
    }
    fn next(&self) {
        self.push("next")
    }
    fn previous(&self) {
        self.push("previous")
    }
    fn seek(&self, p: f64) {
        self.push(format!("seek {p:.1}"))
    }
    fn set_volume(&self, v: f64) {
        self.push(format!("volume {v:.2}"))
    }
    fn show_window(&self, _: Option<String>) {
        self.push("show")
    }
    fn show_popup(&self) {
        self.push("popup")
    }
    fn quit(&self) {
        self.push("quit")
    }
}

struct Bus {
    child: Child,
    address: String,
    _dir: tempfile::TempDir,
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A bus with no service directories: the stock `--session` config would
/// auto-start ksecretd, portals and the like on it (the keyring lookup at
/// startup asks for org.freedesktop.secrets), and those outlive the test.
fn private_bus() -> Option<Bus> {
    let dir = tempfile::tempdir().ok()?;
    let config = dir.path().join("bus.conf");
    std::fs::write(
        &config,
        format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
            dir.path().display()
        ),
    )
    .ok()?;
    let mut child = Command::new("dbus-daemon")
        .arg(format!("--config-file={}", config.display()))
        .args(["--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take()?).read_line(&mut line).ok()?;
    Some(Bus { child, address: line.trim().to_owned(), _dir: dir })
}

fn busctl(bus: &Bus, args: &[&str]) -> String {
    let out = Command::new("busctl").arg(format!("--address={}", bus.address)).args(args).output().expect("busctl");
    assert!(out.status.success(), "busctl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

fn prop(bus: &Bus, name: &str, iface: &str, p: &str) -> serde_json::Value {
    let out = busctl(bus, &["--json=short", "get-property", name, PATH, iface, p]);
    serde_json::from_str::<serde_json::Value>(&out).unwrap()["data"].clone()
}

#[test]
fn metadata_status_and_controls_over_dbus() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon; skipped");
        return;
    };
    if Command::new("busctl").arg("--version").output().is_err() {
        eprintln!("no busctl; skipped");
        return;
    }
    // Before anything touches D-Bus. This test binary runs only this test.
    unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &bus.address) };

    let dir = tempfile::tempdir().unwrap();
    let art = dir.path().join("artwork");
    std::fs::create_dir_all(&art).unwrap();
    std::fs::write(art.join("0123456789abcdef.jpg"), b"\xff\xd8").unwrap();
    let (bard, _events) = Bard::open(dir.path().to_path_buf()).unwrap();
    let bard = Arc::new(Mutex::new(bard));
    let rec = Arc::new(Recorder::default());
    let suffix = format!("bard.test{}", std::process::id());
    let opts = ServicesOptions {
        overlay: false,
        obs: false,
        tray: false,
        migrate_obs_password: false,
        mpris_bus_suffix: suffix.clone(),
        obs_password: ObsPassword::with_service("bard-test-mpris"),
        ..Default::default()
    };
    let services = Services::start(bard, rec.clone(), opts);
    let name = format!("org.mpris.MediaPlayer2.{suffix}");
    assert_eq!(services.mpris_bus_name(), Some(name.as_str()));

    // Nothing playing.
    assert_eq!(prop(&bus, &name, "org.mpris.MediaPlayer2", "Identity"), "Bard");
    assert_eq!(prop(&bus, &name, "org.mpris.MediaPlayer2", "DesktopEntry"), "bard");
    assert_eq!(prop(&bus, &name, PLAYER, "PlaybackStatus"), "Stopped");

    let track = Track {
        id: "0123456789abcdef".into(),
        path: "/music/Some Band/01 Song.flac".into(),
        title: "Song Title".into(),
        artist: "Some Band".into(),
        album: "The Album".into(),
        duration_sec: 245.5,
        has_artwork: true,
        search_text: String::new(),
        track_gain_db: None,
        album_gain_db: None,
        track_peak: None,
        album_peak: None,
    };
    let item = QueueItem { id: "q1".into(), track, added_at: 0 };
    services.handle(&CoreEvent::NowPlaying(Some(item)));
    services.handle(&CoreEvent::Overlay(NowPlaying {
        title: "Song Title".into(),
        artist: "Some Band".into(),
        album: "The Album".into(),
        duration_sec: 245.5,
        position_sec: 30.0,
        is_playing: true,
        art_url: None,
        queue_length: 3,
        updated_at: String::new(),
    }));
    std::thread::sleep(Duration::from_millis(100));

    let md = prop(&bus, &name, PLAYER, "Metadata");
    eprintln!("Metadata over D-Bus: {md}");
    assert_eq!(md["xesam:title"]["data"], "Song Title");
    assert_eq!(md["xesam:artist"]["data"], serde_json::json!(["Some Band"]));
    assert_eq!(md["xesam:album"]["data"], "The Album");
    assert_eq!(md["mpris:length"]["data"], 245_500_000);
    let art_url = format!("file://{}/0123456789abcdef.jpg", art.display());
    assert_eq!(md["mpris:artUrl"]["data"], art_url.as_str());
    assert_eq!(md["mpris:trackid"]["data"], "/org/bard/track/0123456789abcdef");
    assert_eq!(prop(&bus, &name, PLAYER, "PlaybackStatus"), "Playing");
    let pos = prop(&bus, &name, PLAYER, "Position").as_i64().unwrap();
    assert!((30_000_000..32_000_000).contains(&pos), "{pos}");
    for p in ["CanGoNext", "CanGoPrevious", "CanPlay", "CanPause", "CanSeek", "CanControl"] {
        assert_eq!(prop(&bus, &name, PLAYER, p), true, "{p}");
    }
    assert_eq!(prop(&bus, &name, PLAYER, "Volume"), 0.8);

    // Controls reach the app.
    for m in ["PlayPause", "Next", "Previous", "Pause", "Play"] {
        busctl(&bus, &["call", &name, PATH, PLAYER, m]);
    }
    busctl(&bus, &["call", &name, PATH, PLAYER, "SetPosition", "ox", "/org/bard/track/0123456789abcdef", "60000000"]);
    // Stale track id: ignored.
    busctl(&bus, &["call", &name, PATH, PLAYER, "SetPosition", "ox", "/org/bard/track/ffffffffffffffff", "1000000"]);
    busctl(&bus, &["set-property", &name, PATH, PLAYER, "Volume", "d", "0.5"]);
    busctl(&bus, &["call", &name, PATH, "org.mpris.MediaPlayer2", "Raise"]);
    let got = rec.0.lock().unwrap().clone();
    assert_eq!(got, ["toggle", "next", "previous", "pause", "play", "seek 60.0", "volume 0.50", "show"]);

    // Volume follows config; stopping clears the metadata.
    let mut config = bard_core::types::Config::default();
    config.audio.volume = 0.5;
    services.handle(&CoreEvent::ConfigChanged(config));
    services.handle(&CoreEvent::NowPlaying(None));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(prop(&bus, &name, PLAYER, "Volume"), 0.5);
    assert_eq!(prop(&bus, &name, PLAYER, "PlaybackStatus"), "Stopped");
    let md = prop(&bus, &name, PLAYER, "Metadata");
    assert_eq!(md["mpris:trackid"]["data"], "/org/mpris/MediaPlayer2/TrackList/NoTrack");
    assert!(md.get("xesam:title").is_none());

    services.shutdown();
}
