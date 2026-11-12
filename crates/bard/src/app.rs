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

use crate::model::{Backend, Ui};
use crate::views::root::BardWindow;
use crate::views::string_bar::Analyser as StringAnalyser;
use bard_audio::{Player, PlayerEvent, PlayerOptions};
use bard_core::types::{
    AppPaths, Config, ContentIdStatus, PlaybackProgress, TransportAction,
};
use bard_core::{Bard, CoreEvent, gain_multiplier};
use bard_services::{AppControls, Services, ServicesOptions};
use crate::views::popup::{self, Popup};
use std::cell::RefCell;
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
    /// Open the folder picker; it needs the App context, so it goes through the loop.
    ChooseFolder,
    Rescan,
    /// From MPRIS, the media keys and the tray.
    Control(Control),
    /// A sleep timer armed for this deadline has run out.
    SleepDue(Instant),
    /// The OBS password save finished: whether it was stored, and its length.
    ObsSaved { stored: bool, chars: usize },
}

#[derive(Debug, Clone, Copy)]
pub enum Control {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek(f64),
    SetVolume(f64),
    ShowWindow,
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
    fn show_window(&self) {
        self.send(Control::ShowWindow)
    }
    fn show_popup(&self) {
        self.send(Control::ShowPopup)
    }
    fn quit(&self) {
        self.send(Control::Quit)
    }
}

/// Shared with the core-event thread, which feeds every event to the
/// services; taken out on quit to shut them down.
type ServicesSlot = Arc<Mutex<Option<Services>>>;

/// The window's intents, carried out against the core and the engine.
pub struct AppBackend {
    pub bard: Arc<Mutex<Bard>>,
    pub player: Arc<Player>,
    events: UnboundedSender<AppEvent>,
    services: ServicesSlot,
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

pub struct Launch {
    pub data_dir: PathBuf,
    pub silent: bool,
    /// Running on a scratch data dir (`BARD_DATA_DIR`): the services must not
    /// touch what the installed Bard owns -- its keyring slot, the Electron
    /// password file, OBS -- and take their own MPRIS name.
    pub dev: bool,
}

impl Launch {
    pub fn from_env() -> Self {
        let custom = std::env::var_os("BARD_DATA_DIR");
        Self {
            dev: custom.is_some(),
            data_dir: custom.map(PathBuf::from).unwrap_or_else(bard_core::default_data_dir),
            silent: std::env::var_os("BARD_SILENT").is_some(),
        }
    }

    /// The MPRIS name this instance owns, and a second launch looks for.
    /// Dev runs take one per data dir: one instance per set of files is what
    /// single-instance protects, and two scratch copies may run side by side.
    fn mpris_suffix(&self) -> String {
        if !self.dev {
            return bard_services::mpris::BUS_SUFFIX.into();
        }
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.data_dir.hash(&mut h);
        // A bus name element may not start with a digit.
        format!("bard.dev_{:08x}", h.finish() as u32)
    }

    /// Single instance, as Electron's `requestSingleInstanceLock`: when a
    /// Bard is already running, ask it to show its window and report true, so
    /// this launch can exit before it opens the library -- two processes
    /// writing the same stats and queue would lose one's writes.
    ///
    /// The running instance is found by its MPRIS name, which it owns for as
    /// long as it runs, and raised with MPRIS `Raise` -- the same request the
    /// desktop's media controls make.
    pub fn raise_running(&self) -> bool {
        let name = format!("org.mpris.MediaPlayer2.{}", self.mpris_suffix());
        let raise = || -> zbus::Result<bool> {
            let conn = zbus::blocking::Connection::session()?;
            let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
            let bus_name = zbus::names::BusName::try_from(name.as_str())?;
            if !dbus.name_has_owner(bus_name)? {
                return Ok(false);
            }
            conn.call_method(Some(name.as_str()), "/org/mpris/MediaPlayer2", Some("org.mpris.MediaPlayer2"), "Raise", &())?;
            Ok(true)
        };
        // No session bus, or the call failed: carry on and start normally.
        raise().unwrap_or(false)
    }

