//! What the window shows, and where its intents go.
//!
//! The React renderer split this across a zustand store (mirrored server state
//! plus a little local UI state) and `window.bard` (intents to the main
//! process). The same split survives: `Ui` is the store, `Backend` is the
//! bridge. The views read `Ui` and call `Backend`; they never touch the core or
//! the audio engine directly, which is what lets the snapshot tool render the
//! real views from fixture data.

use bard_core::types::{AlbumFlags, Config, ContentIdStatus, QueueItem, ScanProgress, StatsMap, StreamerSource, Track};
use gpui::RenderImage;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// Intents from the window. Mirrors the invoke half of `BardApi`.
pub trait Backend {
    fn enqueue(&self, track_id: &str);
    fn enqueue_many(&self, track_ids: &[String], start_now: bool);
    fn play_now(&self, track_id: &str);
    fn remove_from_queue(&self, queue_item_id: &str);
    fn reorder_queue(&self, from: usize, to: usize);
    fn skip(&self);
    fn previous(&self);
    fn clear_queue(&self);
    fn shuffle_queue(&self);
    fn fill_queue(&self);
    fn toggle_play(&self);
    fn seek(&self, seconds: f64);
    fn set_volume(&self, volume: f64);
    fn update_config(&self, f: Box<dyn FnOnce(&mut Config)>);
    fn set_album_flag(&self, album: &str, status: ContentIdStatus, note: Option<String>);
    fn toggle_loved(&self, track_id: &str);
    fn rescan(&self);
    fn choose_library_folder(&self);
    /// Output sinks as (id, name), without the "System default" row.
    fn outputs(&self) -> Vec<(String, String)>;
    /// Applies an output device. Err carries the message to show.
    fn set_output(&self, id: &str) -> Result<(), String>;
    fn paths(&self) -> Option<bard_core::types::AppPaths>;
    fn reveal(&self, path: &str);
    /// Stores the OBS password in the keyring, or clears it when empty, in
    /// the background. The outcome arrives later and lands in
    /// `Ui::obs_message`; it is never written as plaintext.
    fn set_obs_password(&self, password: &str);
    /// Arms the sleep timer: at `at`, playback pauses if `Ui::sleep_ends_at`
    /// still says `at` (a cancelled or replaced timer then does nothing).
    fn schedule_sleep(&self, at: Instant);
}

/// Does nothing. The snapshot tool renders with this.
pub struct NullBackend;

