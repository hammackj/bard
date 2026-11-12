//! Starts the overlay through `Services` on a free loopback port, against a
//! Bard opened on a temp data directory, and talks HTTP to it.

use bard_core::Bard;
use bard_core::CoreEvent;
use bard_core::types::NowPlaying;
use bard_services::{AppControls, Services, ServicesOptions, secrets::ObsPassword};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct NoControls;
impl AppControls for NoControls {
    fn play(&self) {}
    fn pause(&self) {}
    fn toggle(&self) {}
    fn next(&self) {}
    fn previous(&self) {}
    fn seek(&self, _: f64) {}
    fn set_volume(&self, _: f64) {}
    fn show_window(&self, _: Option<String>) {}
    fn show_popup(&self) {}
    fn quit(&self) {}
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn get(port: u16, path: &str) -> (u16, String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head.to_lowercase(), raw[split + 4..].to_vec())
}

fn start() -> (tempfile::TempDir, Arc<Mutex<Bard>>, Services, u16) {
    let dir = tempfile::tempdir().unwrap();
    let (mut bard, _events) = Bard::open(dir.path().to_path_buf()).unwrap();
    let port = free_port();
    bard.update_config(|c| c.overlay.port = port);
    let bard = Arc::new(Mutex::new(bard));
    let opts = ServicesOptions {
        obs: false,
        mpris: false,
        tray: false,
        migrate_obs_password: false,
        obs_password: ObsPassword::with_service("bard-test-overlay"),
        ..Default::default()
    };
    let services = Services::start(bard.clone(), Arc::new(NoControls), opts);
    (dir, bard, services, port)
}

#[test]
fn serves_the_page_json_art_and_events() {
    let (dir, _bard, services, port) = start();
    assert_eq!(services.overlay_port(), Some(port));
    assert_eq!(services.overlay_url(), Some(format!("http://127.0.0.1:{port}")));

    // The page, unchanged from resources/overlay.
    let (status, head, body) = get(port, "/");
    assert_eq!(status, 200);
    assert!(head.contains("content-type: text/html"));
    assert_eq!(body, std::fs::read("../../resources/overlay/index.html").unwrap());
    let (status, head, body) = get(port, "/overlay.js");
    assert_eq!(status, 200);
    assert!(head.contains("javascript"));
    assert_eq!(body, std::fs::read("../../resources/overlay/overlay.js").unwrap());
    assert_eq!(get(port, "/nope.txt").0, 404);
    assert_eq!(get(port, "/../Cargo.toml").0, 404);

    // nowplaying.json: the snapshot's shape.
    let (status, head, body) = get(port, "/nowplaying.json");
    assert_eq!(status, 200);
    assert!(head.contains("application/json"));
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    for key in ["title", "artist", "album", "durationSec", "positionSec", "isPlaying", "artUrl", "queueLength", "updatedAt"] {
        assert!(v.get(key).is_some(), "{key} in {v}");
    }
    assert_eq!(v["durationSec"], serde_json::json!(0));

    // Art: ids validated, served from the cache.
    assert_eq!(get(port, "/art/NOT-HEX").0, 400);
    assert_eq!(get(port, "/art/abc").0, 400);
    assert_eq!(get(port, "/art/0123456789abcdef").0, 404);
    let art = dir.path().join("artwork");
    std::fs::create_dir_all(&art).unwrap();
    std::fs::write(art.join("0123456789abcdef.jpg"), b"\xff\xd8jpeg").unwrap();
    let (status, head, body) = get(port, "/art/0123456789abcdef");
    assert_eq!(status, 200);
    assert!(head.contains("image/jpeg"));
    assert_eq!(body, b"\xff\xd8jpeg");

    // SSE: a first frame straight away, then one per pushed snapshot.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    let mut r = BufReader::new(s);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        head.push_str(&line.to_lowercase());
    }
    assert!(head.contains("content-type: text/event-stream"), "{head}");
    assert!(head.contains("cache-control: no-cache, no-transform"));
    assert!(head.contains("x-accel-buffering: no"));
    let data_line = |r: &mut BufReader<TcpStream>| loop {
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        if let Some(i) = line.find("data: ") {
            return serde_json::from_str::<serde_json::Value>(line[i + 6..].trim()).unwrap();
        }
    };
    let first = data_line(&mut r);
    assert_eq!(first["isPlaying"], false);
    // Wait for the server to register the client before pushing.
    for _ in 0..50 {
        if services.overlay_client_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(services.overlay_client_count(), 1);
    services.handle(&CoreEvent::Overlay(NowPlaying {
        title: "Song".into(),
        artist: "Band".into(),
        album: "Record".into(),
        duration_sec: 200.0,
        position_sec: 12.5,
        is_playing: true,
        art_url: Some(format!("http://127.0.0.1:{port}/art/0123456789abcdef")),
        queue_length: 4,
        updated_at: "2026-11-12T03:12:50.260Z".into(),
    }));
    let pushed = data_line(&mut r);
    assert_eq!(pushed["title"], "Song");
    assert_eq!(pushed["durationSec"], serde_json::json!(200));
    assert_eq!(pushed["positionSec"], 12.5);
    // And /nowplaying.json follows.
    let v: serde_json::Value = serde_json::from_slice(&get(port, "/nowplaying.json").2).unwrap();
    assert_eq!(v["title"], "Song");

    services.shutdown();
    // The port is free again once stopped.
    std::thread::sleep(Duration::from_millis(100));
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn walks_past_a_taken_port_and_reports_the_bound_one() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let (mut bard, _events) = Bard::open(dir.path().to_path_buf()).unwrap();
    bard.update_config(|c| c.overlay.port = port);
    let bard = Arc::new(Mutex::new(bard));
    let opts = ServicesOptions {
        obs: false,
        mpris: false,
        tray: false,
        migrate_obs_password: false,
        obs_password: ObsPassword::with_service("bard-test-overlay"),
        ..Default::default()
    };
    let services = Services::start(bard, Arc::new(NoControls), opts);
    let bound = services.overlay_port().unwrap();
    assert!(bound > port && bound < port + 10, "{bound} vs {port}");
    assert_eq!(get(bound, "/").0, 200);
}