    fn services(&self) -> ServicesOptions {
        if self.dev {
            ServicesOptions {
                mpris_bus_suffix: self.mpris_suffix(),
                obs_password: bard_services::secrets::ObsPassword::with_service("bard-dev"),
                migrate_obs_password: false,
                obs: std::env::var_os("BARD_OBS").is_some(),
                ..Default::default()
            }
        } else {
            ServicesOptions::default()
        }
    }
}

/// The app's windows and what outlives them.
struct Shell {
    backend: Rc<AppBackend>,
    main: Option<WindowHandle<BardWindow>>,
    /// The window's root view. Reached through the entity rather than the
    /// handle: GPUI takes a window out of its registry while that window's
    /// callbacks run, so the handle cannot be used from inside on-close.
    view: Option<WeakEntity<BardWindow>>,
    /// The window's model while the window is closed to the tray.
    parked: Option<Ui>,
    popup: Option<WindowHandle<Popup>>,
    /// Bumped on every show, so an older auto-hide timer does nothing.
    popup_generation: u64,
    /// The last snapshot, for a popup opened between updates.
    now: Option<bard_core::types::NowPlaying>,
    artwork_dir: PathBuf,
    quitting: bool,
}

type ShellRef = Rc<RefCell<Shell>>;

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
    // A saved device that is gone falls back to the default for this
    // session only, and the setting is kept: a sink can be late to appear at
    // startup, and forgetting the user's choice over that would quietly move
    // music onto the wrong recording track. The engine reports the fallback
    // as an Error event, which becomes the notice.
    if config.audio.output_device_id != "default" {
        let _ = player.set_output(&config.audio.output_device_id);
    }

    let backend = Rc::new(AppBackend {
        bard: bard.clone(),
        player: player.clone(),
        events: tx.clone(),
        services: services.clone(),
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
    show_main(&shell, cx);

    cx.spawn(async move |cx| {
        while let Some(ev) = rx.next().await {
            let shell = shell.clone();
            cx.update(|cx| dispatch(&shell, ev, cx));
        }
    })
    .detach();
    Ok(())
}

/// Opens the main window from the parked model, or raises it if it is open.
fn show_main(shell: &ShellRef, cx: &mut App) {
    if let Some(handle) = shell.borrow().main {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let Some(ui) = shell.borrow_mut().parked.take() else { return };
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(1000.), px(640.))),
            titlebar: Some(TitlebarOptions { title: Some("Bard".into()), ..Default::default() }),
            app_id: Some("bard".into()),
            window_background: WindowBackgroundAppearance::Opaque,
            ..Default::default()
        },
        |window, cx| cx.new(|cx| BardWindow::new(ui, window, cx)),
    );
    let handle = match opened {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[bard] could not open the window: {e:#}");
            return;
        }
    };
    let s = shell.clone();
    let view = handle.update(cx, move |_, window, cx| {
        window.on_window_should_close(cx, move |_, cx| on_close(&s, cx));
        cx.entity().downgrade()
    });
    let mut s = shell.borrow_mut();
    s.main = Some(handle);
    s.view = view.ok();
}

/// The window's close button. Closing hides to the tray by default -- a
/// streamer will hit the X by accident mid-stream, and quitting the music is
/// not a recoverable mistake -- but only when there is a tray to come back
/// from; without one, close quits.
fn on_close(shell: &ShellRef, cx: &mut App) -> bool {
    let (view, quitting) = {
        let s = shell.borrow();
        (s.view.clone(), s.quitting)
    };
    let Some(view) = view.and_then(|v| v.upgrade()) else { return true };
    let to_tray = !quitting && view.read(cx).ui.config.window.close_to_tray && tray_running(shell);
    if !to_tray {
        quit(shell, cx);
        return true;
    }
    let backend = shell.borrow().backend.clone();
    let parked = view.update(cx, |w, _| {
        let blank = Ui::new(backend.clone() as Rc<dyn Backend>, PathBuf::new());
        let mut ui = std::mem::replace(&mut w.ui, blank);
        // Transient UI does not survive the window, as it did not in Electron's
        // reloaded panels.
        ui.menu_open = false;
        ui.sleep_open = false;
        ui.open_select = None;
        ui
    });
    let mut s = shell.borrow_mut();
    s.parked = Some(parked);
    s.main = None;
    s.view = None;
    true
}

fn tray_running(shell: &ShellRef) -> bool {
    let backend = shell.borrow().backend.clone();
    let guard = backend.services.lock().expect("services lock");
    guard.as_ref().is_some_and(|s| s.tray_running())
}

