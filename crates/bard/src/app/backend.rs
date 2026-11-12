//! The window's intents, carried out against the core and the engine.

use super::AppEvent;
use crate::model::Backend;
use bard_audio::Player;
use bard_core::Bard;
use bard_core::types::{AppPaths, Config, ContentIdStatus};
use bard_services::Services;
use futures::channel::mpsc::UnboundedSender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Shared with the core-event thread, which feeds every event to the
/// services; taken out on quit to shut them down.
pub(super) type ServicesSlot = Arc<Mutex<Option<Services>>>;

pub struct AppBackend {
    pub bard: Arc<Mutex<Bard>>,
    pub player: Arc<Player>,
    pub(super) events: UnboundedSender<AppEvent>,
    pub(super) services: ServicesSlot,
    pub(super) scans: Arc<Mutex<ScanState>>,
}

/// One scan at a time: a request while one runs is remembered and run once
/// it finishes, so a burst of requests is one more scan, not a pool each.
#[derive(Default)]
pub(super) struct ScanState {
    running: bool,
    again: bool,
}

impl AppBackend {
    pub(super) fn with<R>(&self, f: impl FnOnce(&mut Bard) -> R) -> R {
        f(&mut self.bard.lock().expect("core lock"))
    }

    /// Scans on a worker thread: a scan takes seconds and must not hold the
    /// core's lock -- or the GPUI thread -- while it runs.
    ///
    /// A request while a scan runs is folded into one more scan after it,
    /// taken then, so it sees the folders as they are by then. The core
    /// drops any result that is no longer the newest (see
    /// `Bard::install_scan`).
    pub fn scan(&self) {
        {
            let mut st = self.scans.lock().expect("scan lock");
            if st.running {
                st.again = true;
                return;
            }
            st.running = true;
        }
        let bard = self.bard.clone();
        let events = self.events.clone();
        let scans = self.scans.clone();
        std::thread::Builder::new()
            .name("bard-scan".into())
            .spawn(move || {
                loop {
                    let scanner = bard.lock().expect("core lock").scanner();
                    let tracks = scanner.run();
                    bard.lock().expect("core lock").install_scan(&scanner, tracks);
                    let mut st = scans.lock().expect("scan lock");
                    if !std::mem::take(&mut st.again) {
                        st.running = false;
                        break;
                    }
                }
                let _ = events.unbounded_send(AppEvent::Scanned);
            })
            .expect("scan thread");
    }
}

