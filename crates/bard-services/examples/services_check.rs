//! Manual checks against the real desktop, none of which write to
//! ~/.config/bard:
//!
//! ```sh
//! # Decrypt the Electron build's obs-password.bin in memory; prints its length.
//! cargo run -p bard-services --example services_check -- electron-password
//! # Migrate a *copy* of it into a throwaway keyring slot, check, then delete it.
//! cargo run -p bard-services --example services_check -- migrate-copy
//! # Read-only connect to OBS (Identify + Get*Status) using that password.
//! cargo run -p bard-services --example services_check -- obs [ws://127.0.0.1:4455]
//! # MPRIS + tray on the session bus with a fake track, for N seconds.
//! cargo run -p bard-services --example services_check -- desktop [seconds]
//! ```

use bard_core::types::{NowPlaying, QueueItem, StreamerSource, Track};
use bard_core::{Bard, CoreEvent};
use bard_services::obs::{ObsHandler, ObsWatch};
use bard_services::secrets::{self, ObsPassword};
use bard_services::{AppControls, Services, ServicesOptions};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn electron_file() -> std::path::PathBuf {
    dirs_home().join(".config/bard").join(secrets::ELECTRON_FILE)
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(Into::into).expect("HOME")
}

struct Print;
impl AppControls for Print {
    fn play(&self) {
        println!("control: play")
    }
    fn pause(&self) {
        println!("control: pause")
    }
    fn toggle(&self) {
        println!("control: toggle")
    }
    fn next(&self) {
        println!("control: next")
    }
    fn previous(&self) {
        println!("control: previous")
    }
    fn seek(&self, p: f64) {
        println!("control: seek {p}")
    }
    fn set_volume(&self, v: f64) {
        println!("control: volume {v}")
    }
    fn show_window(&self, _: Option<String>) {
        println!("control: show window")
    }
    fn show_popup(&self) {
        println!("control: show popup")
    }
    fn quit(&self) {
        println!("control: quit")
    }
}

struct ObsPrint(String);
impl ObsHandler for ObsPrint {
    fn live(&self, source: StreamerSource) {
        println!("obs: live -> would arm from {source:?}");
    }
    fn status(&self, message: String) {
        println!("obs: status: {message}");
    }
    fn password(&self) -> impl Future<Output = String> + Send {
        let p = self.0.clone();
        async move { p }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("electron-password") => match secrets::read_electron(&electron_file()) {
            Ok(p) => println!("decrypted: {} characters (not shown)", p.chars().count()),
            Err(e) => println!("failed: {e}"),
        },
        Some("migrate-copy") => {
            let dir = tempfile::tempdir().unwrap();
            std::fs::copy(electron_file(), dir.path().join(secrets::ELECTRON_FILE)).unwrap();
            let slot = ObsPassword::with_service(format!("bard-migrate-check-{}", std::process::id()));
            println!("migration: {:?}", secrets::migrate_electron(dir.path(), &slot));
            let original = secrets::read_electron(&electron_file()).unwrap_or_default();
            println!("keyring now holds the same password: {}", slot.get() == original && !original.is_empty());
            println!("marker written: {}", dir.path().join(secrets::MIGRATED_MARKER).exists());
            println!("second run: {:?}", secrets::migrate_electron(dir.path(), &slot));
            slot.clear();
            println!("throwaway slot removed: {}", slot.stored() == Some(false));
        }
        Some("obs") => {
            let url = args.get(1).cloned().unwrap_or("ws://127.0.0.1:4455".into());
            let password = secrets::read_electron(&electron_file()).unwrap_or_default();
            let rt = tokio::runtime::Runtime::new().unwrap();
            let watch = ObsWatch::start(rt.handle(), url, Arc::new(ObsPrint(password)));
            std::thread::sleep(Duration::from_secs(6));
            println!("still connected/retrying: {}", !watch.is_finished());
            drop(watch);
        }
        Some("desktop") => {
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
            let dir = tempfile::tempdir().unwrap();
            let art = dir.path().join("artwork");
            std::fs::create_dir_all(&art).unwrap();
            std::fs::copy(
                concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/icons/icon-256.png"),
                art.join("0123456789abcdef.jpg"),
            )
            .unwrap();
            let (bard, _events) = Bard::open(dir.path().to_path_buf()).unwrap();
            let bard = Arc::new(Mutex::new(bard));
            let opts = ServicesOptions {
                overlay: false,
                obs: false,
                migrate_obs_password: false,
                obs_password: ObsPassword::with_service("bard-desktop-check"),
                ..Default::default()
            };
            let services = Services::start(bard, Arc::new(Print), opts);
            println!("mpris: {:?}, tray running: {}", services.mpris_bus_name(), services.tray_running());
            let track = Track {
                id: "0123456789abcdef".into(),
                path: "/music/Check/01 Check.flac".into(),
                title: "Services Check".into(),
                artist: "Bard Test".into(),
                album: "Not A Real Album".into(),
                duration_sec: 200.0,
                has_artwork: true,
                search_text: String::new(),
                track_gain_db: None,
                album_gain_db: None,
                track_peak: None,
                album_peak: None,
            };
            services.handle(&CoreEvent::NowPlaying(Some(QueueItem { id: "q".into(), track, added_at: 0 })));
            services.handle(&CoreEvent::Overlay(NowPlaying {
                title: "Services Check".into(),
                artist: "Bard Test".into(),
                album: "Not A Real Album".into(),
                duration_sec: 200.0,
                position_sec: 10.0,
                is_playing: false,
                art_url: None,
                queue_length: 1,
                updated_at: String::new(),
            }));
            std::thread::sleep(Duration::from_secs(secs));
            services.shutdown();
        }
        _ => eprintln!("usage: services_check electron-password | migrate-copy | obs [url] | desktop [secs]"),
    }
}
