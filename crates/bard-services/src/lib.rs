//! Bard's services: everything that talks to the OS or other programs rather
//! than to the user.
//!
//! | Electron | here |
//! |---|---|
//! | `overlay.ts` (express + SSE) | [`overlay`] (axum), 127.0.0.1 only |
//! | `obs.ts`, `streamer.ts` | [`obs`]; arming is [`Bard::arm`] |
//! | `secrets.ts` (safeStorage) | [`secrets`] (Secret Service), plus a one-time migration of `obs-password.bin` |
//! | Chromium's implicit MPRIS | [`mpris`], `org.mpris.MediaPlayer2.bard` |
//! | `tray.ts` | [`tray`] (StatusNotifierItem) |
//! | `shortcuts.ts` | [`media_keys`]: nothing is grabbed; keys arrive over MPRIS |
//!
//! # Wiring it into the app
//!
//! ```ignore
//! let (bard, events) = Bard::open(default_data_dir())?;
//! let bard = Arc::new(Mutex::new(bard));
//! let services = Services::start(bard.clone(), Arc::new(MyControls { .. }), ServicesOptions::default());
//!
//! // The app owns the CoreEvent receiver. For every event, after (or before)
//! // handling it itself, forward it:
//! for event in events {               // or however the app drains it
//!     services.handle(&event);
//!     app_handle(event);
//! }
//!
//! // Quitting:
//! services.shutdown();
//! bard.lock().unwrap().shutdown();
//! ```
//!
//! - [`Services::start`] locks the Bard mutex briefly, so **don't hold the lock
//!   while calling it**. It binds the overlay port and calls
//!   [`Bard::set_overlay_port`] itself (the overlay's art URLs need the bound
//!   port, not the configured one); [`Services::overlay_url`] has the result
//!   for Settings and the tray popup.
//! - [`Services::handle`] never locks Bard and never blocks: it copies what it
//!   needs and hands the rest to the services' own tokio runtime, so it is
//!   safe to call from the GPUI thread, with or without the Bard lock held.
//!   It needs `Overlay`, `NowPlaying` and `ConfigChanged`; other events are
//!   ignored, but forwarding everything is fine.
//! - [`AppControls`] is how MPRIS and the tray reach the app. Its methods are
//!   called on service threads (D-Bus handlers), so the implementation should
//!   post to the GPUI thread rather than touch views directly.
//! - Settings' OBS password field calls [`Services::set_obs_password`] (it
//!   updates `config.streamer.hasObsPassword` and restarts the OBS watcher);
//!   whether to offer the field at all is [`Services::secure_storage_available`].
//!
//! # Threads
//!
//! Services owns a small multi-threaded tokio runtime for the overlay server,
//! the OBS websocket, MPRIS and the tray. The keyring is the exception: its
//! blocking D-Bus calls panic inside a tokio runtime, so they run on plain
//! threads (see [`secrets`]).
//!
//! [`Bard::arm`]: bard_core::Bard::arm
//! [`Bard::set_overlay_port`]: bard_core::Bard::set_overlay_port

pub mod media_keys;
pub mod mpris;
pub mod obs;
pub mod overlay;
pub mod secrets;
pub mod tray;

use bard_core::types::StreamerSource;
use bard_core::{Bard, CoreEvent};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::runtime::{Handle, Runtime};

/// What MPRIS and the tray ask the app to do. Called from service threads;
/// post to the UI thread as needed. Each should return promptly.
///
/// `next` / `previous` can simply be `bard.lock().skip()` / `.previous()` --
/// the core then sends `PlayTrack` as usual. The rest drive the audio engine.
pub trait AppControls: Send + Sync + 'static {
    fn play(&self);
    fn pause(&self);
    /// Play/pause (the tray's Play/Pause item, MPRIS PlayPause, media keys).
    fn toggle(&self);
    fn next(&self);
    fn previous(&self);
    /// Seek to an absolute position in the current track.
    fn seek(&self, position_sec: f64);
    /// 0.0..=1.0. Apply to the engine and save `config.audio.volume`; the
    /// resulting `ConfigChanged` updates MPRIS.
    fn set_volume(&self, volume: f64);
    /// Show and raise the main window (tray click, "Show Bard", MPRIS Raise).
    fn show_window(&self);
    /// Toggle the tray's now-playing preview window (middle click, menu item).
    fn show_popup(&self);
    /// Quit the app (tray menu, MPRIS Quit).
    fn quit(&self);
}

