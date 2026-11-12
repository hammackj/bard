//! Types shared across the core, the audio engine, the services and the UI.
//!
//! A direct port of `src/shared/types.ts`. Everything that is persisted or
//! served (config.json, stats.json, albums.json, loved.json, nowplaying.json,
//! the overlay's SSE payloads) serialises to exactly the shape the Electron
//! build wrote -- camelCase keys, lowercase enum strings -- so an existing
//! `~/.config/bard` is read as-is and the OBS overlay page needs no changes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// First 16 hex chars of sha1(absolute path). Keys stats and the artwork cache.
    pub id: String,
    pub path: String,
    /// Tag title, else the filename without extension.
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_sec: f64,
    pub has_artwork: bool,
    /// Lowercased "title artist album" -- what search indexes.
    pub search_text: String,
    /// ReplayGain, in dB, when the file carries the tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_gain_db: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_gain_db: Option<f64>,
    /// Linear sample peak, used to hold back gain that would clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_peak: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_peak: Option<f64>,
}

/// Result of checking an album against YouTube Content ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContentIdStatus {
    #[default]
    Untested,
    Safe,
    Claimed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumFlag {
    pub status: ContentIdStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackStats {
    pub plays: u32,
    pub skips: u32,
    /// Epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_skipped_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    /// Identifies this queue entry, not the track -- the same track may be queued twice.
    pub id: String,
    pub track: Track,
    /// Epoch milliseconds.
    pub added_at: i64,
}

/// What the overlay and nowplaying.json carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_sec: f64,
    pub position_sec: f64,
    pub is_playing: bool,
    pub art_url: Option<String>,
    pub queue_length: usize,
    /// ISO-8601, e.g. 2026-11-12T03:12:50.260Z
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackProgress {
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    /// The track the audio engine is actually playing, which during a
    /// crossfade is still the outgoing one. None before anything has loaded.
    pub track_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanProgress {
    pub scanned: usize,
    pub total: usize,
    pub current_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NormalizeMode {
    Off,
    #[default]
    Track,
    Album,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioConfig {
    pub volume: f64,
    /// PipeWire node name in the Rust build. The Electron build stored a
    /// Chromium-hashed id here, which resolves to nothing and falls back to
    /// the default sink.
    pub output_device_id: String,
    pub normalize: NormalizeMode,
    pub crossfade_sec: f64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            volume: 0.8,
            output_device_id: "default".into(),
            normalize: NormalizeMode::Track,
            crossfade_sec: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlayConfig {
    pub port: u16,
    pub write_now_playing_json: bool,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self { port: 4747, write_now_playing_json: true }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShortcutConfig {
    pub global_media_keys: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AutoFillMode {
    Off,
    #[default]
    Shuffle,
    Album,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlaybackConfig {
    // Field order is serialisation order; it follows DEFAULT_CONFIG so a
    // rewritten config.json keeps the layout the Electron build gave it.
    pub auto_fill: AutoFillMode,
    pub repeat: RepeatMode,
    pub avoid_repeat: u32,
    pub fill_count: u32,
    pub max_queue_length: u32,
    pub endless: bool,
    pub endless_lookahead: u32,
    pub exclude_claimed: bool,
    pub loved_weight: u32,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            auto_fill: AutoFillMode::Shuffle,
            repeat: RepeatMode::Off,
            avoid_repeat: 50,
            fill_count: 10,
            max_queue_length: 200,
            endless: true,
            endless_lookahead: 10,
            exclude_claimed: true,
            loved_weight: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TrayPopupCorner {
    #[default]
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowConfig {
    pub close_to_tray: bool,
    pub tray_popup_corner: TrayPopupCorner,
    pub tray_popup_seconds: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            tray_popup_corner: TrayPopupCorner::BottomRight,
            tray_popup_seconds: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamerConfig {
    pub enabled: bool,
    pub auto_arm: bool,
    pub obs_url: String,
    pub has_obs_password: bool,
}

impl Default for StreamerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_arm: false,
            obs_url: "ws://127.0.0.1:4455".into(),
            has_obs_password: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub library_paths: Vec<String>,
    pub audio: AudioConfig,
    pub overlay: OverlayConfig,
    pub shortcuts: ShortcutConfig,
    pub window: WindowConfig,
    pub playback: PlaybackConfig,
    pub streamer: StreamerConfig,
}

/// Transport commands from outside the UI (tray, media keys, MPRIS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAction {
    Toggle,
    Restart,
}

/// Which OBS output armed streamer mode; None when armed by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamerSource {
    Stream,
    Record,
    Replay,
    Virtualcam,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppPaths {
    pub config: String,
    pub config_dir: String,
    pub artwork: String,
    pub now_playing: String,
    pub logs: String,
}

pub type StatsMap = HashMap<String, TrackStats>;
pub type AlbumFlags = HashMap<String, AlbumFlag>;
