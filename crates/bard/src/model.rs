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
use std::collections::HashMap;
use std::path::PathBuf;
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
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Decoded covers, keyed by track id. `None` records a miss so a track without
/// a readable cover is not retried every frame.
#[derive(Default)]
pub struct Covers {
    pub dir: PathBuf,
    cover: HashMap<String, Option<Arc<RenderImage>>>,
    ambient: HashMap<String, Option<Arc<RenderImage>>>,
}

impl Covers {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, ..Default::default() }
    }

    fn decode(&self, id: &str) -> Option<image::DynamicImage> {
        image::open(self.dir.join(format!("{id}.jpg"))).ok()
    }

    pub fn cover(&mut self, id: &str) -> Option<Arc<RenderImage>> {
        if let Some(hit) = self.cover.get(id) {
            return hit.clone();
        }
        let img = self.decode(id).map(|d| crate::backdrop::render_image(d.to_rgba8()));
        self.cover.insert(id.to_string(), img.clone());
        img
    }

    pub fn ambient(&mut self, id: &str) -> Option<Arc<RenderImage>> {
        if let Some(hit) = self.ambient.get(id) {
            return hit.clone();
        }
        let img = self.decode(id).map(|d| crate::backdrop::ambient(&d));
        self.ambient.insert(id.to_string(), img.clone());
        img
    }
}

pub struct Ui {
    pub backend: Rc<dyn Backend>,
    pub config: Config,
    pub tracks: Vec<Track>,
    pub queue: Vec<QueueItem>,

    // playback, as reported by the audio engine
    pub track: Option<Track>,
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    pub volume: f64,
    /// When `position` was last reported, so the string can predict between reports.
    pub position_at: Instant,

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
            queue: Vec::new(),
            track: None,
            position: 0.0,
            duration: 0.0,
            playing: false,
            volume: 0.8,
            position_at: Instant::now(),
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

    /// The rows the library list shows, after view, album and filter.
    pub fn filtered_tracks(&self) -> Vec<Track> {
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

    pub fn filtered_albums(&self) -> Vec<Album> {
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
}
