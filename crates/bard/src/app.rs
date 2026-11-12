//! Wiring: the core, the audio engine and the window.
//!
//! This replaces the Electron app's preload bridge and the two halves of IPC.
//! The core owns the queue and decides what plays; the audio engine plays it
//! and reports back; the window renders both. Events from the core and the
//! engine arrive on their own threads and are forwarded to the GPUI thread,
//! where they update the window's model -- nothing here blocks rendering.

use crate::model::{Backend, Ui};
use crate::views::root::BardWindow;
use crate::views::string_bar::Analyser as StringAnalyser;
use bard_audio::{Player, PlayerEvent, PlayerOptions};
use bard_core::types::{
    Config, ContentIdStatus, PlaybackProgress, TransportAction,
};
use bard_core::{Bard, CoreEvent, gain_multiplier};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::*;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub enum AppEvent {
    Core(CoreEvent),
    Player(PlayerEvent),
    Scanned,
}

/// The window's intents, carried out against the core and the engine.
pub struct AppBackend {
    pub bard: Arc<Mutex<Bard>>,
    pub player: Arc<Player>,
    events: UnboundedSender<AppEvent>,
}

impl AppBackend {
    fn with<R>(&self, f: impl FnOnce(&mut Bard) -> R) -> R {
        f(&mut self.bard.lock().expect("core lock"))
    }