impl Backend for NullBackend {
    fn enqueue(&self, _: &str) {}
    fn enqueue_many(&self, _: &[String], _: bool) {}
    fn play_now(&self, _: &str) {}
    fn remove_from_queue(&self, _: &str) {}
    fn reorder_queue(&self, _: usize, _: usize) {}
    fn skip(&self) {}
    fn previous(&self) {}
    fn clear_queue(&self) {}
    fn shuffle_queue(&self) {}
    fn fill_queue(&self) {}
    fn toggle_play(&self) {}
    fn seek(&self, _: f64) {}
    fn set_volume(&self, _: f64) {}
    fn update_config(&self, _: Box<dyn FnOnce(&mut Config)>) {}
    fn set_album_flag(&self, _: &str, _: ContentIdStatus, _: Option<String>) {}
    fn toggle_loved(&self, _: &str) {}
    fn rescan(&self) {}
    fn choose_library_folder(&self) {}
    fn outputs(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    fn set_output(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn paths(&self) -> Option<bard_core::types::AppPaths> {
        None
    }
    fn reveal(&self, _: &str) {}
    fn set_obs_password(&self, _: &str) {}
    fn schedule_sleep(&self, _: Instant) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryView {
    Albums,
    Tracks,
    Loved,
}

pub struct Album {
    pub name: String,
    pub artist: String,
    pub tracks: Vec<Track>,
    pub duration: f64,
    /// First track with artwork -- the cover is the same for the whole album.
    pub art_track_id: Option<String>,
}

/// Groups the flat track list into albums, in library (path) order.
pub fn group_albums(tracks: &[Track]) -> Vec<Album> {
    let mut order: Vec<Album> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for t in tracks {
        let i = *index.entry(t.album.as_str()).or_insert_with(|| {
            order.push(Album {
                name: t.album.clone(),
                artist: t.artist.clone(),
                tracks: Vec::new(),
                duration: 0.0,
                art_track_id: None,
            });
            order.len() - 1
        });
        let album = &mut order[i];
        album.tracks.push(t.clone());
        album.duration += t.duration_sec;
        if album.art_track_id.is_none() && t.has_artwork {
            album.art_track_id = Some(t.id.clone());
        }
    }
    order
}

/// m:ss. Tabular figures in the UI keep these from jittering as they count.
pub fn fmt_time(seconds: f64) -> String {
    let t = if seconds.is_finite() { seconds.max(0.0).floor() as u64 } else { 0 };
    format!("{}:{:02}", t / 60, t % 60)
}

/// `toLocaleString()` for a count, en-US style: 1,284.
pub fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Which rendition of a cover: sized for where it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoverKind {
    /// An album card: drawn about 115px wide, so decoded at 240px -- enough
    /// for a 2x display -- rather than the cached 512px.
    Thumb,
    /// The now-playing cover, at full size.
    Full,
    /// The blurred backdrop behind the now-playing pane.
    Ambient,
}

impl CoverKind {
    /// How many of each are kept decoded. A card scrolled back into view
    /// past the limit is decoded again, briefly showing its placeholder.
    fn capacity(self) -> usize {
        match self {
            CoverKind::Thumb => 160,
            CoverKind::Full | CoverKind::Ambient => 6,
        }
    }
}

/// Edge of a [`CoverKind::Thumb`] in pixels.
const THUMB_PX: u32 = 240;

type CoverKey = (CoverKind, String);

/// Decoded covers, keyed by kind and track id.
///
/// Decoding a JPEG takes milliseconds, so it happens off the render path: a
/// miss returns None (the view draws its placeholder) and is queued; the
/// window decodes queued covers in the background ([`Covers::load`]), hands
/// them back with [`Covers::insert`] and redraws. Each kind keeps its most
/// recently used covers up to a limit; the evicted ones are handed to the
/// window to release from the GPU too ([`Covers::take_evicted`]).
///
/// `None` in the cache records a cover that could not be read, so it is not
/// retried every frame.
#[derive(Default)]
pub struct Covers {
    pub dir: PathBuf,
    /// Decode on the spot instead: the snapshot tool and the tray preview,
    /// which draw once and want the cover in that frame.
    sync: bool,
    cache: HashMap<CoverKey, Option<Arc<RenderImage>>>,
    /// Least recently used first, per kind.
    order: HashMap<CoverKind, VecDeque<String>>,
    /// Queued or being decoded.
    requested: HashSet<CoverKey>,
    wanted: Vec<CoverKey>,
    evicted: Vec<Arc<RenderImage>>,
}

impl Covers {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, ..Default::default() }
    }

    /// Covers decoded as they are asked for.
    pub fn new_sync(dir: PathBuf) -> Self {
        Self { dir, sync: true, ..Default::default() }
    }

    pub fn cover(&mut self, id: &str) -> Option<Arc<RenderImage>> {
        self.get(CoverKind::Full, id)
    }

    pub fn thumb(&mut self, id: &str) -> Option<Arc<RenderImage>> {
        self.get(CoverKind::Thumb, id)
    }

    pub fn ambient(&mut self, id: &str) -> Option<Arc<RenderImage>> {
        self.get(CoverKind::Ambient, id)
    }

    fn get(&mut self, kind: CoverKind, id: &str) -> Option<Arc<RenderImage>> {
        let key = (kind, id.to_string());
        if let Some(hit) = self.cache.get(&key).cloned() {
            self.touch(&key);
            return hit;
        }
        if self.sync {
            let img = Self::load(&self.dir, kind, id);
            self.insert(key, img.clone());
            return img;
        }
        if self.requested.insert(key.clone()) {
            self.wanted.push(key);
        }
        None
    }

    fn touch(&mut self, (kind, id): &CoverKey) {
        let order = self.order.entry(*kind).or_default();
        if order.back() != Some(id)
            && let Some(i) = order.iter().position(|x| x == id)
        {
            let id = order.remove(i).expect("found");
            order.push_back(id);
        }
    }

    /// Covers asked for and not yet decoded; the caller decodes them.
    pub fn take_wanted(&mut self) -> Vec<CoverKey> {
        std::mem::take(&mut self.wanted)
    }

    /// Covers dropped from the cache, to release from the GPU.
    pub fn take_evicted(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.evicted)
    }

    /// Forgets requests whose results will never arrive -- the window that
    /// would have received them is gone -- so they are made again.
    pub fn forget_requests(&mut self) {
        self.requested.clear();
        self.wanted.clear();
    }

