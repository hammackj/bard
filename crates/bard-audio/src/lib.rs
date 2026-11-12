//! Bard's audio engine: decode, PipeWire output, gain, fades, and the
//! analyser behind the visualiser.
//!
//! This replaces the Electron renderer's Web Audio graph
//! (`src/renderer/src/audio/engine.ts`):
//!
//! ```text
//!   Source (symphonia / libopus, rubato)  -> engine thread
//!     -> chunk queue -> gain -> analyser -> PipeWire stream  (audio thread)
//! ```
//!
//! [`Player`] is a cheap, `Send + Sync` handle. Every method returns at once:
//! commands go to the engine thread over a channel, and the reads
//! ([`Player::position`], [`Player::current_gain`], [`Analyser`]) are atomics.
//! Nothing here runs on, or waits for, the caller's (UI) thread.
//!
//! # Threads
//!
//! - `bard-audio`: the engine. Owns the decoder, the transport state and the
//!   crossfade sequence; emits [`PlayerEvent`]s.
//! - `bard-pipewire`: a PipeWire main loop for the stream and the sink list.
//!   The stream's process callback runs on PipeWire's realtime data thread and
//!   is lock-free.
//!
//! # Deviations from the requested API
//!
//! - [`Player::with_options`] / [`PlayerOptions`] are additions: a `silent`
//!   mode (everything runs, the analyser sees the audio, the device gets
//!   zeros -- the equivalent of the selftest's `setSinkId({type:'none'})`), a
//!   `Null` backend that needs no PipeWire, and the stream's node name.
//! - [`Player::set_output`] with an unknown id falls back to the default sink
//!   and reports it twice, deliberately: it returns `Err` (as engine.ts threw)
//!   *and* sends [`PlayerEvent::Error`], because the call is often made at
//!   startup from saved config, where only the event stream is watched.
//!   Show one of them.
//! - [`Player::outputs`] lists real sinks only; "default" is not in it, as the
//!   Electron settings panel added its own "System default" row.
//! - A crossfade that is waiting on its fade-out still starts the next track
//!   if paused in the meantime -- as engine.ts did, since its `play()` called
//!   `a.play()` after the fade regardless.
//! - If the outgoing track runs out before its crossfade fade-out finishes,
//!   the incoming track starts then and no `Ended` is sent for the outgoing
//!   one (engine.ts sent it, inviting a second queue advance).
//! - [`Player::resume`] after a track has ended starts it over, as
//!   `HTMLMediaElement.play()` does.
//! - A decode error part-way through a file sends `Error` and then ends the
//!   track (`Ended`), rather than leaving the player stuck.

mod analyser;
pub mod decode;
mod engine;
mod gain;
mod mp4;
mod opus;
mod pw;
mod render;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::{Context, Result, anyhow};

pub use analyser::{Analyser, FFT_SIZE};
pub use gain::combined_gain;

use engine::{Cmd, Engine, NullOutput, Output};
use render::{CHUNK_QUEUE, GAIN_QUEUE, Renderer, Shared};

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    /// ~4 Hz while playing, and on every play, pause, seek and stop.
    ///
    /// `track_id` is the track whose audio is loaded right now. It lags the
    /// track most recently passed to [`Player::play`] for the length of a
    /// crossfade's fade-out, which is the point: it lets the app tell "this
    /// is my current track" from "this is the tail of the one before".
    Progress {
        position: f64,
        duration: f64,
        playing: bool,
        track_id: Option<String>,
    },
    /// The loaded track played to its end. Preceded by a final
    /// `Progress { playing: false, .. }`.
    Ended,
    Error(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Position {
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
}

/// A PipeWire sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputDevice {
    /// `node.name`: stable across restarts and replugs; save this.
    pub id: String,
    /// `node.description`, for the picker.
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Backend {
    #[default]
    PipeWire,
    /// Renders in real time into nothing. No device, no sound, no PipeWire.
    Null,
}

#[derive(Clone, Debug)]
pub struct PlayerOptions {
    pub backend: Backend,
    /// Run the whole pipeline but write zeros to the device.
    pub silent: bool,
    /// The stream's `node.name`.
    pub node_name: String,
    /// `application.name` / `node.description`: what mixers show.
    pub app_name: String,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        PlayerOptions { backend: Backend::PipeWire, silent: false, node_name: "bard".into(), app_name: "Bard".into() }
    }
}

struct Inner {
    cmd: Sender<Cmd>,
    shared: Arc<Shared>,
    analyser: Analyser,
    events: Sender<PlayerEvent>,
    engine: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Quit);
        if let Some(t) = self.engine.take() {
            let _ = t.join();
        }
    }
}

/// The player. Clone freely; the engine stops when the last clone is dropped.
#[derive(Clone)]
pub struct Player {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player").field("position", &self.position()).finish()
    }
}

impl Player {
    /// A player on the default PipeWire sink.
    pub fn new() -> Result<(Player, Receiver<PlayerEvent>)> {
        Player::with_options(PlayerOptions::default())
    }