/// Which services to run. All on by default.
#[derive(Debug, Clone)]
pub struct ServicesOptions {
    pub overlay: bool,
    pub obs: bool,
    pub mpris: bool,
    pub tray: bool,
    /// `org.mpris.MediaPlayer2.<this>`; tests use a unique one.
    pub mpris_bus_suffix: String,
    /// The keyring slot for the OBS password; tests use their own.
    pub obs_password: secrets::ObsPassword,
    /// Move the Electron build's obs-password.bin into the keyring once.
    pub migrate_obs_password: bool,
}

impl Default for ServicesOptions {
    fn default() -> Self {
        ServicesOptions {
            overlay: true,
            obs: true,
            mpris: true,
            tray: true,
            mpris_bus_suffix: mpris::BUS_SUFFIX.into(),
            obs_password: secrets::ObsPassword::default(),
            migrate_obs_password: true,
        }
    }
}

struct Inner {
    bard: Arc<Mutex<Bard>>,
    rt: Handle,
    data_dir: PathBuf,
    artwork_dir: PathBuf,
    overlay: Mutex<Option<overlay::OverlayServer>>,
    mpris: Option<mpris::Mpris>,
    tray: Option<tray::Tray>,
    obs_enabled: bool,
    /// Set once the keyring thread has run; until then the watcher waits.
    obs_ready: std::sync::atomic::AtomicBool,
    obs: Mutex<obs::ObsSync>,
    obs_bridge: Arc<ObsBridge>,
    password: secrets::ObsPassword,
}

/// The running services. Dropping it shuts them down.
pub struct Services {
    inner: Arc<Inner>,
    rt: Option<Runtime>,
}

/// The OBS watcher's link back to the core.
struct ObsBridge {
    bard: Arc<Mutex<Bard>>,
    password: secrets::ObsPassword,
}

impl obs::ObsHandler for ObsBridge {
    fn live(&self, source: StreamerSource) {
        let bard = self.bard.clone();
        // Off the async workers: the UI may be holding the lock.
        tokio::task::spawn_blocking(move || bard.lock().unwrap().arm(Some(source)));
    }
    fn status(&self, message: String) {
        let bard = self.bard.clone();
        tokio::task::spawn_blocking(move || bard.lock().unwrap().notice(message));
    }
    fn password(&self) -> impl Future<Output = String> + Send {
        secrets::get_on_thread(self.password.clone())
    }
}