    /// Decodes a cover. Slow; call off the render path.
    pub fn load(dir: &Path, kind: CoverKind, id: &str) -> Option<Arc<RenderImage>> {
        let img = image::open(dir.join(format!("{id}.jpg"))).ok()?;
        Some(match kind {
            CoverKind::Full => crate::backdrop::render_image(img.to_rgba8()),
            CoverKind::Thumb => crate::backdrop::render_image(img.thumbnail(THUMB_PX, THUMB_PX).to_rgba8()),
            CoverKind::Ambient => crate::backdrop::ambient(&img),
        })
    }

    pub fn insert(&mut self, key: CoverKey, img: Option<Arc<RenderImage>>) {
        self.requested.remove(&key);
        let (kind, id) = key.clone();
        if let Some(Some(old)) = self.cache.insert(key, img) {
            self.evicted.push(old);
        }
        let order = self.order.entry(kind).or_default();
        order.retain(|x| *x != id);
        order.push_back(id);
        while order.len() > kind.capacity() {
            let gone = order.pop_front().expect("over capacity");
            if let Some(Some(img)) = self.cache.remove(&(kind, gone)) {
                self.evicted.push(img);
            }
        }
    }
}

/// What the library pane lists, kept until what it is derived from changes:
/// regrouping and filtering every track on every frame (and the playing
/// string redraws several times a second) is the cost on a large library.
#[derive(Default)]
struct Derived {
    albums: Option<(AlbumsKey, Rc<Vec<Album>>)>,
    album_count: Option<(u64, usize)>,
    tracks: Option<(TracksKey, Rc<Vec<Track>>)>,
}

type TracksKey = (u64, String, Option<String>, LibraryView, Vec<String>);
type AlbumsKey = (u64, String);

pub struct Ui {
    pub backend: Rc<dyn Backend>,
    pub config: Config,
    /// Set with [`Ui::set_tracks`], which invalidates the derived lists.
    tracks: Vec<Track>,
    tracks_version: u64,
    derived: std::cell::RefCell<Derived>,
    pub queue: Vec<QueueItem>,

    // playback, as reported by the audio engine
    pub track: Option<Track>,
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    pub volume: f64,
    /// When `position` was last reported, so the string can predict between reports.
    pub position_at: Instant,
    /// The engine request (`Player::play` or `stop`) the window is showing.
    /// Reports from any other are stale: the fade-out of the previous track,
    /// say, which must not pair its timing with the new track's title.
    pub generation: u64,
    /// Tracks that failed to open in a row; see [`Ui::open_failed`].
    pub open_failures: u32,

    pub stats: StatsMap,
    pub loved: Vec<String>,
    pub album_flags: AlbumFlags,
    pub streamer_source: Option<StreamerSource>,
    pub scanning: bool,
    pub scan_progress: Option<ScanProgress>,
    pub overlay_url: Option<String>,
    pub notice: Option<String>,
    pub version: String,
    pub commit: String,

    // local UI state
    pub filter: String,
    pub selected: usize,
    pub view: LibraryView,
    pub open_album: Option<String>,
    pub library_open: bool,
    pub queue_open: bool,
    pub settings_open: bool,
    pub shortcuts_open: bool,
    pub menu_open: bool,
    pub sleep_open: bool,
    pub sleep_ends_at: Option<Instant>,
    /// Which settings select has its list open.
    pub open_select: Option<&'static str>,
    /// Result of the last device switch, shown under the picker.
    pub device_error: Option<String>,
    /// Result of the last OBS password save.
    pub obs_message: Option<String>,
    pub copied: bool,
    /// Filled from the backend when Settings opens, as the Electron panel did.
    pub outputs: Vec<(String, String)>,
    pub paths: Option<bard_core::types::AppPaths>,

    pub covers: Covers,
    /// The clock `position_at` is measured against, for the string's prediction.
    pub epoch: Instant,
    pub string: crate::views::string_bar::StringState,
}