impl Backend for AppBackend {
    fn enqueue(&self, id: &str) {
        self.with(|b| b.enqueue(id));
    }
    fn enqueue_many(&self, ids: &[String], start_now: bool) {
        self.with(|b| b.enqueue_many(ids, start_now));
    }
    fn play_now(&self, id: &str) {
        self.with(|b| b.play_now(id));
    }
    fn remove_from_queue(&self, id: &str) {
        self.with(|b| b.remove_from_queue(id));
    }
    fn move_queue_item(&self, item: &str, onto: &str) {
        self.with(|b| b.move_queue_item(item, onto));
    }
    fn skip(&self) {
        self.with(|b| b.skip());
    }
    fn previous(&self) {
        self.with(|b| b.previous());
    }
    fn clear_queue(&self) {
        self.with(|b| b.clear_queue());
    }
    fn shuffle_queue(&self) {
        self.with(|b| b.shuffle_queue());
    }
    fn fill_queue(&self) {
        self.with(|b| b.fill_queue(None));
    }
    /// The one transport toggle, shared by the button, Space, the tray and
    /// the media keys. With nothing loaded it has to start something: endless
    /// keeps the queue topped up but never selects a track itself, so play
    /// against an empty engine would do nothing. skip() takes the head of the
    /// queue and honours auto-fill.
    fn toggle_play(&self) {
        if self.player.position().duration > 0. || self.with(|b| b.now_playing().is_some()) {
            self.player.toggle();
        } else {
            self.skip();
        }
    }
    fn seek(&self, seconds: f64) {
        self.player.seek(seconds);
    }
    fn set_volume(&self, v: f64) {
        self.player.set_volume(v as f32);
        // Heard at once; saved behind, so a drag never waits on the disk.
        self.with(|b| b.adjust_config(|c| c.audio.volume = v));
    }
    fn update_config(&self, f: Box<dyn FnOnce(&mut Config)>) {
        self.with(|b| b.update_config(f));
    }
    fn adjust_config(&self, f: Box<dyn FnOnce(&mut Config)>) {
        self.with(|b| b.adjust_config(f));
    }
    fn set_album_flag(&self, album: &str, status: ContentIdStatus, note: Option<String>) {
        self.with(|b| b.set_album_flag(album, status, note.as_deref()));
    }
    fn toggle_loved(&self, id: &str) {
        self.with(|b| b.toggle_loved(id));
    }
    fn rescan(&self) {
        self.scan();
    }
    fn choose_library_folder(&self) {
        let _ = self.events.unbounded_send(AppEvent::ChooseFolder);
    }
    fn outputs(&self) -> Vec<(String, String)> {
        self.player.outputs().into_iter().map(|d| (d.id, d.name)).collect()
    }
    fn set_output(&self, id: &str) -> Result<(), String> {
        self.player.set_output(id).map_err(|e| e.to_string())
    }
    fn paths(&self) -> Option<AppPaths> {
        Some(self.with(|b| b.paths()))
    }
    /// Only the paths Settings lists may be opened: this hands a path to the
    /// desktop's file manager, and nothing else should be able to.
    fn reveal(&self, path: &str) {
        let paths = self.with(|b| b.paths());
        let allowed = [&paths.config_dir, &paths.config, &paths.artwork, &paths.now_playing, &paths.logs];
        if !allowed.iter().any(|p| p.as_str() == path) {
            return;
        }
        let target = std::path::Path::new(path);
        let dir = if target.is_dir() { target } else { target.parent().unwrap_or(target) };
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
    /// The deadline runs on its own thread, not in the window: the timer
    /// has to fire with the window closed to the tray, and Electron fired it
    /// from its own timeout so a stalled render could not delay the pause.
    fn schedule_sleep(&self, at: Instant) {
        let events = self.events.clone();
        let _ = std::thread::Builder::new().name("bard-sleep".into()).spawn(move || {
            std::thread::sleep(at.saturating_duration_since(Instant::now()));
            let _ = events.unbounded_send(AppEvent::SleepDue(at));
        });
    }
    fn set_obs_password(&self, password: &str) {
        let events = self.events.clone();
        let chars = password.chars().count();
        let done = move |stored| {
            let _ = events.unbounded_send(AppEvent::ObsSaved { stored, chars });
        };
        match self.services.lock().expect("services lock").as_ref() {
            Some(s) => s.set_obs_password(password.to_string(), done),
            None => done(false),
        }
    }
}

impl super::reduce::Effects for AppBackend {
    fn play(&self, t: &bard_core::types::Track, norm_gain: f32, crossfade_sec: f32) -> u64 {
        self.player.play(t.id.clone(), std::path::PathBuf::from(&t.path), norm_gain, crossfade_sec)
    }
    fn stop(&self) -> u64 {
        self.player.stop()
    }
    fn cut_fade(&self) {
        self.player.cut_fade()
    }
    fn load_paused(&self, t: &bard_core::types::Track, norm_gain: f32) -> u64 {
        self.player.load_paused(t.id.clone(), std::path::PathBuf::from(&t.path), norm_gain)
    }
    fn playing(&self) -> bool {
        self.player.is_playing()
    }
    fn held(&self) -> Option<u64> {
        self.player.held()
    }
    fn pause(&self) {
        self.player.pause()
    }
    fn seek(&self, seconds: f64) {
        self.player.seek(seconds)
    }
    fn set_norm_gain(&self, generation: u64, gain: f32) {
        self.player.set_norm_gain(generation, gain)
    }
    fn set_output(&self, id: &str) -> Result<(), String> {
        Backend::set_output(self, id)
    }
    fn toggle_play(&self) {
        Backend::toggle_play(self)
    }
    fn skip(&self) {
        Backend::skip(self)
    }
    fn previous(&self) {
        Backend::previous(self)
    }
    fn set_volume(&self, volume: f64) {
        Backend::set_volume(self, volume)
    }
    fn update_config(&self, f: Box<dyn FnOnce(&mut Config)>) {
        Backend::update_config(self, f)
    }
    fn scan(&self) {
        AppBackend::scan(self)
    }
    fn report_progress(&self, p: bard_core::types::PlaybackProgress) {
        self.with(|b| b.report_progress(p))
    }
    fn report_ended(&self) {
        self.with(|b| b.report_ended())
    }
    fn report_failed(&self, give_up: bool) {
        self.with(|b| b.report_failed(give_up))
    }
    fn stats(&self) -> bard_core::types::StatsMap {
        self.with(|b| b.stats())
    }
    fn album_flags(&self) -> bard_core::types::AlbumFlags {
        self.with(|b| b.album_flags())
    }
    fn loved(&self) -> Vec<String> {
        self.with(|b| b.loved())
    }
    fn tracks(&self) -> Vec<bard_core::types::Track> {
        self.with(|b| b.tracks().to_vec())
    }
    fn queue(&self) -> Vec<bard_core::types::QueueItem> {
        self.with(|b| b.queue().to_vec())
    }
}