/// Stops the services, flushes the core and exits.
fn quit(shell: &ShellRef, cx: &mut App) {
    let backend = {
        let mut s = shell.borrow_mut();
        if s.quitting {
            return;
        }
        s.quitting = true;
        s.backend.clone()
    };
    backend.player.stop();
    if let Some(services) = backend.services.lock().expect("services lock").take() {
        services.shutdown();
    }
    // Stats, flags and loved tracks, and a stopped nowplaying.json -- or the
    // overlay keeps showing a track that stopped with us.
    backend.with(|b| b.shutdown());
    cx.quit();
}

/// The tray preview: shown in the configured corner, hidden again after the
/// configured time. Toggles.
fn toggle_popup(shell: &ShellRef, cx: &mut App) {
    let open = shell.borrow_mut().popup.take();
    // A handle to a popup that was closed some other way fails here, and the
    // click opens a new one instead of doing nothing.
    if let Some(handle) = open {
        if handle.update(cx, |_, window, _| window.remove_window()).is_ok() {
            return;
        }
    }
    let (config, now, track, artwork_dir) = {
        let s = shell.borrow();
        let ui = current_ui(&s, cx);
        (
            ui.as_ref().map(|u| u.0.clone()).unwrap_or_default(),
            s.now.clone(),
            ui.and_then(|u| u.1),
            s.artwork_dir.clone(),
        )
    };
    let build = || {
        let mut p = Popup::new(artwork_dir.clone());
        p.now = now.clone();
        p.track_id = track.clone();
        p
    };
    let debug = std::env::var_os("BARD_POPUP_DEBUG").is_some();
    let layer = std::env::var_os("BARD_POPUP_PLAIN").is_none();
    let opened = cx.open_window(popup_options(&config, layer), |_, cx| cx.new(|_| build())).or_else(|e| {
        if debug {
            eprintln!("[bard] preview: layer-shell refused ({e:#}), using a plain window");
        }
        // X11, or a compositor without layer-shell: a plain window, placed
        // wherever the window manager decides, as Electron's was on Wayland.
        cx.open_window(popup_options(&config, false), |_, cx| cx.new(|_| build()))
    });
    if debug {
        eprintln!("[bard] preview: opened={} has_snapshot={}", opened.is_ok(), now.is_some());
    }
    let Ok(handle) = opened else { return };
    let generation = {
        let mut s = shell.borrow_mut();
        s.popup = Some(handle);
        s.popup_generation += 1;
        s.popup_generation
    };
    let seconds = config.window.tray_popup_seconds;
    if seconds > 0 {
        let shell = shell.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(seconds as u64)).await;
            let _ = cx.update(|cx| {
                let mut s = shell.borrow_mut();
                if s.popup_generation == generation {
                    if let Some(h) = s.popup.take() {
                        drop(s);
                        let _ = h.update(cx, |_, window, _| window.remove_window());
                    }
                }
            });
        })
        .detach();
    }
}

fn popup_options(config: &Config, layer: bool) -> WindowOptions {
    use bard_core::types::TrayPopupCorner as C;
    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
    let size = size(px(popup::WIDTH), px(popup::HEIGHT));
    let kind = if layer {
        // The compositor places a layer surface by anchor and margin, so on
        // Wayland the corner setting finally works. "Near the cursor" has no
        // equivalent there and uses the default corner.
        let anchor = match config.window.tray_popup_corner {
            C::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
            C::TopRight => Anchor::TOP | Anchor::RIGHT,
            C::TopLeft => Anchor::TOP | Anchor::LEFT,
            C::BottomRight | C::Cursor => Anchor::BOTTOM | Anchor::RIGHT,
        };
        let m = px(16.);
        WindowKind::LayerShell(LayerShellOptions {
            namespace: "bard-popup".into(),
            layer: Layer::Overlay,
            anchor,
            exclusive_zone: None,
            exclusive_edge: None,
            margin: Some((m, m, m, m)),
            keyboard_interactivity: KeyboardInteractivity::None,
        })
    } else {
        WindowKind::PopUp
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(0.), px(0.)), size })),
        titlebar: None,
        focus: false,
        show: true,
        kind,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        app_id: Some("bard".into()),
        window_background: WindowBackgroundAppearance::Transparent,
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

