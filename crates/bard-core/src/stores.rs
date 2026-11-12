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
//! the facade calls it on shutdown.

use crate::types::{AlbumFlag, AlbumFlags, ContentIdStatus, StatsMap, TrackStats};
use crate::util::{Indent, to_js_json, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

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
}

/// A value persisted to one JSON file with debounced, atomic writes.
pub(crate) struct Debounced<T> {
    inner: Arc<Mutex<Inner<T>>>,
    path: PathBuf,
    delay: Duration,
    indent: Indent,
}

impl<T: Serialize + Send + 'static> Debounced<T> {
    pub fn new(path: PathBuf, data: T, delay: Duration, indent: Indent) -> Self {
        Debounced {
            inner: Arc::new(Mutex::new(Inner { data, dirty: false, timer: false })),
            path,
            delay,
            indent,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner<T>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.lock().data)
    }

    /// Mutates and schedules a write.
    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut g = self.lock();
        let out = f(&mut g.data);
        g.dirty = true;
        if !g.timer {
            g.timer = true;
            let inner = Arc::clone(&self.inner);
            let (path, delay, indent) = (self.path.clone(), self.delay, self.indent);
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let mut g = inner.lock().unwrap_or_else(|e| e.into_inner());
                g.timer = false;
                flush_locked(&mut g, &path, indent);
            });
        }
        out
    }

    pub fn flush(&self) {
        let mut g = self.lock();
        flush_locked(&mut g, &self.path, self.indent);
    }
}

fn flush_locked<T: Serialize>(g: &mut Inner<T>, path: &Path, indent: Indent) {
    if !g.dirty {
        return;
    }
    g.dirty = false;
    let ok = to_js_json(&g.data, indent)
        .map_err(std::io::Error::other)
        .and_then(|body| write_atomic(path, &body));
    if ok.is_err() {
        // Losing counts or flags must never interrupt playback; try again on
        // the next write.
        g.dirty = true;
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

// --- stats.json --------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct StatsFile {
    pub version: u32,
    pub tracks: StatsMap,
}

pub(crate) struct Stats(Debounced<StatsFile>);

impl Stats {
    pub fn load(path: PathBuf, delay: Duration) -> Self {
        // Absent or unreadable: start fresh rather than refusing to play music.
        let data = read_json::<StatsFile>(&path).unwrap_or(StatsFile { version: 1, tracks: StatsMap::new() });
        Stats(Debounced::new(path, data, delay, Indent::Compact))
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

    pub fn flush(&self) {
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
    pub fn load(path: PathBuf, delay: Duration) -> Self {
        // Absent or unreadable: everything is simply untested.
        let data = read_json::<AlbumsFile>(&path)
            .unwrap_or(AlbumsFile { version: 1, albums: AlbumFlags::new() });
        Albums(Debounced::new(path, data, delay, Indent::Two))
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
        let flag = AlbumFlag {
            status,
            checked_at: (status != ContentIdStatus::Untested).then_some(now),
            note,
        };
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
            a.albums
                .iter()
                .filter(|(_, f)| f.status == ContentIdStatus::Claimed)
                .map(|(n, _)| n.clone())
                .collect()
        })
    }

    pub fn flush(&self) {
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
    pub fn load(path: PathBuf, delay: Duration) -> Self {
        // No file yet: nothing is loved.
        let mut data = read_json::<LovedFile>(&path).unwrap_or_default();
        data.version = 1;
        let mut seen = std::collections::HashSet::new();
        data.loved.retain(|id| seen.insert(id.clone()));
        Loved(Debounced::new(path, data, delay, Indent::Two))
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

    pub fn flush(&self) {
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
        let s = Stats::load(path.clone(), Duration::from_millis(20));
        s.record_play("abc", 1000);
        s.record_skip("abc", 2000);
        assert!(!path.exists(), "debounced, not written at once");
        std::thread::sleep(Duration::from_millis(200));
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            body,
            r#"{"version":1,"tracks":{"abc":{"plays":1,"skips":1,"lastPlayedAt":1000,"lastSkippedAt":2000}}}"#
        );
        let again = Stats::load(path, STATS_DEBOUNCE);
        assert_eq!(again.get("abc").plays, 1);
        assert_eq!(again.get("nope"), TrackStats::default());
    }

    #[test]
    fn unreadable_stats_start_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(Stats::load(path, STATS_DEBOUNCE).all().is_empty());
    }

    #[test]
    fn album_flags_trim_notes_and_drop_untested_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("albums.json");
        let a = Albums::load(path.clone(), ALBUMS_DEBOUNCE);
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
        let l = Loved::load(path.clone(), LOVED_DEBOUNCE);
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
        assert!(Loved::load(path, LOVED_DEBOUNCE).contains("a"));
    }

    #[test]
    fn real_install_shapes_parse() {
        let a: AlbumsFile = serde_json::from_str(
            r#"{"version":1,"albums":{"Rain":{"status":"safe","checkedAt":1786252986373}}}"#,
        )
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