impl Services {
    /// Starts everything enabled in `opts`. Failures are logged and leave
    /// that service off; the app runs regardless. Don't hold the Bard lock
    /// while calling this.
    pub fn start(bard: Arc<Mutex<Bard>>, controls: Arc<dyn AppControls>, opts: ServicesOptions) -> Services {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("bard-services")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let handle = rt.handle().clone();

        let (config, data_dir, artwork_dir, snapshot, item) = {
            let b = bard.lock().unwrap();
            (
                b.config().clone(),
                b.data_dir().to_path_buf(),
                PathBuf::from(b.paths().artwork),
                b.now_playing_snapshot(),
                b.now_playing().cloned(),
            )
        };

        // Overlay.
        let overlay = if opts.overlay {
            match overlay::OverlayServer::start(&handle, config.overlay.port, artwork_dir.clone(), snapshot.clone()) {
                Some(server) => {
                    let mut b = bard.lock().unwrap();
                    // Snapshots build their art URL from this, so it has to
                    // be the port we got, not the one we asked for.
                    b.set_overlay_port(Some(server.port()));
                    server.push(&b.now_playing_snapshot());
                    eprintln!("[bard] overlay: {}", server.url());
                    Some(server)
                }
                None => {
                    eprintln!("[bard] overlay: FAILED to bind a port");
                    None
                }
            }
        } else {
            None
        };

        // MPRIS.
        let mpris = if opts.mpris {
            let initial = mpris::PlayerState {
                track: item.as_ref().map(|i| mpris::MprisTrack::from_item(i, &artwork_dir)),
                playing: snapshot.is_playing,
                duration_sec: snapshot.duration_sec,
                position_sec: snapshot.position_sec,
                queue_length: snapshot.queue_length,
                volume: config.audio.volume,
                ..Default::default()
            };
            match mpris::start_blocking(&handle, &opts.mpris_bus_suffix, initial, controls.clone()) {
                Ok(m) => Some(m),
                Err(e) => {
                    eprintln!("[bard] mpris: {e}");
                    None
                }
            }
        } else {
            None
        };

        // Tray.
        let tray = if opts.tray {
            let initial = tray::TrayState {
                current: item.as_ref().map(|i| (i.track.title.clone(), i.track.artist.clone())),
                playing: snapshot.is_playing,
                queue_length: snapshot.queue_length,
            };
            let c = controls.clone();
            match handle.block_on(async move { tray::Tray::start(initial, c).await }) {
                Ok(t) => Some(t),
                Err(e) => {
                    eprintln!("[bard] tray: {e}");
                    None
                }
            }
        } else {
            None
        };

        let inner = Arc::new(Inner {
            obs_bridge: Arc::new(ObsBridge { bard: bard.clone(), password: opts.obs_password.clone() }),
            bard,
            rt: handle,
            data_dir,
            artwork_dir,
            overlay: Mutex::new(overlay),
            mpris,
            tray,
            obs_enabled: opts.obs,
            obs_ready: false.into(),
            obs: Mutex::new(obs::ObsSync::default()),
            password: opts.obs_password,
        });

        // Keyring work on a plain thread (see `secrets`): migrate the Electron
        // password, square the config flag with what the keyring holds, and
        // only then start the OBS watcher, so it does not ask OBS for a
        // password that is a moment away from being migrated.
        let migrate = opts.migrate_obs_password;
        let i = inner.clone();
        std::thread::Builder::new()
            .name("bard-keyring".into())
            .spawn(move || {
                if migrate {
                    match secrets::migrate_electron(&i.data_dir, &i.password) {
                        secrets::Migration::NothingToDo => {}
                        outcome => eprintln!("[bard] obs password migration: {outcome:?}"),
                    }
                }
                if let Some(stored) = i.password.stored() {
                    let mut b = i.bard.lock().unwrap();
                    if b.config().streamer.has_obs_password != stored {
                        b.update_config(|c| c.streamer.has_obs_password = stored);
                    }
                }
                let s = i.bard.lock().unwrap().config().streamer.clone();
                i.obs_ready.store(true, std::sync::atomic::Ordering::SeqCst);
                i.sync_obs(s.auto_arm, &s.obs_url, false);
            })
            .expect("spawn keyring thread");

        Services { inner, rt: Some(rt) }
    }

    /// Feeds a core event to the services. Never blocks and never locks Bard;
    /// see the crate docs.
    pub fn handle(&self, event: &CoreEvent) {
        let i = &self.inner;
        match event {
            CoreEvent::Overlay(np) => {
                if let Some(o) = i.overlay.lock().unwrap().as_ref() {
                    o.push(np);
                }
                if let Some(m) = &i.mpris {
                    m.update(mpris::Update::Progress(np.clone()));
                }
                if let Some(t) = &i.tray {
                    t.update(tray::TrayState { playing: np.is_playing, queue_length: np.queue_length, ..t.state() });
                }
            }
            CoreEvent::NowPlaying(item) => {
                if let Some(m) = &i.mpris {
                    let track = item.as_ref().map(|it| mpris::MprisTrack::from_item(it, &i.artwork_dir));
                    m.update(mpris::Update::Track(track));
                }
                if let Some(t) = &i.tray {
                    let current = item.as_ref().map(|it| (it.track.title.clone(), it.track.artist.clone()));
                    let mut next = tray::TrayState { current, ..t.state() };
                    if item.is_none() {
                        next.playing = false;
                    }
                    t.update(next);
                }
            }
            CoreEvent::ConfigChanged(c) => {
                if let Some(m) = &i.mpris {
                    m.update(mpris::Update::Volume(c.audio.volume));
                }
                // A changed endpoint or auto-arm toggle takes effect now, not
                // on the next restart.
                i.sync_obs(c.streamer.auto_arm, &c.streamer.obs_url, false);
            }
            _ => {}
        }
    }

