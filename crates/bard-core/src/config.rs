//! config.json. A port of `src/main/config.ts` and the parts of electron-store
//! it relied on.
//!
//! Same file, same shape: tab-indented, keys in DEFAULT_CONFIG order, no
//! trailing newline, written atomically. Reading fills in anything an older
//! version did not write (every field has a default), and anything this
//! version no longer has is dropped.

use crate::types::Config;
use crate::util::{Indent, to_js_json, write_atomic};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// How long a continuous control (a slider drag) settles before its value
/// is written: one write per pause, not one per pixel.
pub(crate) const SETTLE: Duration = Duration::from_millis(250);
const MAX_RETRY: Duration = Duration::from_secs(30);

type WriteFn = dyn Fn(&Path, &Config) -> anyhow::Result<()> + Send + Sync;

/// Every write to config.json goes through here, in order.
///
/// Two kinds of change reach the file. Discrete ones (a folder, streamer
/// mode, a device) are transactional: written first, applied only once on
/// disk ([`ConfigStore::set`]). Continuous ones -- the volume and crossfade
/// sliders -- apply at once and are written behind, on this thread
/// ([`ConfigStore::set_deferred`]): writing each slider step on the UI thread,
/// under the core lock, put a create/write/rename in the path of every frame
/// of a drag.
///
/// Each change takes a version; a write records the newest version it
/// carried, and an older one is never written after it. Two locks: `state`,
/// held only to hand a value over, so a slider step never waits on the disk;
/// and `disk`, held across a write, so writes happen one at a time and in
/// order. `written` changes only under both.
struct Writer {
    state: Mutex<WriterState>,
    disk: Mutex<()>,
    wake: Condvar,
    path: PathBuf,
    write: Box<WriteFn>,
    notify: Mutex<Option<crate::stores::Notify>>,
}

#[derive(Default)]
struct WriterState {
    /// The newest deferred change not yet on disk.
    pending: Option<(u64, Config)>,
    /// The newest version on disk.
    written: u64,
    failing: bool,
    quit: bool,
    running: bool,
}

impl Writer {
    fn lock(&self) -> MutexGuard<'_, WriterState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tell(&self, msg: String) {
        if let Some(n) = self.notify.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            n(msg);
        }
    }

    fn disk(&self) -> MutexGuard<'_, ()> {
        self.disk.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Writes what is pending, if it is newer than what is on disk. The
    /// state lock is not held during the write: a change arriving meanwhile
    /// is taken at once and written next. Returns false if the write failed.
    fn write_pending(&self) -> bool {
        let _disk = self.disk();
        let (version, config) = {
            let mut st = self.lock();
            match st.pending.take() {
                None => return true,
                Some((v, _)) if v <= st.written => return true,
                Some(p) => p,
            }
        };
        let result = (self.write)(&self.path, &config);
        let ok = result.is_ok();
        let mut st = self.lock();
        let msg = match result {
            Ok(()) => {
                st.written = version;
                std::mem::take(&mut st.failing).then(|| "Settings are saving again".to_string())
            }
            Err(e) => {
                // Unless something newer arrived meanwhile, keep it to retry.
                if st.pending.is_none() {
                    st.pending = Some((version, config));
                }
                (!std::mem::replace(&mut st.failing, true))
                    .then(|| format!("Could not save settings: {e:#} \u{2014} will keep trying"))
            }
        };
        drop(st);
        if let Some(m) = msg {
            self.tell(m);
        }
        ok
    }

    fn run(self: Arc<Self>) {
        let mut wait = SETTLE;
        loop {
            {
                let mut st = self.lock();
                while st.pending.is_none() && !st.quit {
                    st = self.wake.wait(st).unwrap_or_else(|e| e.into_inner());
                }
                if st.quit {
                    // Drop flushes; nothing to do here.
                    st.running = false;
                    return;
                }
            }
            // Let a drag settle, without holding the lock.
            std::thread::sleep(wait);
            wait = if self.write_pending() { SETTLE } else { (wait * 2).min(MAX_RETRY) };
        }
    }
}

pub(crate) struct ConfigStore {
    path: PathBuf,
    config: Config,
    /// Version of `config`.
    version: u64,
    writer: Arc<Writer>,
}

