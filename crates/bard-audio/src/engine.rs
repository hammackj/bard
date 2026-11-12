//! The engine thread: owns the decoder, runs the transport state machine, and
//! keeps the audio thread's chunk queue topped up.
//!
//! It is the Rust counterpart of engine.ts plus the HTMLAudioElement it
//! drove: play / pause / seek, the crossfade sequence, and the element's
//! `play` / `pause` / `timeupdate` / `ended` / `error` events, which become
//! [`PlayerEvent`]s.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::PlayerEvent;
use crate::decode::{CHANNELS, OUT_RATE, Source};
use crate::gain::{GainCmd, clamp_norm, clamp_volume, combined_gain};
use crate::render::{CHUNK_FRAMES, Chunk, GainMsg, Shared};

/// Progress cadence while playing: the rate of the element's `timeupdate`,
/// and what the overlay and nowplaying.json want. The visualiser reads the
/// analyser directly rather than riding this.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// How long a paused stream stays live before it is deactivated, so the
/// declick can finish and the sink can go idle.
const DEACTIVATE_AFTER: Duration = Duration::from_millis(200);

/// Poll interval while the chunk queue is full.
const IDLE_POLL: Duration = Duration::from_millis(5);

pub(crate) enum Cmd {
    /// `start: false` loads it paused at its beginning (no fade).
    Play {
        track_id: String,
        path: PathBuf,
        norm_gain: f32,
        crossfade_sec: f32,
        generation: u64,
        start: bool,
    },
    Pause,
    Resume,
    Seek(f64),
    Volume(f32),
    Norm(f32),
    Stop(u64),
    /// Ends a fade-out now: the incoming track starts at once.
    CutFade,
    SetTarget(Option<String>),
    Quit,
}

/// Where rendered audio goes: PipeWire, or nowhere (tests).
pub(crate) trait Output: Send {
    fn set_active(&mut self, active: bool);
    /// `None` is the default sink.
    fn set_target(&mut self, target: Option<String>);
}

struct PendingPlay {
    track_id: String,
    path: PathBuf,
    norm_gain: f32,
    crossfade_sec: f32,
    generation: u64,
    /// Where it starts: a seek made while it waits on the fade-out.
    start_at: f64,
}

pub(crate) struct Engine {
    shared: Arc<Shared>,
    events: Sender<PlayerEvent>,
    chunks: rtrb::Producer<Chunk>,
    gains: rtrb::Producer<GainMsg>,
    output: Box<dyn Output>,

    source: Option<Source>,
    /// The track whose audio is loaded. It lags the track most recently
    /// asked for by the length of a crossfade's fade-out, which is the point:
    /// it lets the caller tell "this is my current track" from "this is the
    /// tail of the one before".
    track_id: Option<String>,
    /// The request that put the engine in its current state; every report
    /// carries it.
    generation: u64,
    eof_queued: bool,
    ended_reported: bool,

    volume: f32,
    norm_gain: f32,

    /// A track waiting for the fade-out of the current one to finish.
    pending: Option<(PendingPlay, Instant)>,

    active: bool,
    deactivate_at: Option<Instant>,
    next_progress: Instant,
}

impl Engine {
    pub fn new(
        shared: Arc<Shared>,
        events: Sender<PlayerEvent>,
        chunks: rtrb::Producer<Chunk>,
        gains: rtrb::Producer<GainMsg>,
        output: Box<dyn Output>,
    ) -> Engine {
        Engine {
            shared,
            events,
            chunks,
            gains,
            output,
            source: None,
            track_id: None,
            generation: 0,
            eof_queued: true,
            ended_reported: true,
            volume: 1.0,
            norm_gain: 1.0,
            pending: None,
            active: false,
            deactivate_at: None,
            next_progress: Instant::now(),
        }
    }