    /// Scans on a worker thread: a scan takes seconds and must not hold the
    /// core's lock -- or the GPUI thread -- while it runs.
    pub fn scan(&self) {
        let scanner = self.with(|b| b.scanner());
        let bard = self.bard.clone();
        let events = self.events.clone();
        std::thread::Builder::new()
            .name("bard-scan".into())
            .spawn(move || {
                let tracks = scanner.run();
                bard.lock().expect("core lock").install_library(tracks);
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
    fn reorder_queue(&self, from: usize, to: usize) {
        self.with(|b| b.reorder_queue(from, to));
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
        self.with(|b| b.update_config(|c| c.audio.volume = v));
    }
    fn update_config(&self, f: Box<dyn FnOnce(&mut Config)>) {
        self.with(|b| b.update_config(f));
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
        // Settings and first run share this; the folder picker is wired
        // with the settings panel.
    }
}

pub struct Launch {
    pub data_dir: PathBuf,
    pub silent: bool,
}

impl Launch {
    pub fn from_env() -> Self {
        Self {
            data_dir: std::env::var_os("BARD_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(bard_core::default_data_dir),
            silent: std::env::var_os("BARD_SILENT").is_some(),
        }
    }
}

pub fn open(cx: &mut App, launch: Launch) -> anyhow::Result<()> {
    let (bard, core_rx) = Bard::open(launch.data_dir.clone())?;
    let (player, player_rx) = Player::with_options(PlayerOptions {
        silent: launch.silent,
        ..Default::default()
    })?;
    let player = Arc::new(player);
    let config = bard.config().clone();
    let paths = bard.paths();
    let bard = Arc::new(Mutex::new(bard));

    let (tx, mut rx) = unbounded::<AppEvent>();
    {
        let tx = tx.clone();
        std::thread::Builder::new().name("bard-core-events".into()).spawn(move || {
            for ev in core_rx {
                if tx.unbounded_send(AppEvent::Core(ev)).is_err() {
                    break;
                }
            }
        })?;
    }
    {
        let tx = tx.clone();
        std::thread::Builder::new().name("bard-player-events".into()).spawn(move || {
            for ev in player_rx {
                if tx.unbounded_send(AppEvent::Player(ev)).is_err() {
                    break;
                }
            }
        })?;
    }

    player.set_volume(config.audio.volume as f32);
    // A saved device that is gone falls back to the default for this
    // session only, and the setting is kept: a sink can be late to appear at
    // startup, and forgetting the user's choice over that would quietly move
    // music onto the wrong recording track. The engine reports the fallback
    // as an Error event, which becomes the notice.
    if config.audio.output_device_id != "default" {
        let _ = player.set_output(&config.audio.output_device_id);
    }

    let backend = Rc::new(AppBackend { bard: bard.clone(), player: player.clone(), events: tx });
    let mut ui = Ui::new(backend.clone(), PathBuf::from(&paths.artwork));
    ui.config = config.clone();
    ui.volume = config.audio.volume;
    {
        let b = bard.lock().expect("core lock");
        ui.stats = b.stats();
        ui.album_flags = b.album_flags();
        ui.loved = b.loved();
        ui.streamer_source = b.streamer_source();
    }
    ui.commit = option_env!("BARD_COMMIT").unwrap_or("dev").to_string();
    let analyser = player.analyser();
    let read: StringAnalyser = Rc::new(move |out: &mut [u8]| analyser.time_domain(out));
    ui.string.analyser = Some(read);

    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(1000.), px(640.))),
            titlebar: Some(TitlebarOptions { title: Some("Bard".into()), ..Default::default() }),
            app_id: Some("bard".into()),
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        |_, cx| cx.new(|_| BardWindow::new(ui)),
    )?;
    let view = window.update(cx, |_, _, cx| cx.entity())?;

    if !config.library_paths.is_empty() {
        view.update(cx, |w, _| w.ui.scanning = true);
        backend.scan();
    }

    let weak = view.downgrade();
    cx.spawn(async move |cx| {
        while let Some(ev) = rx.next().await {
            if weak.update(cx, |w, cx| apply(w, &backend, ev, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    Ok(())
}

fn apply(w: &mut BardWindow, backend: &AppBackend, ev: AppEvent, cx: &mut Context<BardWindow>) {
    let ui = &mut w.ui;
    match ev {
        AppEvent::Scanned => {
            let b = backend.bard.lock().expect("core lock");
            ui.tracks = b.tracks().to_vec();
            ui.queue = b.queue().to_vec();
            ui.scanning = false;
            ui.scan_progress = None;
            drop(b);
            // Dev aid: start playback once the library is in, so a change can
            // be checked end to end without clicking.
            if std::env::var_os("BARD_AUTOPLAY").is_some() {
                backend.toggle_play();
            }
        }
        AppEvent::Core(ev) => match ev {
            CoreEvent::QueueChanged(q) => ui.queue = q,
            CoreEvent::PlayTrack(t) => {
                let gain = gain_multiplier(&t, ui.config.audio.normalize) as f32;
                backend.player.play(
                    t.id.clone(),
                    PathBuf::from(&t.path),
                    gain,
                    ui.config.audio.crossfade_sec as f32,
                );
                ui.track = Some(t);
                ui.position = 0.;
                ui.position_at = Instant::now();
            }
            // Only the null case matters here: a non-null now-playing is
            // always followed by PlayTrack, which is what loads the audio.
            // Without this the window kept the finished track's title and
            // cover while the overlay had already gone blank.
            CoreEvent::NowPlaying(None) => {
                backend.player.stop();
                ui.track = None;
                ui.position = 0.;
                ui.duration = 0.;
                ui.playing = false;
            }
            CoreEvent::NowPlaying(Some(_)) => {}
            CoreEvent::ScanProgress(p) => {
                ui.scanning = true;
                ui.scan_progress = Some(p);
            }
            CoreEvent::Overlay(_) => {}
            CoreEvent::StatsChanged => ui.stats = backend.bard.lock().expect("core lock").stats(),
            CoreEvent::AlbumFlagsChanged => {
                ui.album_flags = backend.bard.lock().expect("core lock").album_flags()
            }
            CoreEvent::LovedChanged => ui.loved = backend.bard.lock().expect("core lock").loved(),
            CoreEvent::Transport(TransportAction::Toggle) => backend.toggle_play(),
            CoreEvent::Transport(TransportAction::Restart) => backend.player.seek(0.),
            CoreEvent::Notice(msg) => ui.notice = Some(msg),
            CoreEvent::ConfigChanged(c) => {
                // Normalisation is a property of playback, not of a track, so
                // it follows the setting rather than being applied at play time.
                if c.audio.normalize != ui.config.audio.normalize {
                    if let Some(t) = &ui.track {
                        backend.player.set_norm_gain(gain_multiplier(t, c.audio.normalize) as f32);
                    }
                }
                ui.config = c;
            }
            CoreEvent::StreamerSource(s) => ui.streamer_source = s,
        },
        AppEvent::Player(ev) => match ev {
            PlayerEvent::Progress { position, duration, playing, track_id } => {
                ui.position = position;
                ui.duration = duration;
                ui.playing = playing;
                ui.position_at = Instant::now();
                backend.with(|b| b.report_progress(PlaybackProgress { position, duration, playing, track_id }));
            }
            PlayerEvent::Ended => {
                ui.playing = false;
                backend.with(|b| b.report_ended());
            }
            PlayerEvent::Error(msg) => ui.notice = Some(msg),
        },
    }
    cx.notify();
}