/// The config and the playing track id, from wherever the model is.
fn current_ui(s: &Shell, cx: &App) -> Option<(Config, Option<String>)> {
    let pick = |u: &Ui| (u.config.clone(), u.track.as_ref().map(|t| t.id.clone()));
    match (s.view.as_ref().and_then(|v| v.upgrade()), &s.parked) {
        (Some(v), _) => Some(pick(&v.read(cx).ui)),
        (None, Some(u)) => Some(pick(u)),
        (None, None) => None,
    }
}

/// Routes one event: window-level ones here, model updates to `apply` on the
/// open window or the parked model.
fn dispatch(shell: &ShellRef, ev: AppEvent, cx: &mut App) {
    match ev {
        AppEvent::Control(Control::ShowWindow) => return show_main(shell, cx),
        AppEvent::Control(Control::ShowPopup) => return toggle_popup(shell, cx),
        AppEvent::Control(Control::Quit) => return quit(shell, cx),
        AppEvent::Core(CoreEvent::Overlay(ref np)) => {
            let popup = {
                let mut s = shell.borrow_mut();
                s.now = Some(np.clone());
                s.popup
            };
            if let Some(h) = popup {
                let np = np.clone();
                let track = {
                    let s = shell.borrow();
                    current_ui(&s, cx).and_then(|u| u.1)
                };
                let _ = h.update(cx, |p, _, cx| {
                    p.now = Some(np);
                    p.track_id = track;
                    cx.notify();
                });
            }
            return;
        }
        _ => {}
    }
    let (view, backend) = {
        let s = shell.borrow();
        (s.view.as_ref().and_then(|v| v.upgrade()), s.backend.clone())
    };
    if let Some(view) = view {
        view.update(cx, |w, cx| {
            apply(&mut w.ui, &backend, ev, cx);
            cx.notify();
        });
        return;
    }
    let mut parked = shell.borrow_mut().parked.take();
    if let Some(ui) = parked.as_mut() {
        apply(ui, &backend, ev, cx);
    }
    shell.borrow_mut().parked = parked;
}

fn apply(ui: &mut Ui, backend: &AppBackend, ev: AppEvent, cx: &mut App) {
    match ev {
        AppEvent::Control(c) => match c {
            Control::Play => {
                if !ui.playing {
                    backend.toggle_play();
                }
            }
            Control::Pause => backend.player.pause(),
            Control::Toggle => backend.toggle_play(),
            Control::Next => backend.skip(),
            Control::Previous => backend.previous(),
            Control::Seek(s) => backend.seek(s),
            Control::SetVolume(v) => {
                ui.volume = v.clamp(0., 1.);
                backend.set_volume(ui.volume);
            }
            // Handled by dispatch.
            Control::ShowWindow | Control::ShowPopup | Control::Quit => {}
        },
        AppEvent::SleepDue(at) => {
            // Pauses rather than quitting: waking to a closed app and a lost
            // queue is worse than waking to a paused one.
            if ui.sleep_ends_at == Some(at) {
                ui.sleep_ends_at = None;
                if ui.playing {
                    backend.player.pause();
                }
            }
        }
        AppEvent::ObsSaved { stored, chars } => {
            // A clear is not news; a save says so either way, since a
            // credential field that changes nothing visible on save is
            // indistinguishable from one that failed.
            ui.obs_message = if chars == 0 {
                None
            } else if stored {
                Some(format!("Password saved ({chars} characters) — reconnecting to OBS"))
            } else {
                Some("No OS keyring available — password not stored".into())
            };
        }
        AppEvent::ChooseFolder => {
            let picked = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Add music folder".into()),
            });
            let bard = backend.bard.clone();
            let events = backend.events.clone();
            cx.spawn(async move |_| {
                if let Ok(Ok(Some(paths))) = picked.await {
                    if let Some(dir) = paths.into_iter().next() {
                        let dir = dir.to_string_lossy().into_owned();
                        let mut b = bard.lock().expect("core lock");
                        let mut added = false;
                        b.update_config(|c| {
                            if !c.library_paths.contains(&dir) {
                                c.library_paths.push(dir);
                                added = true;
                            }
                        });
                        drop(b);
                        if added {
                            let _ = events.unbounded_send(AppEvent::Rescan);
                        }
                    }
                }
            })
            .detach();
        }
        AppEvent::Rescan => {
            ui.scanning = true;
            backend.scan();
        }
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
}