    pub fn with_options(opts: PlayerOptions) -> Result<(Player, Receiver<PlayerEvent>)> {
        let shared = Arc::new(Shared::new(opts.silent));
        let (chunk_tx, chunk_rx) = rtrb::RingBuffer::new(CHUNK_QUEUE);
        let (gain_tx, gain_rx) = rtrb::RingBuffer::new(GAIN_QUEUE);
        let (analyser, writer) = analyser::analyser();
        let renderer = Renderer::new(shared.clone(), chunk_rx, gain_rx, writer);
        let (ev_tx, ev_rx) = mpsc::channel();

        let output: Box<dyn Output> = match opts.backend {
            Backend::PipeWire => Box::new(pw::PwOutput::spawn(
                renderer,
                shared.clone(),
                ev_tx.clone(),
                pw::PwOptions { node_name: opts.node_name, app_name: opts.app_name },
            )?),
            Backend::Null => Box::new(NullOutput::spawn(renderer).context("spawning the null output")?),
        };

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let engine = Engine::new(shared.clone(), ev_tx.clone(), chunk_tx, gain_tx, output);
        let thread = std::thread::Builder::new()
            .name("bard-audio".into())
            .spawn(move || engine.run(cmd_rx))
            .context("spawning the audio engine thread")?;

        let inner = Inner { cmd: cmd_tx, shared, analyser, events: ev_tx, engine: Some(thread) };
        Ok((Player { inner: Arc::new(inner) }, ev_rx))
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.inner.cmd.send(cmd);
    }

    /// Loads and plays a track. With `crossfade_sec > 0` and something
    /// audible, the current track fades out over that long first, then the
    /// new one fades in from silence over the same time.
    ///
    /// `norm_gain` is the track's normalisation multiplier (ReplayGain, as a
    /// linear factor; 1.0 for none). It replaces the previous track's.
    pub fn play(&self, track_id: String, path: PathBuf, norm_gain: f32, crossfade_sec: f32) {
        self.send(Cmd::Play { track_id, path, norm_gain, crossfade_sec });
    }

    /// Pauses if playing, resumes if paused; returns the new state. Does
    /// nothing (and returns false) when nothing is loaded.
    pub fn toggle(&self) -> bool {
        if !self.inner.shared.loaded.load(Ordering::Relaxed) {
            return false;
        }
        let was = self.inner.shared.playing.load(Ordering::Relaxed);
        if was {
            self.pause()
        } else {
            self.resume()
        }
        !was
    }

    pub fn pause(&self) {
        self.inner.shared.playing.store(false, Ordering::Relaxed);
        self.send(Cmd::Pause);
    }

    pub fn resume(&self) {
        if self.inner.shared.loaded.load(Ordering::Relaxed) {
            self.inner.shared.playing.store(true, Ordering::Relaxed);
        }
        self.send(Cmd::Resume);
    }

    /// Seconds; clamped to the track.
    pub fn seek(&self, seconds: f64) {
        self.send(Cmd::Seek(seconds));
    }

    /// 0..1, ramped so it does not click. Kept separate from the
    /// normalisation gain; the two are multiplied (see [`combined_gain`]).
    pub fn set_volume(&self, v: f32) {
        self.send(Cmd::Volume(v));
    }

    /// The current track's normalisation multiplier, e.g. when the
    /// normalisation setting changes mid-track. Ramped like volume.
    pub fn set_norm_gain(&self, m: f32) {
        self.send(Cmd::Norm(m));
    }

    /// Unloads the track: the queue ran dry. Position and duration go to 0.
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }

    pub fn position(&self) -> Position {
        let s = &self.inner.shared;
        Position { position: s.position(), duration: s.duration(), playing: s.playing.load(Ordering::Relaxed) }
    }

    /// The visualiser's tap. Clone it into the UI; reading never blocks.
    pub fn analyser(&self) -> Analyser {
        self.inner.analyser.clone()
    }

    /// PipeWire sinks (`media.class = Audio/Sink`), virtual ones included,
    /// sorted by name. Kept current as devices come and go.
    pub fn outputs(&self) -> Vec<OutputDevice> {
        self.inner.shared.outputs.lock().map(|o| o.clone()).unwrap_or_default()
    }

    /// Routes playback to a sink by [`OutputDevice::id`], or `"default"`.
    /// Takes effect mid-track without losing the position.
    ///
    /// An id that is not a current sink -- the saved device is gone, an
    /// interface unplugged between runs, say -- falls back to the system
    /// default rather than leaving the app silent, and says so (see the
    /// crate docs for why both as `Err` and as an event).
    pub fn set_output(&self, id: &str) -> Result<()> {
        if id.is_empty() || id == "default" {
            self.send(Cmd::SetTarget(None));
            return Ok(());
        }
        if self.outputs().iter().any(|o| o.id == id) {
            self.send(Cmd::SetTarget(Some(id.to_string())));
            return Ok(());
        }
        self.send(Cmd::SetTarget(None));
        let msg = "saved output device unavailable, using system default".to_string();
        let _ = self.inner.events.send(PlayerEvent::Error(format!("output device: {msg}")));
        Err(anyhow!(msg))
    }

    /// The gain stage's live value, fades included.
    ///
    /// Distinct from what the level *should* be from volume and
    /// normalisation: this reads the stage itself, so it also sees fades in
    /// progress -- the only way to tell a crossfade actually happened.
    pub fn current_gain(&self) -> f32 {
        self.inner.shared.gain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn handle_is_send_sync() {
        assert_send_sync::<Player>();
        assert_send_sync::<Analyser>();
    }
}
