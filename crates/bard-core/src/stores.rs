//! The small JSON stores beside config.json: stats.json, albums.json and
//! loved.json. Ports of `src/main/stats.ts`, `albums.ts` and `loved.ts`.
//!
//! Each is its own file, apart from config.json and from each other, because
//! they change at different rates and mean different things:
//!
//! - **stats** change on every track while settings change on user action;
//!   mixing them would rewrite the settings file constantly and make a corrupt
//!   write take the settings down with it.
//! - **albums** are findings the user records about the world, not settings
//!   and not derived numbers. Marking an album `claimed` is what keeps it out
//!   of auto-play, so losing this file would quietly put flagged music back on
//!   stream.
//! - **loved** is something the user declared. Play counts can be rebuilt by
//!   listening again; a lost list of favourites cannot.
//!
//! Writes are debounced and atomic (temp + rename), so a crash mid-track costs
//! at most the last few seconds rather than the whole history. The debounce
//! runs on a short-lived thread; [`Debounced::flush`] writes immediately, and
//! the facade calls it on shutdown. A failed write is retried, with backoff,
//! and said once (and again when saving works again).
//!
//! A file that cannot be used is never silently replaced. Each one used to
//! load as empty on any failure, and the next play, flag or love wrote that
//! empty store over it -- one edit after a bad read erased the history.
//! Now ([`LoadState`]):
//! - missing: start empty, as before;
//! - not valid JSON: the file is moved aside as `<name>.corrupt-<ms>` --
//!   bytes intact, for recovery by hand -- and the store starts empty;
//! - unreadable (permissions, I/O): the store runs in memory and does not
//!   write that path this session; a later start reads it again.
//!
//! Either way the user is told, and music keeps playing.

use crate::types::{AlbumFlag, AlbumFlags, ContentIdStatus, StatsMap, TrackStats};
use crate::util::{Indent, to_js_json, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Tells the user something about a store (the facade sends a notice).
pub(crate) type Notify = Arc<dyn Fn(String) + Send + Sync>;

/// Longest wait between retries of a failing write.
const MAX_RETRY: Duration = Duration::from_secs(60);

/// How a store came to have what it has, and whether it can be saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    /// Read from disk, or no file yet.
    Ok,
    /// The file was not valid JSON. It was moved to `backup`; the store
    /// started empty and saves normally.
    Recovered { backup: PathBuf },
    /// The file could not be read. Kept in memory only, so what is on disk
    /// survives for a later start.
    ReadOnly { reason: String },
}

pub const STATS_DEBOUNCE: Duration = Duration::from_millis(3000);
pub const ALBUMS_DEBOUNCE: Duration = Duration::from_millis(1500);
pub const LOVED_DEBOUNCE: Duration = Duration::from_millis(1500);

/// A track counts as played once it has been heard this far...
pub const PLAY_THRESHOLD_SEC: f64 = 30.0;
/// ...or this proportion of it, whichever comes first, for short tracks.
pub const PLAY_THRESHOLD_RATIO: f64 = 0.5;

struct Inner<T> {
    data: T,
    dirty: bool,
    timer: bool,
    /// The last write failed (said once, until one succeeds).
    failing: bool,
    /// Never write the file (it could not be read).
    read_only: bool,
}

/// A value persisted to one JSON file with debounced, atomic writes.
pub(crate) struct Debounced<T> {
    inner: Arc<Mutex<Inner<T>>>,
    path: PathBuf,
    delay: Duration,
    indent: Indent,
    state: LoadState,
    notify: Notify,
}

/// Reads `path`, or says why it could not be used. A file that is not valid
/// JSON is moved aside first.
fn load<T: for<'de> Deserialize<'de>>(path: &Path, fresh: impl FnOnce() -> T) -> (T, LoadState) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (fresh(), LoadState::Ok),
        Err(e) => return (fresh(), LoadState::ReadOnly { reason: format!("{name} could not be read: {e}") }),
    };
    match serde_json::from_str(&body) {
        Ok(v) => (v, LoadState::Ok),
        Err(_) => {
            let mut backup = path.as_os_str().to_owned();
            backup.push(format!(".corrupt-{}", crate::util::now_ms()));
            let backup = PathBuf::from(backup);
            match std::fs::rename(path, &backup) {
                Ok(()) => (fresh(), LoadState::Recovered { backup }),
                // Could not keep the bytes safe: do not write over them.
                Err(e) => (
                    fresh(),
                    LoadState::ReadOnly { reason: format!("{name} is damaged and could not be set aside: {e}") },
                ),
            }
        }
    }
}

