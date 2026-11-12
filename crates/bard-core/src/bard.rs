//! The facade: what `BardApi` in `src/shared/ipc.ts` offered the renderer,
//! plus the orchestration `ipc-handlers.ts` and `index.ts` did around it.

use crate::config::ConfigStore;
use crate::deck::Deck;
use crate::library::{self, Library};
use crate::nowplaying::NowPlayingState;
use crate::paths::{is_revealable, resolve};
use crate::search::{self, Matcher};
use crate::stores::{self, Albums, Loved, Stats};
use crate::types::*;
use crate::util::now_ms;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// Everything the core tells the rest of the app.
///
/// One channel replaces the Electron build's `webContents.send` pushes and
/// the nowplaying listener list. Events are sent in the order the TypeScript
/// sent them.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    QueueChanged(Vec<QueueItem>),
    /// Start playing this track from the beginning (also used to restart).
    PlayTrack(Track),
    ScanProgress(ScanProgress),
    /// The queue item now playing, or None when playback stopped.
    NowPlaying(Option<QueueItem>),
    /// Whenever the overlay would push an SSE frame: track or play state
    /// change, every progress report, queue length change, overlay port change.
    Overlay(NowPlaying),
    /// Re-read with [`Bard::stats`].
    StatsChanged,
    /// Re-read with [`Bard::album_flags`].
    AlbumFlagsChanged,
    /// Re-read with [`Bard::loved`].
    LovedChanged,
    /// Transport commands from outside the UI (tray, media keys, MPRIS), relayed
    /// through [`Bard::transport`].
    Transport(TransportAction),
    /// Something the user must be told -- a refusal and its reason, chiefly.
    /// Streamer mode blocks tracks, and a block the user cannot see is
    /// indistinguishable from a broken player.
    Notice(String),
    /// The config changed. Unlike the Electron build, sent for every change,
    /// not only those main made on its own, so services (media keys, the OBS
    /// watcher) can react to settings without a separate path.
    ConfigChanged(Config),
    /// What armed streamer mode; None when armed by hand or disarmed.
    StreamerSource(Option<StreamerSource>),
}

/// Debounce intervals for the stores; tests shorten them.
#[derive(Debug, Clone)]
pub struct Options {
    pub stats_debounce: Duration,
    pub albums_debounce: Duration,
    pub loved_debounce: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            stats_debounce: stores::STATS_DEBOUNCE,
            albums_debounce: stores::ALBUMS_DEBOUNCE,
            loved_debounce: stores::LOVED_DEBOUNCE,
        }
    }
}

/// The default data directory: `~/.config/bard`.
///
/// Also performs the Electron build's one-time adoption of `~/.config/Bard`:
/// Electron derived its data directory from productName, so renaming the
/// product from "bard" to "Bard" silently relocated every setting and the
/// artwork cache. When only the capitalised directory exists it is renamed
/// into place.
pub fn default_data_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let target = base.join("bard");
    let stray = base.join("Bard");
    if !target.exists() && stray.exists() {
        // Falling through is safe: a fresh directory is created on open.
        let _ = std::fs::rename(&stray, &target);
    }
    target
}

/// Bard's core state. `Send`, so the app can hold it as `Arc<Mutex<Bard>>`.
pub struct Bard {
    pub(crate) data_dir: PathBuf,
    pub(crate) events: Sender<CoreEvent>,
    pub(crate) config: ConfigStore,
    pub(crate) library: Library,
    pub(crate) stats: Stats,
    pub(crate) albums: Albums,
    pub(crate) loved: Loved,
    pub(crate) np: NowPlayingState,
    pub(crate) rng: fastrand::Rng,

    // queue.ts state
    pub(crate) queue: Vec<QueueItem>,
    pub(crate) now_playing: Option<QueueItem>,
    /// Whether the current track has already been counted, and how far it got.
    pub(crate) counted: bool,
    pub(crate) position: f64,
    /// Recently played ids, newest last. Keeps shuffle from repeating itself.
    pub(crate) history: Vec<String>,
    /// Tracks actually played, newest last, for stepping backwards.
    ///
    /// Separate from `history`, which exists to stop shuffle repeating itself
    /// and is capped by a different setting. Conflating them would make the
    /// back button depend on a shuffle preference.
    pub(crate) played: Vec<QueueItem>,
    /// Shuffle deck: dealt through completely before any track repeats.
    pub(crate) deck: Option<Deck>,