    pub fn run(mut self, rx: Receiver<Cmd>) {
        self.gain_now(GainCmd::SetLevel(combined_gain(self.volume, self.norm_gain)));
        self.gain_now(GainCmd::SetEnvelope(1.0));
        loop {
            let timeout = self.timeout();
            match rx.recv_timeout(timeout) {
                Ok(cmd) => {
                    if !self.handle(cmd) {
                        break;
                    }
                    // Take a burst of commands (a slider drag) in one go.
                    while let Ok(cmd) = rx.try_recv() {
                        if !self.handle(cmd) {
                            return self.shutdown();
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
            self.decode_some();
        }
        self.shutdown();
    }

    fn shutdown(mut self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        self.output.set_active(false);
    }

    fn timeout(&self) -> Duration {
        let mut t = if self.wants_decode() && self.chunks.slots() > 0 { Duration::ZERO } else { IDLE_POLL };
        let now = Instant::now();
        if let Some((_, at)) = &self.pending {
            t = t.min(at.saturating_duration_since(now));
        }
        if self.shared.playing.load(Ordering::Relaxed) {
            t = t.min(self.next_progress.saturating_duration_since(now));
        }
        t
    }

    fn emit(&self, e: PlayerEvent) {
        let _ = self.events.send(e);
    }

    fn progress(&self) {
        self.emit(PlayerEvent::Progress {
            position: self.shared.position(),
            duration: self.shared.duration(),
            playing: self.shared.playing.load(Ordering::Relaxed),
            track_id: self.track_id.clone(),
            generation: self.generation,
        });
    }

    fn gain_now(&mut self, cmd: GainCmd) {
        let _ = self.gains.push(GainMsg { after_epoch: 0, cmd });
    }

    fn target_gain(&self) -> f32 {
        combined_gain(self.volume, self.norm_gain)
    }

    fn bump_epoch(&self) -> u32 {
        self.shared.epoch.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn activate(&mut self) {
        self.deactivate_at = None;
        if !self.active {
            self.active = true;
            self.output.set_active(true);
        }
    }

    fn deactivate_soon(&mut self) {
        self.deactivate_at = Some(Instant::now() + DEACTIVATE_AFTER);
    }

    fn is_audible(&self) -> bool {
        self.source.is_some() && !self.ended_reported && self.shared.playing.load(Ordering::Relaxed)
    }

    /// Returns false to stop the thread.
    fn handle(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Play { track_id, path, norm_gain, crossfade_sec, generation, start } => {
                let crossfade_sec = if crossfade_sec.is_finite() { crossfade_sec.max(0.0) } else { 0.0 };
                let play = PendingPlay {
                    track_id,
                    path,
                    norm_gain: clamp_norm(norm_gain),
                    crossfade_sec,
                    generation,
                    start_at: 0.0,
                };
                if !start {
                    // Replaces what is playing and any fade, as a cut does.
                    self.pending = None;
                    self.swap(play, false);
                    return true;
                }
                if crossfade_sec > 0.0 {
                    if let Some((p, _)) = &mut self.pending {
                        // Already fading out: the newest request replaces the
                        // queued one and inherits the fade in progress.
                        *p = play;
                        return true;
                    }
                    if self.is_audible() {
                        // A true crossfade needs two decoders playing at once,
                        // which this engine deliberately does not have -- one
                        // decoder, one stream. This is a fade out, swap, fade
                        // in: it removes the hard edge between tracks, which is
                        // what the seam actually sounds like, without a second
                        // audio path to keep in sync.
                        self.gain_now(GainCmd::Fade { to: 0.0, secs: crossfade_sec });
                        let at = Instant::now() + Duration::from_secs_f32(crossfade_sec);
                        self.pending = Some((play, at));
                        return true;
                    }
                }
                self.pending = None;
                self.swap(play, true);
            }
            Cmd::Pause => {
                // Paused mid-fade: the caller has already moved on to the
                // incoming track, so it is loaded now, paused, rather than
                // left queued -- the fade's deadline would otherwise start
                // it with no one having asked to play. Resume starts it.
                if let Some((p, _)) = self.pending.take() {
                    self.swap(p, false);
                    return true;
                }
                self.shared.playing.store(false, Ordering::Relaxed);
                if self.source.is_some() {
                    self.progress();
                }
                self.deactivate_soon();
            }
            Cmd::Resume => self.resume(),
            Cmd::Seek(secs) => match &mut self.pending {
                // The caller has moved on to the incoming track, so a seek is
                // about that one. It starts there when the fade-out ends; the
                // outgoing track fades out undisturbed.
                Some((p, _)) => {
                    if secs.is_finite() {
                        p.start_at = secs.max(0.0);
                    }
                }
                None => self.seek(secs),
            },
            Cmd::Volume(v) => {
                self.volume = clamp_volume(v);
                // Ramped rather than set, so volume changes don't click.
                self.gain_now(GainCmd::Level(self.target_gain()));
            }
            Cmd::Norm(m) => {
                self.norm_gain = clamp_norm(m);
                self.gain_now(GainCmd::Level(self.target_gain()));
            }
            Cmd::Stop(generation) => self.stop(generation),
            Cmd::CutFade => {
                if let Some((mut p, _)) = self.pending.take() {
                    // Straight in at its level: a fade-in would still be a
                    // fade, and the point is that the outgoing track is gone.
                    p.crossfade_sec = 0.0;
                    self.swap(p, true);
                }
            }
            Cmd::SetTarget(t) => self.output.set_target(t),
            Cmd::Quit => return false,
        }
        true
    }

    /// Loads a track, and starts it if `start`; otherwise it waits, paused,
    /// at its beginning, for a resume.
    fn swap(&mut self, mut p: PendingPlay, start: bool) {
        if !start {
            // There is nothing to fade in from a pause; resume plays it at
            // its level.
            p.crossfade_sec = 0.0;
        }
        self.norm_gain = p.norm_gain;
        let target = self.target_gain();
        // Set the level before the audio starts, or the first moments of a
        // loud track play at the previous track's gain.
        //
        // When crossfading, the envelope starts closed. The Electron engine once
        // used fadeIn(0) here, which snapped the gain straight to full --
        // undoing the fade-out that had just finished -- and the incoming
        // track began at full volume in the gap before the fade-in was
        // scheduled: the hard edge the crossfade exists to remove. Here the
        // fade-in is also held until the new track's first chunk is played.
        self.gain_now(GainCmd::SetLevel(target));
        self.gain_now(GainCmd::SetEnvelope(if p.crossfade_sec > 0.0 { 0.0 } else { 1.0 }));
        let epoch = self.bump_epoch();
        self.generation = p.generation;
        self.track_id = Some(p.track_id.clone());
        self.source = None;
        self.eof_queued = true;
        self.ended_reported = true;
        self.shared.set_position(0.0);

        match Source::open(&p.path) {
            Ok(mut src) => {
                self.shared.set_duration(src.duration());
                if p.start_at > 0.0 {
                    let dur = src.duration();
                    let at = if dur > 0.0 { p.start_at.min(dur) } else { p.start_at };
                    match src.seek(at) {
                        Ok(()) => self.shared.set_position(at),
                        Err(e) => self.emit(PlayerEvent::Error(format!("{e:#}"))),
                    }
                }
                self.source = Some(src);
                self.eof_queued = false;
                self.ended_reported = false;
                self.shared.loaded.store(true, Ordering::Relaxed);
                self.shared.playing.store(start, Ordering::Relaxed);
                if !start {
                    self.decode_some();
                    self.deactivate_soon();
                    self.progress();
                    return;
                }
                if p.crossfade_sec > 0.0 {
                    let _ = self
                        .gains
                        .push(GainMsg { after_epoch: epoch, cmd: GainCmd::Fade { to: 1.0, secs: p.crossfade_sec } });
                }
                self.decode_some();
                self.activate();
                self.next_progress = Instant::now() + PROGRESS_EVERY;
                self.progress();
            }
            Err(e) => {
                self.shared.set_duration(0.0);
                self.shared.loaded.store(false, Ordering::Relaxed);
                self.shared.playing.store(false, Ordering::Relaxed);
                self.deactivate_soon();
                self.progress();
                self.emit(PlayerEvent::Failed {
                    generation: p.generation,
                    track_id: p.track_id,
                    message: format!("{e:#}"),
                });
            }
        }
    }

    fn resume(&mut self) {
        if self.source.is_none() {
            self.shared.playing.store(false, Ordering::Relaxed);
            return;
        }
        // Like a media element, play() after the end starts over.
        if self.ended_reported {
            self.seek(0.0);
        }
        self.shared.playing.store(true, Ordering::Relaxed);
        self.activate();
        self.next_progress = Instant::now() + PROGRESS_EVERY;
        self.progress();
    }

    fn seek(&mut self, secs: f64) {
        if self.source.is_none() || !secs.is_finite() {
            return;
        }
        self.bump_epoch();
        let src = self.source.as_mut().expect("checked above");
        let dur = src.duration();
        let secs = if dur > 0.0 { secs.clamp(0.0, dur) } else { secs.max(0.0) };
        let res = src.seek(secs);
        self.shared.set_position(secs);
        self.eof_queued = false;
        self.ended_reported = false;
        if let Err(e) = res {
            self.emit(PlayerEvent::Error(format!("{e:#}")));
        }
        self.decode_some();
        self.progress();
    }

    fn stop(&mut self, generation: u64) {
        self.generation = generation;
        self.pending = None;
        self.bump_epoch();
        self.source = None;
        self.track_id = None;
        self.eof_queued = true;
        self.ended_reported = true;
        self.shared.loaded.store(false, Ordering::Relaxed);
        self.shared.playing.store(false, Ordering::Relaxed);
        self.shared.set_position(0.0);
        self.shared.set_duration(0.0);
        self.gain_now(GainCmd::SetLevel(self.target_gain()));
        self.gain_now(GainCmd::SetEnvelope(1.0));
        self.deactivate_soon();
        self.progress();
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        let ran_out = self.source.is_some()
            && !self.ended_reported
            && self.eof_queued
            && self.shared.ended_epoch.load(Ordering::Acquire) == epoch;

        if let Some((_, at)) = &self.pending
            && (now >= *at || ran_out)
        {
            // The outgoing track ran out before its fade-out finished: the
            // next track is already chosen, so start it now. Reporting Ended
            // here as well would make the app advance the queue a second time.
            let (p, _) = self.pending.take().expect("checked above");
            self.swap(p, true);
        } else if ran_out {
            // The element's order: a final timeupdate, pause, then ended.
            self.ended_reported = true;
            self.shared.playing.store(false, Ordering::Relaxed);
            self.progress();
            self.emit(PlayerEvent::Ended { generation: self.generation });
            self.deactivate_soon();
        }

        if self.shared.playing.load(Ordering::Relaxed) {
            if now >= self.next_progress {
                self.progress();
                self.next_progress = now + PROGRESS_EVERY;
            }
        } else if let Some(at) = self.deactivate_at
            && now >= at
        {
            self.deactivate_at = None;
            if self.active {
                self.active = false;
                self.output.set_active(false);
            }
        }
    }

    fn wants_decode(&self) -> bool {
        self.source.is_some() && !self.eof_queued
    }

    /// Decodes until the chunk queue is full (a few chunks per call, so
    /// commands are never kept waiting behind a long decode).
    fn decode_some(&mut self) {
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        for _ in 0..4 {
            if !self.wants_decode() || self.chunks.slots() == 0 {
                return;
            }
            let src = self.source.as_mut().expect("wants_decode");
            let mut chunk =
                Chunk { epoch, pos: src.position(), frames: 0, eof: false, data: [0.0; CHUNK_FRAMES * CHANNELS] };
            match src.fill(&mut chunk.data) {
                Ok(n) => {
                    chunk.frames = n as u32;
                    chunk.eof = n < CHUNK_FRAMES;
                }
                Err(e) => {
                    // Play what decoded and end there, rather than hang on a
                    // damaged file with no way forward.
                    chunk.eof = true;
                    self.emit(PlayerEvent::Error(format!("{e:#}")));
                }
            }
            self.eof_queued = chunk.eof;
            let _ = self.chunks.push(chunk);
        }
    }
}

/// An output that consumes audio in real time and sends it nowhere: the
/// engine runs exactly as it would against a device. For tests and for
/// machines without PipeWire.
pub(crate) struct NullOutput {
    active: Arc<std::sync::atomic::AtomicBool>,
    quit: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl NullOutput {
    pub fn spawn(mut renderer: crate::render::Renderer) -> std::io::Result<NullOutput> {
        use std::sync::atomic::AtomicBool;
        let active = Arc::new(AtomicBool::new(false));
        let quit = Arc::new(AtomicBool::new(false));
        let (a, q) = (active.clone(), quit.clone());
        let thread = std::thread::Builder::new().name("bard-audio-null".into()).spawn(move || {
            const PERIOD: usize = 480;
            let mut buf = vec![0.0f32; PERIOD * CHANNELS];
            let tick = Duration::from_secs_f64(PERIOD as f64 / OUT_RATE as f64);
            let mut next = Instant::now();
            while !q.load(Ordering::Relaxed) {
                if a.load(Ordering::Relaxed) {
                    renderer.render(&mut buf);
                }
                next += tick;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
            }
        })?;
        Ok(NullOutput { active, quit, thread: Some(thread) })
    }
}

impl Output for NullOutput {
    fn set_active(&mut self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
    }
    fn set_target(&mut self, _target: Option<String>) {}
}

impl Drop for NullOutput {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
