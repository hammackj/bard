//! Assembles the now-playing payload and keeps nowplaying.json current.
//! A port of `src/main/nowplaying.ts`.
//!
//! Progress arrives several times a second. The overlay wants all of it, but
//! the file does not -- writing on every tick is four disk writes a second for
//! an eight-hour stream. The file is written only when something a viewer
//! would notice changes: the track, or play/pause.
//!
//! Every place nowplaying.ts notified its listeners (which is when overlay.ts
//! pushed an SSE frame) emits [`CoreEvent::Overlay`] here instead.

use crate::CoreEvent;
use crate::types::{NowPlaying, PlaybackProgress, QueueItem};
use crate::util::{Indent, iso8601, now_ms, to_js_json, write_atomic};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

fn stopped() -> PlaybackProgress {
    PlaybackProgress { position: 0.0, duration: 0.0, playing: false, track_id: None }
}

pub(crate) struct NowPlayingState {
    path: PathBuf,
    events: Sender<CoreEvent>,
    current: Option<QueueItem>,
    progress: PlaybackProgress,
    queue_length: usize,
    /// The port the overlay server actually bound, or None when it is not
    /// running. Pushed in by whoever runs the server.
    overlay_port: Option<u16>,
    last_written: String,
}

impl NowPlayingState {
    pub fn new(path: PathBuf, events: Sender<CoreEvent>) -> Self {
        NowPlayingState {
            path,
            events,
            current: None,
            progress: stopped(),
            queue_length: 0,
            overlay_port: None,
            last_written: String::new(),
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn set_overlay_port(&mut self, port: Option<u16>) {
        if port == self.overlay_port {
            return;
        }
        self.overlay_port = port;
        self.emit();
    }

    pub fn snapshot(&self) -> NowPlaying {
        let track = self.current.as_ref().map(|c| &c.track);
        let duration = if self.progress.duration != 0.0 && !self.progress.duration.is_nan() {
            self.progress.duration
        } else {
            track.map(|t| t.duration_sec).filter(|d| *d != 0.0 && !d.is_nan()).unwrap_or(0.0)
        };
        NowPlaying {
            title: track.map(|t| t.title.clone()).unwrap_or_default(),
            artist: track.map(|t| t.artist.clone()).unwrap_or_default(),
            album: track.map(|t| t.album.clone()).unwrap_or_default(),
            duration_sec: duration,
            position_sec: self.progress.position,
            is_playing: self.progress.playing,
            // The bound port, not the configured one. When the configured port
            // is taken the server walks upward, and building this from config
            // pointed the overlay's cover art at a port some other process
            // holds -- so every track showed a blank cover for the whole
            // session, and nowplaying.json handed the same dead URL to anything
            // else reading it. None when nothing is bound: there is no server
            // to serve the art.
            art_url: match (track, self.overlay_port) {
                (Some(t), Some(port)) if t.has_artwork => {
                    Some(format!("http://127.0.0.1:{port}/art/{}", t.id))
                }
                _ => None,
            },
            queue_length: self.queue_length,
            updated_at: iso8601(now_ms()),
        }
    }

    fn emit(&self) {
        let _ = self.events.send(CoreEvent::Overlay(self.snapshot()));
    }

    /// Track changed, or playback stopped entirely.
    pub fn set_now_playing(&mut self, item: Option<QueueItem>, write_file: bool) {
        if item.is_none() {
            self.progress = stopped();
        }
        self.current = item;
        self.emit();
        self.write_if_interesting(true, write_file);
    }

    pub fn set_queue_length(&mut self, n: usize) {
        if n == self.queue_length {
            return;
        }
        self.queue_length = n;
        self.emit();
    }

    pub fn set_progress(&mut self, p: PlaybackProgress, write_file: bool) {
        let play_state_changed = p.playing != self.progress.playing;
        self.progress = p;
        self.emit();
        if play_state_changed {
            self.write_if_interesting(false, write_file);
        }
    }

    /// Writes the stopped state on the way out.
    ///
    /// Nothing reset this file on quit in early Electron builds, so it kept
    /// whatever was last written -- after a normal exit it sat at
    /// `"isPlaying": true` with the last track's title indefinitely. Anything
    /// reading it, an OBS text or browser source or another tool, went on
    /// showing a now-playing card for a track that stopped when the app did,
    /// with no running process left to correct it.
    pub fn write_stopped(&mut self, write_file: bool) {
        if !write_file {
            return;
        }
        self.current = None;
        self.progress = stopped();
        self.queue_length = 0;
        // A failed overlay write must never hold up quitting.
        let _ = self.write_now();
    }

    /// Writes atomically -- temp file then rename -- so OBS never reads a
    /// half-written file and shows a torn overlay.
    ///
    /// Synchronous: the Electron build chained async writes on a promise so two
    /// rapid changes could not interleave on the temp file; here the caller
    /// holds `&mut self`, which serialises them by construction.
    fn write_if_interesting(&mut self, force: bool, write_file: bool) {
        if !write_file {
            return;
        }
        let snap = self.snapshot();
        // positionSec/updatedAt always differ; key off what a viewer would notice.
        let key = format!("{}|{}|{}|{}", snap.title, snap.artist, snap.is_playing, snap.queue_length);
        if !force && key == self.last_written {
            return;
        }
        self.last_written = key;
        // A failed overlay file write must never break playback.
        let _ = self.write_now();
    }

    fn write_now(&self) -> std::io::Result<()> {
        let body = to_js_json(&self.snapshot(), Indent::Two).map_err(std::io::Error::other)?;
        write_atomic(&self.path, &body)
    }
}