    // streamer.ts state
    /// What armed the mode, for the banner to name.
    ///
    /// Runtime state rather than config: it describes what OBS is doing now,
    /// and persisting it would have the banner claim a recording that stopped
    /// hours ago.
    pub(crate) streamer_source: Option<StreamerSource>,
}

impl Bard {
    /// Opens (creating if needed) a data directory and loads every store.
    ///
    /// The library starts empty; call [`Bard::scan_library`] (or run a
    /// [`LibraryScanner`] and [`Bard::install_library`]) to fill it.
    pub fn open(data_dir: PathBuf) -> anyhow::Result<(Bard, Receiver<CoreEvent>)> {
        Self::open_with(data_dir, Options::default())
    }

    pub fn open_with(data_dir: PathBuf, opts: Options) -> anyhow::Result<(Bard, Receiver<CoreEvent>)> {
        std::fs::create_dir_all(&data_dir)?;
        let (tx, rx) = channel();
        let bard = Bard {
            config: ConfigStore::open(data_dir.join("config.json"))?,
            library: Library::default(),
            stats: Stats::load(data_dir.join("stats.json"), opts.stats_debounce),
            albums: Albums::load(data_dir.join("albums.json"), opts.albums_debounce),
            loved: Loved::load(data_dir.join("loved.json"), opts.loved_debounce),
            np: NowPlayingState::new(data_dir.join("nowplaying.json"), tx.clone()),
            rng: fastrand::Rng::new(),
            events: tx,
            data_dir,
            queue: Vec::new(),
            now_playing: None,
            counted: false,
            position: 0.0,
            history: Vec::new(),
            played: Vec::new(),
            deck: None,
            streamer_source: None,
        };
        Ok((bard, rx))
    }

    /// Makes shuffling deterministic, for tests.
    #[doc(hidden)]
    pub fn seed_rng(&mut self, seed: u64) {
        self.rng = fastrand::Rng::with_seed(seed);
    }

