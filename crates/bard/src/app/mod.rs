//! Wiring: the core, the audio engine and the window.
//!
//! This replaces the Electron app's preload bridge and the two halves of IPC.
//! The core owns the queue and decides what plays; the audio engine plays it
//! and reports back; the window renders both. Events from the core and the
//! engine arrive on their own threads and are forwarded to the GPUI thread,
//! where they update the window's model -- nothing here blocks rendering.
//!
//! The services (overlay server, MPRIS, tray, OBS, keyring) run on their own
//! runtime in `bard-services`. Core events are fed to them on the forwarding
//! thread, before they reach the window; their requests (media keys, tray
//! menu) come back as `AppEvent::Control`.
//!
//! The window can close while the app keeps playing (close to tray). GPUI
//! cannot hide a window on Linux, so it really closes: its `Ui` is parked in
//! the `Shell` and a new window is built from it when the tray asks.

use crate::model::Ui;
use crate::views::string_bar::Analyser as StringAnalyser;
use backend::{AppBackend, ServicesSlot};
use bard_audio::{Player, PlayerEvent, PlayerOptions};
use bard_core::{Bard, CoreEvent};
use bard_services::{AppControls, Services};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::*;
use shell::{Shell, ShellRef, dispatch, show_main};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

mod backend;
mod launch;
#[cfg(test)]
mod live_tests;
mod output;
mod reduce;
mod shell;

pub use launch::Launch;

// CoreEvent carries a whole config or track, so it is the large variant.
// These go through a channel a few times a second at most, and boxing it
// would cost the nested `AppEvent::Core(CoreEvent::…)` patterns in `reduce`.
#[allow(clippy::large_enum_variant)]
pub enum AppEvent {
    Core(CoreEvent),
    Player(PlayerEvent),
    Scanned,
    /// Open the folder picker; it needs the App context, so it goes through the loop.
    ChooseFolder,
    Rescan,
    /// From MPRIS, the media keys and the tray.
    Control(Control),
    /// A sleep timer armed for this deadline has run out.
    SleepDue(Instant),
    /// The saved output device appeared: the saved id and the sink's id,
    /// which differ when the setting came from Electron.
    OutputAppeared {
        saved: String,
        id: String,
    },
    /// The OBS password save finished: whether it was stored, and its length.
    ObsSaved {
        stored: bool,
        chars: usize,
    },
}

#[derive(Debug, Clone)]
pub enum Control {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek(f64),
    SetVolume(f64),
    /// With the xdg-activation token the request carried, if any.
    ShowWindow(Option<String>),
    ShowPopup,
    Quit,
}

/// `AppControls` for the services: everything is posted to the GPUI thread.
struct Controls(UnboundedSender<AppEvent>);

impl Controls {
    fn send(&self, c: Control) {
        let _ = self.0.unbounded_send(AppEvent::Control(c));
    }
}

impl AppControls for Controls {
    fn play(&self) {
        self.send(Control::Play)
    }
    fn pause(&self) {
        self.send(Control::Pause)
    }
    fn toggle(&self) {
        self.send(Control::Toggle)
    }
    fn next(&self) {
        self.send(Control::Next)
    }
    fn previous(&self) {
        self.send(Control::Previous)
    }
    fn seek(&self, position_sec: f64) {
        self.send(Control::Seek(position_sec))
    }
    fn set_volume(&self, volume: f64) {
        self.send(Control::SetVolume(volume))
    }
    fn show_window(&self, activation_token: Option<String>) {
        self.send(Control::ShowWindow(activation_token))
    }
    fn show_popup(&self) {
        self.send(Control::ShowPopup)
    }
    fn quit(&self) {
        self.send(Control::Quit)
    }
}

pub fn open(cx: &mut App, launch: Launch) -> anyhow::Result<()> {
    let (bard, core_rx) = Bard::open(launch.data_dir.clone())?;
    let (player, player_rx) = Player::with_options(PlayerOptions { silent: launch.silent, ..Default::default() })?;
    let player = Arc::new(player);
    let config = bard.config().clone();
    let paths = bard.paths();
    let bard = Arc::new(Mutex::new(bard));

    let (tx, mut rx) = unbounded::<AppEvent>();
    let services: ServicesSlot = Arc::new(Mutex::new(None));
    {
        let tx = tx.clone();
        let services = services.clone();
        std::thread::Builder::new().name("bard-core-events".into()).spawn(move || {
            for ev in core_rx {
                // Never blocks and never takes the core lock.
                if let Some(s) = services.lock().expect("services lock").as_ref() {
                    s.handle(&ev);
                }
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
    let config = output::restore(&player, &bard, &tx, config, &paths.config_dir)?;

    let backend = Rc::new(AppBackend {
        bard: bard.clone(),
        player: player.clone(),
        events: tx.clone(),
        services: services.clone(),
        scans: Default::default(),
    });
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
    ui.commit = env!("BARD_COMMIT").to_string();
    let analyser = player.analyser();
    let read: StringAnalyser = Rc::new(move |out: &mut [u8]| analyser.time_domain(out));
    ui.string.analyser = Some(read);

    // Started before the window so Settings has the overlay URL from the
    // first frame. Must not be called with the core lock held.
    // Logout, shutdown and `kill` end the process with a signal. Quit
    // properly instead, as Electron's before-quit did: flush stats and loved
    // tracks, and write a stopped nowplaying.json -- or the overlay keeps
    // showing a track that stopped with us.
    {
        let tx = tx.clone();
        if let Err(e) = ctrlc::set_handler(move || {
            let _ = tx.unbounded_send(AppEvent::Control(Control::Quit));
        }) {
            eprintln!("[bard] signal handler: {e}");
        }
    }

    let started = Services::start(bard.clone(), Arc::new(Controls(tx)), launch.services());
    ui.overlay_url = started.overlay_url();
    *services.lock().expect("services lock") = Some(started);

    if !config.library_paths.is_empty() {
        ui.scanning = true;
        backend.scan();
    }

    // Closing is only a request; Shell decides between hiding and quitting.
    cx.set_quit_mode(QuitMode::Explicit);
    let shell: ShellRef = Rc::new(RefCell::new(Shell {
        backend: backend.clone(),
        main: None,
        view: None,
        parked: Some(ui),
        popup: None,
        popup_generation: 0,
        now: None,
        artwork_dir: PathBuf::from(&paths.artwork),
        quitting: false,
    }));
    show_main(&shell, None, cx);

    cx.spawn(async move |cx| {
        while let Some(ev) = rx.next().await {
            let shell = shell.clone();
            cx.update(|cx| dispatch(&shell, ev, cx));
        }
    })
    .detach();
    Ok(())
}