    /// The port the overlay actually bound.
    pub fn overlay_port(&self) -> Option<u16> {
        self.inner.overlay.lock().unwrap().as_ref().map(|o| o.port())
    }

    /// `getOverlayUrl`: `http://127.0.0.1:<port>`, or None when not running.
    pub fn overlay_url(&self) -> Option<String> {
        self.inner.overlay.lock().unwrap().as_ref().map(|o| o.url())
    }

    /// Connected overlay (SSE) clients.
    pub fn overlay_client_count(&self) -> usize {
        self.inner.overlay.lock().unwrap().as_ref().map_or(0, |o| o.client_count())
    }

    /// True when the tray icon (StatusNotifierItem) started and is still up.
    /// With no tray, closing the window must quit rather than hide to it, or
    /// Bard would keep running with no way back to it.
    pub fn tray_running(&self) -> bool {
        self.inner.tray.as_ref().is_some_and(|t| t.is_running())
    }

    /// The MPRIS bus name owned, if MPRIS is running.
    pub fn mpris_bus_name(&self) -> Option<&str> {
        self.inner.mpris.as_ref().map(|m| m.bus_name())
    }

    /// `secureStorageAvailable`. Blocking D-Bus call on first use; call it off
    /// the UI thread if that matters, and never on a tokio runtime thread.
    pub fn secure_storage_available() -> bool {
        secrets::available()
    }

    /// `setObsPassword`: stores (or clears, given "") the password on a
    /// background thread, keeps `config.streamer.hasObsPassword` in step,
    /// restarts the OBS watcher so the new password takes effect, then calls
    /// `done` with whether it was stored (false: no keyring to store into).
    pub fn set_obs_password(&self, password: String, done: impl FnOnce(bool) + Send + 'static) {
        let i = self.inner.clone();
        std::thread::spawn(move || {
            let ok = i.password.set(&password);
            drop(password);
            // The user has spoken; the old Electron file must not overrule them.
            secrets::mark_migrated(&i.data_dir);
            let has = i.password.stored().unwrap_or(false);
            let s = {
                let mut b = i.bard.lock().unwrap();
                // Keep the config flag in step so Settings can offer "replace"
                // rather than an empty field implying no password is set.
                b.update_config(|c| c.streamer.has_obs_password = has).streamer
            };
            i.sync_obs(s.auto_arm, &s.obs_url, true);
            done(ok);
        });
    }

    /// Stops everything: the OBS watcher, the overlay (freeing its port), the
    /// tray and MPRIS. Call before [`Bard::shutdown`](bard_core::Bard::shutdown).
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let Some(rt) = self.rt.take() else { return };
        self.inner.obs.lock().unwrap().stop();
        if self.inner.overlay.lock().unwrap().take().is_some()
            && let Ok(mut b) = self.inner.bard.try_lock()
        {
            b.set_overlay_port(None);
        }
        if let Some(t) = &self.inner.tray {
            let awaiter = t.shutdown();
            let _ = rt.block_on(async { tokio::time::timeout(std::time::Duration::from_secs(1), awaiter).await });
        }
        rt.shutdown_timeout(std::time::Duration::from_secs(1));
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Inner {
    fn sync_obs(&self, auto_arm: bool, url: &str, force: bool) {
        if !self.obs_enabled || !self.obs_ready.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.obs.lock().unwrap().sync(&self.rt, auto_arm, url, force, &self.obs_bridge);
    }
}