impl ConfigStore {
    /// Loads config.json, rewriting it once if it needs it.
    ///
    /// Two cases rewrite at startup, as the Electron build did:
    /// - the file is missing, or lacks a top-level section (electron-store
    ///   wrote its defaults in at construction), and
    /// - the file carries settings this version no longer has. Without this a
    ///   removed setting sits in the file until some unrelated change happens
    ///   to trigger a write, which for the Twitch and request-rule blocks meant
    ///   months after the feature was deleted.
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let raw: Option<Value> = match std::fs::read_to_string(&path) {
            Ok(body) => Some(serde_json::from_str(&body).map_err(|e| {
                // electron-store refuses to start on a corrupt file as well;
                // overwriting it with defaults would lose every setting.
                anyhow::anyhow!("{} is not valid JSON: {e}", path.display())
            })?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let config = raw.as_ref().map(parse_lenient).unwrap_or_default();
        if raw.as_ref().is_none_or(needs_rewrite) {
            write(&path, &config)?;
        }
        Ok(Self::with_writer(path, config, Box::new(write)))
    }

    fn with_writer(path: PathBuf, config: Config, write: Box<WriteFn>) -> Self {
        let writer = Arc::new(Writer {
            state: Mutex::new(WriterState::default()),
            disk: Mutex::new(()),
            wake: Condvar::new(),
            path: path.clone(),
            write,
            notify: Mutex::new(None),
        });
        ConfigStore { path, config, version: 0, writer }
    }

    /// Where a deferred write's failure (and recovery) is told.
    pub fn set_notify(&self, notify: crate::stores::Notify) {
        *self.writer.notify.lock().unwrap_or_else(|e| e.into_inner()) = Some(notify);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> &Config {
        &self.config
    }

    /// Writes the config and, once it is on disk, replaces the one in
    /// memory. Returns whether anything changed. A failed write changes
    /// nothing, so the app never runs on settings that a restart would lose,
    /// and retrying the same change is still a change.
    pub fn set(&mut self, next: Config) -> anyhow::Result<bool> {
        if next == self.config {
            return Ok(false);
        }
        // Waits for a write in flight: this one must be on disk, and after
        // it, before the change applies.
        let _disk = self.writer.disk();
        (self.writer.write)(&self.path, &next)?;
        self.version += 1;
        // Carries every deferred change too (they are in `config` already),
        // so anything pending is now older than the file.
        let mut st = self.writer.lock();
        st.written = self.version;
        st.pending = None;
        drop(st);
        self.config = next;
        Ok(true)
    }

    /// Applies at once and writes behind, coalesced, on the writer thread.
    /// For continuous controls only: the value is in use before it is on
    /// disk. Returns whether anything changed.
    pub fn set_deferred(&mut self, next: Config) -> bool {
        if next == self.config {
            return false;
        }
        self.version += 1;
        self.config = next.clone();
        let mut st = self.writer.lock();
        st.pending = Some((self.version, next));
        if !st.running {
            st.running = true;
            let w = Arc::clone(&self.writer);
            let spawned = std::thread::Builder::new().name("bard-config-writer".into()).spawn(move || w.run());
            if spawned.is_err() {
                // No thread: write now rather than not at all.
                st.running = false;
                drop(st);
                self.writer.write_pending();
                return true;
            }
        }
        self.writer.wake.notify_one();
        true
    }

    /// Writes any deferred change now. Returns whether the file is current.
    pub fn flush(&self) -> bool {
        self.writer.write_pending()
    }
}

impl Drop for ConfigStore {
    /// The last slider position is not lost on quit.
    fn drop(&mut self) {
        self.flush();
        let mut st = self.writer.lock();
        st.quit = true;
        self.writer.wake.notify_one();
    }
}

fn write(path: &Path, config: &Config) -> anyhow::Result<()> {
    // Debug aid for checking the UI stays responsive on a slow disk.
    if let Some(ms) = std::env::var("BARD_SLOW_CONFIG_WRITE_MS").ok().and_then(|v| v.parse().ok()) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    write_atomic(path, &to_js_json(config, Indent::Tab)?)?;
    Ok(())
}

/// Reads a config value, section by section.
///
/// A section with a value of the wrong type falls back to its defaults rather
/// than failing the whole file -- electron-store would have carried the bad
/// value along, but the app could not have used it either.
fn parse_lenient(raw: &Value) -> Config {
    if let Ok(c) = serde_json::from_value::<Config>(raw.clone()) {
        return c;
    }
    let mut out = serde_json::to_value(Config::default()).expect("Config serialises");
    if let (Some(dst), Some(src)) = (out.as_object_mut(), raw.as_object()) {
        for (key, slot) in dst.iter_mut() {
            let Some(v) = src.get(key) else { continue };
            let mut probe = serde_json::to_value(Config::default()).expect("Config serialises");
            probe[key] = v.clone();
            if serde_json::from_value::<Config>(probe).is_ok() {
                *slot = v.clone();
            }
        }
    }
    serde_json::from_value(out).unwrap_or_default()
}

fn needs_rewrite(raw: &Value) -> bool {
    let shape = serde_json::to_value(Config::default()).expect("Config serialises");
    let Some(obj) = raw.as_object() else { return true };
    let missing_section = shape.as_object().unwrap().keys().any(|k| !obj.contains_key(k));
    missing_section || prune(&shape, raw) != *raw
}

/// Reshapes `value` to the keys of `shape`, recursively.
///
/// Arrays and primitives are taken wholesale -- only object keys are filtered,
/// and only where the shape says an object belongs.
fn prune(shape: &Value, value: &Value) -> Value {
    match (shape, value) {
        (Value::Object(s), Value::Object(v)) => {
            Value::Object(s.iter().filter_map(|(k, sv)| v.get(k).map(|vv| (k.clone(), prune(sv, vv)))).collect())
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real ~/.config/bard/config.json, byte for byte.
    const REAL: &str = "{\n\t\"libraryPaths\": [\n\t\t\"/home/user/Music\"\n\t],\n\t\"audio\": {\n\t\t\"volume\": 1,\n\t\t\"outputDeviceId\": \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",\n\t\t\"normalize\": \"track\",\n\t\t\"crossfadeSec\": 0\n\t},\n\t\"overlay\": {\n\t\t\"port\": 4747,\n\t\t\"writeNowPlayingJson\": true\n\t},\n\t\"shortcuts\": {\n\t\t\"globalMediaKeys\": false\n\t},\n\t\"window\": {\n\t\t\"closeToTray\": true,\n\t\t\"trayPopupCorner\": \"bottom-right\",\n\t\t\"trayPopupSeconds\": 8\n\t},\n\t\"playback\": {\n\t\t\"autoFill\": \"shuffle\",\n\t\t\"repeat\": \"off\",\n\t\t\"avoidRepeat\": 50,\n\t\t\"fillCount\": 10,\n\t\t\"maxQueueLength\": 200,\n\t\t\"endless\": true,\n\t\t\"endlessLookahead\": 10,\n\t\t\"excludeClaimed\": true,\n\t\t\"lovedWeight\": 3\n\t},\n\t\"streamer\": {\n\t\t\"enabled\": true,\n\t\t\"autoArm\": true,\n\t\t\"obsUrl\": \"ws://127.0.0.1:4455\",\n\t\t\"hasObsPassword\": true\n\t}\n}";

    #[test]
    fn a_current_file_loads_unchanged_and_round_trips_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, REAL).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let store = ConfigStore::open(path.clone()).unwrap();
        assert_eq!(store.get().library_paths, ["/home/user/Music"]);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before, "not rewritten");
        assert_eq!(to_js_json(store.get(), Indent::Tab).unwrap(), REAL);
    }

    #[test]
    fn a_missing_file_is_written_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.json");
        let store = ConfigStore::open(path.clone()).unwrap();
        assert_eq!(*store.get(), Config::default());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("{\n\t\"libraryPaths\": [],\n\t\"audio\": {\n\t\t\"volume\": 0.8,"));
    }

    #[test]
    fn removed_settings_are_pruned_and_missing_ones_filled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let old = REAL.replace(
            "\t\"streamer\": {",
            "\t\"twitch\": { \"channel\": \"x\" },\n\t\"streamer\": {\n\t\t\"requestRules\": 1,",
        );
        std::fs::write(&path, &old).unwrap();
        let store = ConfigStore::open(path.clone()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), REAL);
        assert!(store.get().streamer.enabled);

        // An older file without some keys: filled in on read, and not
        // rewritten just for that (only whole sections trigger a write).
        std::fs::write(&path, r#"{"libraryPaths":["/m"],"audio":{"volume":0.5},"overlay":{},"shortcuts":{},"window":{},"playback":{},"streamer":{}}"#).unwrap();
        let store = ConfigStore::open(path.clone()).unwrap();
        assert_eq!(store.get().audio.volume, 0.5);
        assert_eq!(store.get().audio.normalize, crate::types::NormalizeMode::Track);
        assert_eq!(store.get().playback.endless_lookahead, 10);
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("{\"libraryPaths\""));
    }

    #[test]
    fn a_bad_section_falls_back_without_losing_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let bad = REAL.replace("\"volume\": 1", "\"volume\": \"loud\"");
        std::fs::write(&path, bad).unwrap();
        let store = ConfigStore::open(path).unwrap();
        assert_eq!(store.get().audio.volume, 0.8);
        assert_eq!(store.get().library_paths, ["/home/user/Music"]);
        assert!(store.get().streamer.has_obs_password);
    }

    #[test]
    fn corrupt_json_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ nope").unwrap();
        assert!(ConfigStore::open(path.clone()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ nope");
    }

    type Log<T> = Arc<Mutex<Vec<T>>>;

    /// A store whose writes are recorded (volume of each), optionally slow,
    /// failing the first `fail` times; the file itself is written too.
    fn recording(delay: Duration, fail: usize) -> (tempfile::TempDir, ConfigStore, Log<f64>, Log<String>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let writes = Arc::new(Mutex::new(Vec::new()));
        let left = Arc::new(Mutex::new(fail));
        let w = writes.clone();
        let store = ConfigStore::with_writer(
            path,
            Config::default(),
            Box::new(move |p, c| {
                std::thread::sleep(delay);
                let mut left = left.lock().unwrap();
                if *left > 0 {
                    *left -= 1;
                    anyhow::bail!("disk full");
                }
                // Recorded once on disk, so a test that sees the value here
                // can read the file (Windows writes slowly enough to lose
                // that race otherwise).
                write(p, c)?;
                w.lock().unwrap().push(c.audio.volume);
                Ok(())
            }),
        );
        let said = Arc::new(Mutex::new(Vec::new()));
        let s2 = said.clone();
        store.set_notify(Arc::new(move |m| s2.lock().unwrap().push(m)));
        (dir, store, writes, said)
    }

    fn on_disk(dir: &tempfile::TempDir) -> Config {
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("config.json")).unwrap()).unwrap()
    }

    fn wait_until(f: impl Fn() -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while !f() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A drag on a slow disk: every step applies at once, nothing waits on
    /// the write, and the last value is what lands -- in one or two writes.
    #[test]
    fn slider_steps_apply_at_once_and_coalesce_into_the_last_value() {
        let (dir, mut store, writes, _) = recording(Duration::from_millis(300), 0);
        let t = std::time::Instant::now();
        for i in 0..=100 {
            let mut next = store.get().clone();
            next.audio.volume = i as f64 / 100.;
            assert!(store.set_deferred(next));
            assert_eq!(store.get().audio.volume, i as f64 / 100., "in use at once");
        }
        assert!(t.elapsed() < Duration::from_millis(100), "never waited on the disk: {:?}", t.elapsed());
        wait_until(|| writes.lock().unwrap().last() == Some(&1.0));
        assert_eq!(on_disk(&dir).audio.volume, 1.0);
        assert!(writes.lock().unwrap().len() <= 2, "coalesced: {:?}", writes.lock().unwrap());
    }

    /// Slider steps while a slow write is already on the disk: none waits
    /// for it, and the newest value lands after it. A discrete change in the
    /// same window waits its turn, and a slider step after that is newer
    /// than both.
    #[test]
    fn slider_steps_during_a_write_in_flight_do_not_wait_for_it() {
        let (dir, mut store, writes, _) = recording(Duration::from_millis(800), 0);
        let mut next = store.get().clone();
        next.audio.volume = 0.21;
        store.set_deferred(next);
        // The writer settles, then spends 800 ms writing 0.21.
        std::thread::sleep(SETTLE + Duration::from_millis(150));
        let t = std::time::Instant::now();
        for i in 0..20 {
            let mut next = store.get().clone();
            next.audio.volume = 0.3 + i as f64 / 100.;
            store.set_deferred(next);
        }
        assert!(t.elapsed() < Duration::from_millis(50), "slider waited on the disk: {:?}", t.elapsed());
        wait_until(|| writes.lock().unwrap().last() == Some(&0.49));
        assert_eq!(on_disk(&dir).audio.volume, 0.49);
        assert_eq!(*writes.lock().unwrap(), [0.21, 0.49], "in order, coalesced");

        // A transactional change while a deferred write is in flight, then
        // a slider step: the step is the newest, and lands last.
        let (dir, mut store, writes, _) = recording(Duration::from_millis(300), 0);
        let mut next = store.get().clone();
        next.audio.volume = 0.1;
        store.set_deferred(next);
        std::thread::sleep(SETTLE + Duration::from_millis(100));
        let mut next = store.get().clone();
        next.streamer.enabled = true;
        assert!(store.set(next).unwrap(), "waits for the write in flight, then writes");
        assert!(on_disk(&dir).streamer.enabled, "on disk before set returns");
        let mut next = store.get().clone();
        next.audio.volume = 0.7;
        store.set_deferred(next);
        wait_until(|| writes.lock().unwrap().last() == Some(&0.7));
        let disk = on_disk(&dir);
        assert!(disk.streamer.enabled && disk.audio.volume == 0.7, "{:?}", writes.lock().unwrap());
        assert_eq!(*writes.lock().unwrap(), [0.1, 0.1, 0.7]);
    }

    /// A discrete change while a slider value is being written behind: it
    /// waits its turn, lands with the slider value in it, and the older
    /// pending write never lands after it.
    #[test]
    fn a_transactional_change_is_never_overwritten_by_an_older_deferred_write() {
        let (dir, mut store, writes, _) = recording(Duration::from_millis(150), 0);
        let mut next = store.get().clone();
        next.audio.volume = 0.3;
        store.set_deferred(next);
        // The writer is (or soon will be) mid-write of 0.3.
        std::thread::sleep(SETTLE + Duration::from_millis(50));
        let mut next = store.get().clone();
        next.audio.volume = 0.6;
        store.set_deferred(next);
        let mut next = store.get().clone();
        next.streamer.enabled = true;
        assert!(store.set(next).unwrap());
        std::thread::sleep(SETTLE * 3);
        let disk = on_disk(&dir);
        assert!(disk.streamer.enabled);
        assert_eq!(disk.audio.volume, 0.6, "the slider value rode along");
        assert_eq!(writes.lock().unwrap().last(), Some(&0.6), "{:?}", writes.lock().unwrap());
    }

    /// A failing disk: the value stays in use, the user is told once, the
    /// write is retried on its own, and recovery is said.
    #[test]
    fn a_failing_deferred_write_is_retried_and_reported() {
        let (dir, mut store, _, said) = recording(Duration::ZERO, 2);
        let mut next = store.get().clone();
        next.audio.crossfade_sec = 5.;
        store.set_deferred(next);
        wait_until(|| dir.path().join("config.json").exists() && on_disk(&dir).audio.crossfade_sec == 5.);
        assert_eq!(on_disk(&dir).audio.crossfade_sec, 5.);
        let said = said.lock().unwrap().clone();
        assert_eq!(said.iter().filter(|m| m.starts_with("Could not save settings")).count(), 1, "{said:?}");
        assert!(said.iter().any(|m| m.contains("saving again")), "{said:?}");
    }

    /// Quitting right after a drag keeps its last value.
    #[test]
    fn the_last_value_is_written_on_shutdown() {
        let (dir, mut store, _, _) = recording(Duration::ZERO, 0);
        let mut next = store.get().clone();
        next.audio.volume = 0.42;
        store.set_deferred(next);
        drop(store); // before the writer's settle time
        assert_eq!(on_disk(&dir).audio.volume, 0.42);

        let (dir, mut store, _, _) = recording(Duration::ZERO, 0);
        let mut next = store.get().clone();
        next.audio.volume = 0.24;
        store.set_deferred(next);
        assert!(store.flush());
        assert_eq!(on_disk(&dir).audio.volume, 0.24);
    }

    #[test]
    fn set_writes_only_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = ConfigStore::open(path.clone()).unwrap();
        let mut next = store.get().clone();
        assert!(!store.set(next.clone()).unwrap());
        next.audio.volume = 1.0;
        assert!(store.set(next).unwrap());
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"volume\": 1,"));
    }
}