impl Ui {
    pub fn new(backend: Rc<dyn Backend>, artwork_dir: PathBuf) -> Self {
        Self {
            backend,
            config: Config::default(),
            tracks: Vec::new(),
            tracks_version: 0,
            derived: Default::default(),
            queue: Vec::new(),
            track: None,
            position: 0.0,
            duration: 0.0,
            playing: false,
            volume: 0.8,
            position_at: Instant::now(),
            generation: 0,
            open_failures: 0,
            stats: StatsMap::default(),
            loved: Vec::new(),
            album_flags: AlbumFlags::default(),
            streamer_source: None,
            scanning: false,
            scan_progress: None,
            overlay_url: None,
            notice: None,
            version: env!("CARGO_PKG_VERSION").to_string(),
            commit: String::from("unknown"),
            filter: String::new(),
            selected: 0,
            view: LibraryView::Albums,
            open_album: None,
            library_open: true,
            queue_open: true,
            settings_open: false,
            shortcuts_open: false,
            menu_open: false,
            sleep_open: false,
            sleep_ends_at: None,
            open_select: None,
            device_error: None,
            obs_message: None,
            copied: false,
            outputs: Vec::new(),
            paths: None,
            covers: Covers::new(artwork_dir),
            epoch: Instant::now(),
            string: Default::default(),
        }
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn set_tracks(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
        self.tracks_version += 1;
    }

    /// The rows the library list shows, after view, album and filter.
    pub fn filtered_tracks(&self) -> Rc<Vec<Track>> {
        let key = (
            self.tracks_version,
            self.filter.clone(),
            self.open_album.clone(),
            self.view,
            if self.view == LibraryView::Loved { self.loved.clone() } else { Vec::new() },
        );
        if let Some((k, v)) = &self.derived.borrow().tracks
            && *k == key
        {
            return v.clone();
        }
        let v = Rc::new(self.compute_filtered_tracks());
        self.derived.borrow_mut().tracks = Some((key, v.clone()));
        v
    }

    fn compute_filtered_tracks(&self) -> Vec<Track> {
        let q = self.filter.trim().to_lowercase();
        self.tracks
            .iter()
            .filter(|t| match (&self.open_album, self.view) {
                (Some(a), _) => &t.album == a,
                (None, LibraryView::Loved) => self.loved.contains(&t.id),
                _ => true,
            })
            .filter(|t| q.is_empty() || t.search_text.contains(&q))
            .cloned()
            .collect()
    }

    /// Albums in the whole library, filter or not.
    pub fn album_count(&self) -> usize {
        if let Some((v, n)) = self.derived.borrow().album_count
            && v == self.tracks_version
        {
            return n;
        }
        let n = self.tracks.iter().map(|t| t.album.as_str()).collect::<HashSet<_>>().len();
        self.derived.borrow_mut().album_count = Some((self.tracks_version, n));
        n
    }

    pub fn filtered_albums(&self) -> Rc<Vec<Album>> {
        let key = (self.tracks_version, self.filter.clone());
        if let Some((k, v)) = &self.derived.borrow().albums
            && *k == key
        {
            return v.clone();
        }
        let v = Rc::new(self.compute_filtered_albums());
        self.derived.borrow_mut().albums = Some((key, v.clone()));
        v
    }

    fn compute_filtered_albums(&self) -> Vec<Album> {
        let q = self.filter.trim().to_lowercase();
        group_albums(&self.tracks)
            .into_iter()
            .filter(|a| {
                q.is_empty()
                    || a.name.to_lowercase().contains(&q)
                    || a.artist.to_lowercase().contains(&q)
                    || a.tracks.iter().any(|t| t.title.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub fn album_status(&self, album: &str) -> ContentIdStatus {
        self.album_flags.get(album).map(|f| f.status).unwrap_or_default()
    }

    pub fn is_loved(&self, id: &str) -> bool {
        self.loved.iter().any(|l| l == id)
    }

    pub fn armed(&self) -> bool {
        self.config.streamer.enabled
    }

    /// Whether an engine report belongs to the request on screen.
    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// The current track could not be opened. Shows why and returns whether
    /// to give up: after [`MAX_OPEN_FAILURES`] in a row playback stops rather
    /// than run through a queue that cannot play (a library on a drive that is
    /// not mounted), and the count starts over.
    pub fn open_failed(&mut self, message: &str) -> bool {
        self.playing = false;
        self.open_failures += 1;
        let title = self.track.as_ref().map(|t| t.title.as_str()).unwrap_or("track");
        let give_up = self.open_failures >= MAX_OPEN_FAILURES;
        self.notice = Some(if give_up {
            format!(
                "Could not play \u{201c}{title}\u{201d}: {message} \u{2014} stopped after {MAX_OPEN_FAILURES} tracks in a row failed"
            )
        } else {
            format!("Could not play \u{201c}{title}\u{201d}: {message} \u{2014} skipping")
        });
        if give_up {
            self.open_failures = 0;
        }
        give_up
    }
}

/// See [`Ui::open_failed`].
pub const MAX_OPEN_FAILURES: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        Ui::new(Rc::new(NullBackend), PathBuf::new())
    }

    #[test]
    fn only_reports_for_the_current_request_are_current() {
        let mut u = ui();
        u.generation = 7;
        assert!(u.is_current(7));
        assert!(!u.is_current(6), "the outgoing track's fade-out");
        assert!(!u.is_current(8));
    }

    fn img() -> Option<Arc<RenderImage>> {
        Some(crate::backdrop::render_image(image::RgbaImage::new(1, 1)))
    }

    #[test]
    fn a_cover_miss_is_queued_once_and_drawn_once_decoded() {
        let mut c = Covers::new(PathBuf::from("/nonexistent"));
        assert!(c.thumb("a").is_none());
        assert!(c.thumb("a").is_none(), "asked again before it arrives");
        assert_eq!(c.take_wanted(), [(CoverKind::Thumb, "a".to_string())], "queued once");
        assert!(c.take_wanted().is_empty());
        c.insert((CoverKind::Thumb, "a".into()), img());
        assert!(c.thumb("a").is_some());
        assert!(c.cover("a").is_none(), "each kind is its own entry");
        assert_eq!(c.take_wanted(), [(CoverKind::Full, "a".to_string())]);
        // An unreadable cover is remembered, not asked for every frame.
        c.insert((CoverKind::Full, "a".into()), None);
        assert!(c.cover("a").is_none());
        assert!(c.take_wanted().is_empty());
    }

    #[test]
    fn the_least_recently_used_cover_is_evicted_and_handed_back() {
        let mut c = Covers::new(PathBuf::from("/nonexistent"));
        let cap = CoverKind::Full.capacity();
        for i in 0..cap {
            c.insert((CoverKind::Full, i.to_string()), img());
        }
        assert!(c.cover("0").is_some(), "touching 0 makes 1 the oldest");
        c.insert((CoverKind::Full, "new".into()), img());
        assert_eq!(c.take_evicted().len(), 1);
        assert!(c.cover("0").is_some());
        assert!(c.cover("1").is_none(), "1 was evicted");
        assert_eq!(c.take_wanted(), [(CoverKind::Full, "1".to_string())], "and is decoded again if needed");
    }

    #[test]
    fn requests_a_closed_window_never_answered_are_made_again() {
        let mut c = Covers::new(PathBuf::from("/nonexistent"));
        c.thumb("a");
        c.take_wanted();
        c.forget_requests();
        c.thumb("a");
        assert_eq!(c.take_wanted().len(), 1);
    }

    #[test]
    fn derived_lists_follow_the_tracks_and_the_filter() {
        let mut u = ui();
        let t = |id: &str, album: &str| Track {
            id: id.into(),
            path: format!("/m/{id}.flac"),
            title: id.into(),
            artist: String::new(),
            album: album.into(),
            duration_sec: 60.,
            has_artwork: false,
            search_text: id.into(),
            track_gain_db: None,
            album_gain_db: None,
            track_peak: None,
            album_peak: None,
        };
        u.set_tracks(vec![t("a1", "A"), t("b1", "B")]);
        let first = u.filtered_albums();
        assert!(Rc::ptr_eq(&first, &u.filtered_albums()), "kept between frames");
        assert_eq!(u.album_count(), 2);
        u.filter = "b1".into();
        assert_eq!(u.filtered_albums().len(), 1);
        assert_eq!(u.filtered_tracks().len(), 1);
        u.filter.clear();
        u.set_tracks(vec![t("a1", "A")]);
        assert_eq!(u.filtered_albums().len(), 1, "new tracks, new list");
        assert_eq!(u.album_count(), 1);
    }

    #[test]
    fn open_failures_skip_then_give_up_and_start_over() {
        let mut u = ui();
        u.playing = true;
        assert!(!u.open_failed("no such file"));
        assert!(!u.playing);
        assert!(u.notice.as_deref().unwrap().ends_with("skipping"));
        assert!(!u.open_failed("no such file"));
        assert!(u.open_failed("no such file"), "the third in a row gives up");
        assert!(u.notice.as_deref().unwrap().contains("stopped after 3"));
        assert_eq!(u.open_failures, 0);
        assert!(!u.open_failed("no such file"), "and counting starts over");
    }
}