impl<T: Serialize + for<'de> Deserialize<'de> + Send + 'static> Debounced<T> {
    /// Loads `path` (see [`LoadState`]), telling the user through `notify`
    /// when it could not be used as it was.
    pub fn open(path: PathBuf, fresh: impl FnOnce() -> T, delay: Duration, indent: Indent, notify: Notify) -> Self {
        let (data, state) = load(&path, fresh);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match &state {
            LoadState::Ok => {}
            LoadState::Recovered { backup } => notify(format!(
                "{name} was damaged and has been set aside as {} \u{2014} starting it fresh",
                backup.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            )),
            LoadState::ReadOnly { reason } => {
                notify(format!("{reason} \u{2014} changes to it this session will not be saved"))
            }
        }
        let read_only = matches!(state, LoadState::ReadOnly { .. });
        Debounced {
            inner: Arc::new(Mutex::new(Inner { data, dirty: false, timer: false, failing: false, read_only })),
            path,
            delay,
            indent,
            state,
            notify,
        }
    }

    pub fn state(&self) -> &LoadState {
        &self.state
    }

    fn lock(&self) -> MutexGuard<'_, Inner<T>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.lock().data)
    }

    /// Normalises what was loaded without scheduling a write.
    fn read_mut_quietly(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.lock().data)
    }

    /// Mutates and schedules a write.
    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut g = self.lock();
        let out = f(&mut g.data);
        g.dirty = true;
        if !g.timer && !g.read_only {
            g.timer = true;
            let inner = Arc::clone(&self.inner);
            let (path, delay, indent, notify) = (self.path.clone(), self.delay, self.indent, self.notify.clone());
            std::thread::spawn(move || {
                // Until it is saved: a failed write waits longer each time
                // rather than waiting for the next change to try again.
                let mut wait = delay;
                loop {
                    std::thread::sleep(wait);
                    let mut g = inner.lock().unwrap_or_else(|e| e.into_inner());
                    flush_locked(&mut g, &path, indent, &notify);
                    if !g.dirty {
                        g.timer = false;
                        return;
                    }
                    wait = (wait.max(Duration::from_millis(1)) * 2).min(MAX_RETRY);
                }
            });
        }
        out
    }

    /// Writes now. Returns whether the store is saved (false while writes
    /// fail, and for a store that is not being saved).
    pub fn flush(&self) -> bool {
        let mut g = self.lock();
        flush_locked(&mut g, &self.path, self.indent, &self.notify);
        !g.dirty && !g.read_only
    }
}

fn flush_locked<T: Serialize>(g: &mut Inner<T>, path: &Path, indent: Indent, notify: &Notify) {
    if !g.dirty || g.read_only {
        return;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match to_js_json(&g.data, indent).map_err(std::io::Error::other).and_then(|body| write_atomic(path, &body)) {
        Ok(()) => {
            g.dirty = false;
            if std::mem::take(&mut g.failing) {
                notify(format!("{name} is saving again"));
            }
        }
        // Losing counts or flags must never interrupt playback: keep them in
        // memory, retry, and say so once.
        Err(e) => {
            if !std::mem::replace(&mut g.failing, true) {
                notify(format!("Could not save {name}: {e} \u{2014} will keep trying"));
            }
        }
    }
}

// --- stats.json --------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct StatsFile {
    pub version: u32,
    pub tracks: StatsMap,
}

pub(crate) struct Stats(Debounced<StatsFile>);

impl Stats {
    pub fn load(path: PathBuf, delay: Duration, notify: Notify) -> Self {
        let fresh = || StatsFile { version: 1, tracks: StatsMap::new() };
        Stats(Debounced::open(path, fresh, delay, Indent::Compact, notify))
    }

    pub fn state(&self) -> &LoadState {
        self.0.state()
    }

    pub fn all(&self) -> StatsMap {
        self.0.read(|s| s.tracks.clone())
    }

    pub fn get(&self, id: &str) -> TrackStats {
        self.0.read(|s| s.tracks.get(id).cloned().unwrap_or_default())
    }

    pub fn record_play(&self, id: &str, at: i64) -> TrackStats {
        self.0.update(|s| {
            let e = s.tracks.entry(id.to_string()).or_default();
            e.plays += 1;
            e.last_played_at = Some(at);
            e.clone()
        })
    }