    pub(crate) fn emit(&self, e: CoreEvent) {
        let _ = self.events.send(e);
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub(crate) fn artwork_dir(&self) -> PathBuf {
        self.data_dir.join("artwork")
    }

    /// Where config, the artwork cache and nowplaying.json live on disk.
    pub fn paths(&self) -> AppPaths {
        let s = |p: &Path| p.to_string_lossy().into_owned();
        AppPaths {
            config: s(self.config.path()),
            config_dir: s(&self.data_dir),
            artwork: s(&self.artwork_dir()),
            now_playing: s(self.np.path()),
            // Electron's app.getPath('logs') on Linux.
            logs: s(&self.data_dir.join("logs")),
        }
    }

    /// Whether `target` may be opened in the file manager: the data directory,
    /// a library folder, or something beneath one.
    ///
    /// Opening a path hands it to the desktop's default handler -- a .desktop
    /// file under a directory admitted by mistake is executed -- so the UI must
    /// check this before revealing anything.
    pub fn can_reveal(&self, target: impl AsRef<Path>) -> bool {
        let mut roots = vec![self.data_dir.to_string_lossy().into_owned()];
        roots.extend(self.config.get().library_paths.iter().cloned());
        is_revealable(&roots, target)
    }

    // --- library --------------------------------------------------------------

    pub fn tracks(&self) -> &[Track] {
        self.library.tracks()
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.library.get(id)
    }

    /// The library pane's filter: tracks whose title, artist or album contain
    /// `query`, case-insensitively. Blank matches everything.
    pub fn search(&self, query: &str) -> Vec<&Track> {
        search::filter(self.library.tracks(), query)
    }

    /// The single best fuzzy match for `query` (match.ts's `createMatcher`),
    /// or None rather than a weak guess.
    pub fn best_match(&self, query: &str) -> Option<&Track> {
        Matcher::new(self.library.tracks()).best(query)
    }

    /// Everything needed to scan the library without holding `&mut Bard`.
    ///
    /// A scan of a few hundred files takes seconds; with the app holding Bard
    /// behind a mutex, `scan_library` would lock everyone else out for that
    /// long. Run the scanner on a worker thread and hand its result to
    /// [`Bard::install_library`].
    pub fn scanner(&self) -> LibraryScanner {
        LibraryScanner {
            roots: self.config.get().library_paths.clone(),
            artwork_dir: self.artwork_dir(),
            events: self.events.clone(),
        }
    }

    /// Scans the configured folders and installs the result. Blocking; sends
    /// [`CoreEvent::ScanProgress`] as it goes.
    pub fn scan_library(&mut self) -> Vec<Track> {
        let tracks = self.scanner().run();
        self.install_library(tracks.clone());
        tracks
    }

    /// Replaces the library with a finished scan.
    pub fn install_library(&mut self, tracks: Vec<Track>) {
        self.library = Library::new(tracks);
        // Stock the queue now the library is known.
        //
        // Endless only ever reacted to something happening -- a track ending,
        // an item removed, the setting being changed. None of those occur at
        // startup, so the app opened with endless on and an empty queue, and
        // the only way to fill it was to toggle the setting off and on again.
        // The scan is the first moment there is anything to draw from.
        //
        // This deliberately fills the queue without starting playback: seeing
        // what is queued and choosing when it goes out is the whole point on a
        // stream. A no-op when endless is off or the queue is already full, so
        // rescans cost nothing.
        self.refresh_endless();
    }

    /// Adds a music folder (the UI shows the picker). Returns every library
    /// path. Already-listed folders are not added twice.
    pub fn add_library_path(&mut self, path: impl AsRef<Path>) -> Vec<String> {
        self.add_library_paths([path])
    }

    pub fn add_library_paths<P: AsRef<Path>>(&mut self, paths: impl IntoIterator<Item = P>) -> Vec<String> {
        let mut merged = self.config.get().library_paths.clone();
        for p in paths {
            let p = resolve(p).to_string_lossy().into_owned();
            if !merged.contains(&p) {
                merged.push(p);
            }
        }
        self.update_config(|c| c.library_paths = merged).library_paths
    }

    /// The cached cover for a track, or None when it has none.
    ///
    /// Replaces `getArtwork`, which returned a data URL: the UI loads the file
    /// itself, and nothing base64-encodes a JPEG to cross a process boundary.
    pub fn artwork_path(&self, track_id: &str) -> Option<PathBuf> {
        crate::artwork::artwork_path(&self.artwork_dir(), track_id)
    }

    // --- config ---------------------------------------------------------------

    pub fn config(&self) -> &Config {
        self.config.get()
    }

    /// Edits the config, writes it, and applies what `setConfig` did around it:
    ///
    /// - endless or its lookahead changed: top the queue up now;
    /// - streamer mode toggled: clear what armed it (turning it off by hand
    ///   clears the OBS source; turning it on by hand means there is none to
    ///   name) and apply the mode to the queue and the track on air.
    ///
    /// Media keys and the OBS watcher are other crates' business; they react
    /// to the [`CoreEvent::ConfigChanged`] this sends.
    ///
    /// Side effects key off what actually changed. The TypeScript keyed off
    /// which fields the patch named, so re-sending an unchanged value re-ran
    /// them; [`Bard::arm`] / [`Bard::disarm`] keep that behaviour for the
    /// streamer toggle, where it mattered.
    pub fn update_config(&mut self, f: impl FnOnce(&mut Config)) -> Config {
        let before = self.config.get().clone();
        let mut next = before.clone();
        f(&mut next);
        if !self.write_config(next) {
            return before;
        }
        let after = self.config.get().clone();
        if before.playback.endless != after.playback.endless
            || before.playback.endless_lookahead != after.playback.endless_lookahead
        {
            self.refresh_endless();
        }
        if before.streamer.enabled != after.streamer.enabled {
            self.set_streamer_source(None);
            self.refresh_streamer_mode();
        }
        self.config.get().clone()
    }

    /// Stores a config and announces it. Returns whether it changed.
    pub(crate) fn write_config(&mut self, next: Config) -> bool {
        match self.config.set(next) {
            Ok(true) => {
                self.emit(CoreEvent::ConfigChanged(self.config.get().clone()));
                true
            }
            Ok(false) => false,
            Err(e) => {
                self.emit(CoreEvent::Notice(format!("Could not save settings: {e}")));
                false
            }
        }
    }

    // --- playback reporting -------------------------------------------------

    /// Progress from the audio engine.
    ///
    /// Reports belonging to a track already moved on from are dropped. During
    /// a crossfade the engine keeps playing the outgoing track for several more
    /// seconds; crediting those to the incoming one counted a full play for a
    /// track nobody heard, and left `position` stale enough that Previous read
    /// as "restart" and replayed the track that had just started.
    pub fn report_progress(&mut self, p: PlaybackProgress) {
        if !self.is_current_track(p.track_id.as_deref()) {
            return;
        }
        let write = self.config.get().overlay.write_now_playing_json;
        let (position, duration, playing) = (p.position, p.duration, p.playing);
        self.np.set_progress(p, write);
        if playing {
            self.report_position(position, duration);
        }
    }

    /// The current track finished.
    pub fn report_ended(&mut self) {
        self.on_track_ended();
    }

    /// Relays a transport command (tray, media keys, MPRIS) to whoever drives
    /// the audio engine, as `EVENT.transport` did.
    pub fn transport(&self, action: TransportAction) {
        self.emit(CoreEvent::Transport(action));
    }

    /// Shows the user a message through the UI's notice area.
    pub fn notice(&self, message: impl Into<String>) {
        self.emit(CoreEvent::Notice(message.into()));
    }

    // --- overlay ----------------------------------------------------------------

    /// The port the overlay server actually bound, or None when it is not
    /// running. Cover art URLs are built from this, not the configured port.
    pub fn set_overlay_port(&mut self, port: Option<u16>) {
        self.np.set_overlay_port(port);
    }

    /// What `/nowplaying.json` and a new SSE client's first frame carry.
    pub fn now_playing_snapshot(&self) -> NowPlaying {
        self.np.snapshot()
    }

    // --- stats, album flags, loved ------------------------------------------

    /// Play counts keyed by track id.
    pub fn stats(&self) -> StatsMap {
        self.stats.all()
    }

    pub fn track_stats(&self, track_id: &str) -> TrackStats {
        self.stats.get(track_id)
    }

    pub fn clear_stats(&mut self) {
        self.stats.clear();
        self.emit(CoreEvent::StatsChanged);
    }

    /// Content ID results, keyed by album name.
    pub fn album_flags(&self) -> AlbumFlags {
        self.albums.all()
    }

    pub fn album_status(&self, album: &str) -> ContentIdStatus {
        self.albums.status(album)
    }

    pub fn set_album_flag(&mut self, album: &str, status: ContentIdStatus, note: Option<&str>) -> AlbumFlags {
        self.albums.set(album, status, note, now_ms());
        self.emit(CoreEvent::AlbumFlagsChanged);
        // A newly claimed album must leave the pending queue, not just future
        // picks -- otherwise it still plays in a few minutes' time.
        self.drop_claimed_from_queue();
        self.albums.all()
    }

    /// Track ids the user has loved, in the order they were loved.
    pub fn loved(&self) -> Vec<String> {
        self.loved.all()
    }

    pub fn is_loved(&self, track_id: &str) -> bool {
        self.loved.contains(track_id)
    }

    pub fn toggle_loved(&mut self, track_id: &str) -> Vec<String> {
        self.loved.toggle(track_id);
        self.emit(CoreEvent::LovedChanged);
        self.loved.all()
    }

    // --- lifecycle --------------------------------------------------------------

    /// Writes any pending stats, album flags and loved tracks now.
    pub fn flush(&self) {
        self.stats.flush();
        self.albums.flush();
        self.loved.flush();
    }

    /// What `before-quit` did: flush the stores, and leave nowplaying.json in
    /// the stopped state so nothing reading it keeps showing a track that
    /// stopped with the app.
    pub fn shutdown(&mut self) {
        self.flush();
        let write = self.config.get().overlay.write_now_playing_json;
        self.np.write_stopped(write);
    }
}

/// A library scan detached from [`Bard`], to run on any thread.
pub struct LibraryScanner {
    pub roots: Vec<String>,
    pub artwork_dir: PathBuf,
    events: Sender<CoreEvent>,
}

impl LibraryScanner {
    /// Scans, sending [`CoreEvent::ScanProgress`] at most every 100ms (and
    /// always for the last file) -- one message per file would flood the UI
    /// on a large library and make the progress bar the slow part.
    pub fn run(&self) -> Vec<Track> {
        let last: Mutex<Option<Instant>> = Mutex::new(None);
        let events = &self.events;
        library::scan(&self.roots, &self.artwork_dir, &|p: ScanProgress| {
            let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            if p.scanned != p.total && last.is_some_and(|l| now.duration_since(l) < Duration::from_millis(100)) {
                return;
            }
            *last = Some(now);
            let _ = events.send(CoreEvent::ScanProgress(p));
        })
    }
}