    /// A track abandoned before the threshold.
    ///
    /// Counted separately rather than not at all: a track you skip every time
    /// is a fact worth surfacing, and lumping it in with plays would make the
    /// most-played list meaningless.
    pub fn record_skip(&self, id: &str, at: i64) -> TrackStats {
        self.0.update(|s| {
            let e = s.tracks.entry(id.to_string()).or_default();
            e.skips += 1;
            e.last_skipped_at = Some(at);
            e.clone()
        })
    }

    pub fn clear(&self) {
        self.0.update(|s| *s = StatsFile { version: 1, tracks: StatsMap::new() });
    }

    pub fn flush(&self) -> bool {
        self.0.flush()
    }
}

// --- albums.json -------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AlbumsFile {
    pub version: u32,
    pub albums: AlbumFlags,
}

pub(crate) struct Albums(Debounced<AlbumsFile>);

impl Albums {
    pub fn load(path: PathBuf, delay: Duration, notify: Notify) -> Self {
        let fresh = || AlbumsFile { version: 1, albums: AlbumFlags::new() };
        Albums(Debounced::open(path, fresh, delay, Indent::Two, notify))
    }

    pub fn state(&self) -> &LoadState {
        self.0.state()
    }

    pub fn all(&self) -> AlbumFlags {
        self.0.read(|a| a.albums.clone())
    }

    pub fn status(&self, album: &str) -> ContentIdStatus {
        self.0.read(|a| a.albums.get(album).map(|f| f.status).unwrap_or_default())
    }

    /// Records a Content ID result. An album set back to `untested` with no
    /// note is removed outright rather than stored as a no-op entry.
    pub fn set(&self, album: &str, status: ContentIdStatus, note: Option<&str>, now: i64) -> AlbumFlag {
        let note = note.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
        let flag = AlbumFlag { status, checked_at: (status != ContentIdStatus::Untested).then_some(now), note };
        self.0.update(|a| {
            if status == ContentIdStatus::Untested && flag.note.is_none() {
                a.albums.remove(album);
            } else {
                a.albums.insert(album.to_string(), flag.clone());
            }
        });
        flag
    }

    /// Albums the user has marked as claimed.
    pub fn claimed(&self) -> std::collections::HashSet<String> {
        self.0.read(|a| {
            a.albums.iter().filter(|(_, f)| f.status == ContentIdStatus::Claimed).map(|(n, _)| n.clone()).collect()
        })
    }

    pub fn flush(&self) -> bool {
        self.0.flush()
    }
}

// --- loved.json --------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct LovedFile {
    pub version: u32,
    /// In the order they were loved, as the JavaScript Set kept them.
    pub loved: Vec<String>,
}

pub(crate) struct Loved(Debounced<LovedFile>);

impl Loved {
    pub fn load(path: PathBuf, delay: Duration, notify: Notify) -> Self {
        let store = Debounced::open(path, LovedFile::default, delay, Indent::Two, notify);
        store.read_mut_quietly(|data| {
            data.version = 1;
            let mut seen = std::collections::HashSet::new();
            data.loved.retain(|id| seen.insert(id.clone()));
        });
        Loved(store)
    }

    pub fn state(&self) -> &LoadState {
        self.0.state()
    }

    pub fn all(&self) -> Vec<String> {
        self.0.read(|l| l.loved.clone())
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.read(|l| l.loved.iter().any(|x| x == id))
    }

    /// Returns whether the track is loved afterwards.
    pub fn toggle(&self, id: &str) -> bool {
        self.0.update(|l| {
            if let Some(i) = l.loved.iter().position(|x| x == id) {
                l.loved.remove(i);
                false
            } else {
                l.loved.push(id.to_string());
                true
            }
        })
    }

    pub fn flush(&self) -> bool {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_write_compact_json_after_the_debounce() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.json");
        let s = Stats::load(path.clone(), Duration::from_millis(20), quiet());
        s.record_play("abc", 1000);
        s.record_skip("abc", 2000);
        assert!(!path.exists(), "debounced, not written at once");
        std::thread::sleep(Duration::from_millis(200));
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            body,
            r#"{"version":1,"tracks":{"abc":{"plays":1,"skips":1,"lastPlayedAt":1000,"lastSkippedAt":2000}}}"#
        );
        let again = Stats::load(path, STATS_DEBOUNCE, quiet());
        assert_eq!(again.get("abc").plays, 1);
        assert_eq!(again.get("nope"), TrackStats::default());
    }

    fn quiet() -> Notify {
        Arc::new(|_| {})
    }

    /// A Notify that keeps what it was told.
    fn told() -> (Notify, Arc<Mutex<Vec<String>>>) {
        let said = Arc::new(Mutex::new(Vec::new()));
        let s = said.clone();
        (Arc::new(move |m| s.lock().unwrap().push(m)), said)
    }

    fn backups(dir: &Path, name: &str) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(&format!("{name}.corrupt-")))
            .collect();
        v.sort();
        v
    }

    /// Malformed or empty: the bytes are kept aside, untouched, the store
    /// starts fresh and saves normally, and the user is told.
    #[test]
    fn a_damaged_store_is_set_aside_not_overwritten() {
        for body in [r#"{ "version": 1, "tracks": { "abc": {"plays": 79"#, ""] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("stats.json");
            std::fs::write(&path, body).unwrap();
            let (notify, said) = told();
            let s = Stats::load(path.clone(), STATS_DEBOUNCE, notify);
            let kept = backups(dir.path(), "stats.json");
            assert_eq!(kept.len(), 1);
            assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), body, "the original bytes");
            assert_eq!(s.state(), &LoadState::Recovered { backup: kept[0].clone() });
            assert!(s.all().is_empty());
            let msgs = said.lock().unwrap().clone();
            assert_eq!(msgs.len(), 1);
            assert!(msgs[0].contains("set aside"), "{msgs:?}");

            s.record_play("x", 1);
            assert!(s.flush());
            assert!(std::fs::read_to_string(&path).unwrap().contains(r#""x""#));
            assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), body, "still untouched");
        }
    }

    /// The same for the other two, which matter more: flags keep claimed
    /// music off stream, loves cannot be rebuilt.
    #[test]
    fn damaged_albums_and_loved_are_set_aside_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("albums.json"), r#"{"albums": ["#).unwrap();
        std::fs::write(dir.path().join("loved.json"), "nope").unwrap();
        let a = Albums::load(dir.path().join("albums.json"), ALBUMS_DEBOUNCE, quiet());
        let l = Loved::load(dir.path().join("loved.json"), LOVED_DEBOUNCE, quiet());
        assert!(matches!(a.state(), LoadState::Recovered { .. }));
        assert!(matches!(l.state(), LoadState::Recovered { .. }));
        assert_eq!(std::fs::read_to_string(&backups(dir.path(), "albums.json")[0]).unwrap(), r#"{"albums": ["#);
        assert_eq!(std::fs::read_to_string(&backups(dir.path(), "loved.json")[0]).unwrap(), "nope");
    }

    /// A file that cannot be read is never written: the session runs in
    /// memory, and the next start reads it intact.
    #[test]
    fn an_unreadable_store_is_left_alone_for_a_later_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("loved.json");
        let original = "{\n  \"version\": 1,\n  \"loved\": [\n    \"keep\"\n  ]\n}";
        std::fs::write(&path, original).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            return; // running as root: permissions do not stop a read
        }
        let (notify, said) = told();
        let l = Loved::load(path.clone(), Duration::from_millis(10), notify);
        assert!(matches!(l.state(), LoadState::ReadOnly { .. }), "{:?}", l.state());
        let msgs = said.lock().unwrap().clone();
        assert!(msgs[0].contains("will not be saved"), "{msgs:?}");
        assert!(l.toggle("new"), "works in memory");
        assert!(!l.flush(), "not saved");
        std::thread::sleep(Duration::from_millis(50));

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original, "untouched");
        assert!(backups(dir.path(), "loved.json").is_empty());
        let later = Loved::load(path, LOVED_DEBOUNCE, quiet());
        assert_eq!(later.state(), &LoadState::Ok);
        assert_eq!(later.all(), ["keep"]);
    }

    /// Other read errors (here: a directory where the file should be) too.
    #[test]
    fn a_read_error_is_not_mistaken_for_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("albums.json");
        std::fs::create_dir(&path).unwrap();
        let a = Albums::load(path.clone(), ALBUMS_DEBOUNCE, quiet());
        assert!(matches!(a.state(), LoadState::ReadOnly { .. }));
        a.set("Gold", ContentIdStatus::Claimed, None, 1);
        assert!(!a.flush());
        assert!(path.is_dir());
    }

    /// A write that fails is retried without waiting for another change, and
    /// both the failure and the recovery are said once.
    #[test]
    fn a_failing_write_is_retried_and_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("data");
        std::fs::create_dir(&sub).unwrap();
        let path = sub.join("stats.json");
        let (notify, said) = told();
        let s = Stats::load(path.clone(), Duration::from_millis(10), notify);
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(sub.join("probe"), "").is_ok() {
            std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
            return; // running as root
        }
        s.record_play("a", 1);
        s.record_play("b", 2);
        std::thread::sleep(Duration::from_millis(150));
        assert!(!path.exists());
        let failures = said.lock().unwrap().iter().filter(|m| m.starts_with("Could not save")).count();
        assert_eq!(failures, 1, "said once: {:?}", said.lock().unwrap());

        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(std::fs::read_to_string(&path).unwrap().contains(r#""b""#), "retried on its own");
        std::thread::sleep(Duration::from_millis(20));
        assert!(said.lock().unwrap().iter().any(|m| m.contains("saving again")), "{:?}", said.lock().unwrap());
    }

    /// A crash mid-write leaves the temp file beside an intact store: the
    /// store loads, and the next write replaces the leftover.
    #[test]
    fn a_leftover_temp_file_from_an_interrupted_write_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.json");
        std::fs::write(&path, r#"{"version":1,"tracks":{"abc":{"plays":3,"skips":0}}}"#).unwrap();
        std::fs::write(dir.path().join("stats.json.tmp"), r#"{"version":1,"tra"#).unwrap();
        let s = Stats::load(path.clone(), STATS_DEBOUNCE, quiet());
        assert_eq!(s.state(), &LoadState::Ok);
        assert_eq!(s.get("abc").plays, 3);
        s.record_play("abc", 9);
        assert!(s.flush());
        assert!(!dir.path().join("stats.json.tmp").exists());
        assert_eq!(Stats::load(path, STATS_DEBOUNCE, quiet()).get("abc").plays, 4);
    }

    #[test]
    fn album_flags_trim_notes_and_drop_untested_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("albums.json");
        let a = Albums::load(path.clone(), ALBUMS_DEBOUNCE, quiet());
        let f = a.set("Gold", ContentIdStatus::Claimed, Some("  claimed by X  "), 42);
        assert_eq!(f.note.as_deref(), Some("claimed by X"));
        assert_eq!(f.checked_at, Some(42));
        a.set("Rain", ContentIdStatus::Safe, Some("   "), 43);
        assert_eq!(a.all()["Rain"].note, None);
        assert_eq!(a.claimed().into_iter().collect::<Vec<_>>(), ["Gold"]);

        let u = a.set("Rain", ContentIdStatus::Untested, None, 44);
        assert_eq!(u.checked_at, None);
        assert!(!a.all().contains_key("Rain"));
        // Untested with a note is kept, without a checkedAt.
        a.set("Moon", ContentIdStatus::Untested, Some("ask"), 45);
        assert_eq!(a.status("Moon"), ContentIdStatus::Untested);
        assert!(a.all().contains_key("Moon"));

        a.flush();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("{\n  \"version\": 1,\n  \"albums\": {"), "{body}");
        assert!(body.contains("\"status\": \"claimed\",\n      \"note\": \"claimed by X\",\n      \"checkedAt\": 42"));
    }

    #[test]
    fn loved_toggles_and_keeps_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("loved.json");
        let l = Loved::load(path.clone(), LOVED_DEBOUNCE, quiet());
        assert!(l.toggle("b"));
        assert!(l.toggle("a"));
        assert!(!l.toggle("b"));
        assert!(l.toggle("b"));
        assert_eq!(l.all(), ["a", "b"]);
        l.flush();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"loved\": [\n    \"a\",\n    \"b\"\n  ]\n}"
        );
        assert!(Loved::load(path, LOVED_DEBOUNCE, quiet()).contains("a"));
    }

    #[test]
    fn real_install_shapes_parse() {
        let a: AlbumsFile =
            serde_json::from_str(r#"{"version":1,"albums":{"Rain":{"status":"safe","checkedAt":1786252986373}}}"#)
                .unwrap();
        assert_eq!(a.albums["Rain"].status, ContentIdStatus::Safe);
        let s: StatsFile = serde_json::from_str(
            r#"{"version":1,"tracks":{"50f36209b3904081":{"plays":79,"skips":24,"lastSkippedAt":1786252001477,"lastPlayedAt":1794339303386}}}"#,
        )
        .unwrap();
        assert_eq!(s.tracks["50f36209b3904081"].plays, 79);
        let l: LovedFile = serde_json::from_str("{\n  \"version\": 1,\n  \"loved\": []\n}").unwrap();
        assert!(l.loved.is_empty());
    }
}
