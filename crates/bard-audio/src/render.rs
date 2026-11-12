//! The audio-thread side: turns decoded chunks into output frames.
//!
//! Everything here runs inside PipeWire's process callback (or the null
//! output's clock thread) and must not block or allocate. It talks to the
//! engine thread only through lock-free queues and atomics:
//!
//! - chunks of decoded audio arrive on an SPSC ring, each stamped with the
//!   *epoch* it was decoded for. Play, seek and stop bump the shared
//!   epoch, which instantly orphans everything already queued -- no
//!   flush handshake needed;
//! - gain automation arrives on a second ring, optionally held back until the
//!   first chunk of a given epoch is reached, so a crossfade's fade-in
//!   starts with the incoming track rather than whenever the decoder happened
//!   to open it.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::OutputDevice;
use crate::analyser::AnalyserWriter;
use crate::decode::{CHANNELS, OUT_RATE};
use crate::gain::{GainCmd, GainRamp};

pub(crate) const CHUNK_FRAMES: usize = 1024;
/// ~340 ms of decoded audio queued ahead of the output.
pub(crate) const CHUNK_QUEUE: usize = 16;
pub(crate) const GAIN_QUEUE: usize = 64;

/// Pause, resume and track starts are faded over this long so they don't click.
const DECLICK_FRAMES: f32 = 240.0;

pub(crate) struct Chunk {
    pub epoch: u32,
    /// Media time of the first frame.
    pub pos: f64,
    pub frames: u32,
    /// The last chunk of the track: once played out, the track has ended.
    pub eof: bool,
    pub data: [f32; CHUNK_FRAMES * CHANNELS],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GainMsg {
    /// 0: apply now. Otherwise wait for the first chunk of this epoch.
    pub after_epoch: u32,
    pub cmd: GainCmd,
}

/// State shared by the handle, the engine thread and the audio thread.
pub(crate) struct Shared {
    /// Bumped on every load, seek and stop. 0 = nothing ever loaded.
    pub epoch: AtomicU32,
    /// The play/pause intent. The audio thread follows it, with a declick.
    pub playing: AtomicBool,
    /// Something is loaded (play() has opened a file and stop() hasn't run).
    pub loaded: AtomicBool,
    pub position: AtomicU64,
    pub duration: AtomicU64,
    /// Live gain incl. ramps (f32 bits).
    pub gain: AtomicU32,
    /// The epoch whose last chunk has been played out.
    pub ended_epoch: AtomicU32,
    /// Render as normal (analyser included) but write zeros to the device.
    pub silent: bool,
    pub outputs: Mutex<Vec<OutputDevice>>,
}

impl Shared {
    pub fn new(silent: bool) -> Shared {
        Shared {
            epoch: AtomicU32::new(0),
            playing: AtomicBool::new(false),
            loaded: AtomicBool::new(false),
            position: AtomicU64::new(0f64.to_bits()),
            duration: AtomicU64::new(0f64.to_bits()),
            gain: AtomicU32::new(0f32.to_bits()),
            ended_epoch: AtomicU32::new(0),
            silent,
            outputs: Mutex::new(Vec::new()),
        }
    }

    pub fn position(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Relaxed))
    }
    pub fn set_position(&self, v: f64) {
        self.position.store(v.to_bits(), Ordering::Relaxed)
    }
    pub fn duration(&self) -> f64 {
        f64::from_bits(self.duration.load(Ordering::Relaxed))
    }
    pub fn set_duration(&self, v: f64) {
        self.duration.store(v.to_bits(), Ordering::Relaxed)
    }
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
}

pub(crate) struct Renderer {
    shared: Arc<Shared>,
    chunks: rtrb::Consumer<Chunk>,
    gains: rtrb::Consumer<GainMsg>,
    analyser: AnalyserWriter,
    gain: GainRamp,
    declick: f32,
    /// The chunk being played, and how far into it. Boxed once up front so
    /// popping a chunk is a copy into it, not an allocation.
    cur: Box<Chunk>,
    cur_valid: bool,
    off: usize,
    /// Generation of the last chunk started, for gated gain messages.
    active_epoch: u32,
    /// Media time just past the last frame played, and its epoch.
    last_pos: f64,
    last_pos_epoch: u32,
}

impl Renderer {
    pub fn new(
        shared: Arc<Shared>,
        chunks: rtrb::Consumer<Chunk>,
        gains: rtrb::Consumer<GainMsg>,
        analyser: AnalyserWriter,
    ) -> Renderer {
        Renderer {
            shared,
            chunks,
            gains,
            analyser,
            gain: GainRamp::new(OUT_RATE, 1.0),
            declick: 0.0,
            cur: Box::new(Chunk { epoch: 0, pos: 0.0, frames: 0, eof: false, data: [0.0; CHUNK_FRAMES * CHANNELS] }),
            cur_valid: false,
            off: 0,
            active_epoch: 0,
            last_pos: 0.0,
            last_pos_epoch: 0,
        }
    }

    fn drain_gains(&mut self) {
        while let Ok(head) = self.gains.peek() {
            if head.after_epoch != 0 && head.after_epoch > self.active_epoch {
                break;
            }
            let msg = *head;
            let _ = self.gains.pop();
            self.gain.apply(msg.cmd);
        }
    }

    /// Next source frame for epoch `epoch`, or silence if none is ready.
    #[inline]
    fn next_frame(&mut self, epoch: u32) -> (f32, f32) {
        loop {
            if self.cur_valid && self.cur.epoch == epoch {
                if self.off < self.cur.frames as usize {
                    let i = self.off * CHANNELS;
                    self.off += 1;
                    return (self.cur.data[i], self.cur.data[i + 1]);
                }
                if self.cur.eof {
                    self.shared.ended_epoch.store(epoch, Ordering::Release);
                    return (0.0, 0.0);
                }
            }
            // `pop` hands the chunk over by value (an 8 KB copy); it lands in
            // a box allocated once up front, so nothing allocates here.
            match self.chunks.pop() {
                Ok(c) => {
                    if c.epoch != epoch {
                        continue;
                    }
                    *self.cur = c;
                    self.cur_valid = true;
                    self.off = 0;
                    if epoch != self.active_epoch {
                        // A new track or a seek: start it from silence so the
                        // cut is not a click, and release gain messages that
                        // were waiting for it.
                        self.active_epoch = epoch;
                        self.declick = 0.0;
                        self.drain_gains();
                    }
                    self.last_pos = self.cur.pos;
                    self.last_pos_epoch = epoch;
                }
                Err(_) => return (0.0, 0.0),
            }
        }
    }

    /// Fills `out` (interleaved stereo).
    pub fn render(&mut self, out: &mut [f32]) {
        self.drain_gains();
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        let playing = self.shared.playing.load(Ordering::Relaxed);
        let target = if playing { 1.0 } else { 0.0 };
        let step = 1.0 / DECLICK_FRAMES;
        let silent = self.shared.silent;

        for frame in out.chunks_exact_mut(CHANNELS) {
            if self.declick < target {
                self.declick = (self.declick + step).min(1.0);
            } else if self.declick > target {
                self.declick = (self.declick - step).max(0.0);
            }
            let (l, r) = if self.declick > 0.0 || playing { self.next_frame(epoch) } else { (0.0, 0.0) };
            let k = self.gain.next() * self.declick;
            let (l, r) = (l * k, r * k);
            self.analyser.push((l + r) * 0.5);
            if silent {
                frame[0] = 0.0;
                frame[1] = 0.0;
            } else {
                frame[0] = l;
                frame[1] = r;
            }
        }
        self.analyser.commit();
        self.shared.gain.store(self.gain.value().to_bits(), Ordering::Relaxed);

        if self.cur_valid && self.cur.epoch == epoch {
            self.last_pos = self.cur.pos + self.off as f64 / OUT_RATE as f64;
        }
        // Re-checked: a seek that landed mid-callback has already set the
        // position it wants, and a stale one must not overwrite it.
        if self.last_pos_epoch == epoch && self.shared.epoch.load(Ordering::Acquire) == epoch {
            self.shared.set_position(self.last_pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyser::{FFT_SIZE, analyser};

    struct Rig {
        shared: Arc<Shared>,
        chunks: rtrb::Producer<Chunk>,
        gains: rtrb::Producer<GainMsg>,
        r: Renderer,
    }

    fn rig() -> Rig {
        let shared = Arc::new(Shared::new(false));
        let (cp, cc) = rtrb::RingBuffer::new(CHUNK_QUEUE);
        let (gp, gc) = rtrb::RingBuffer::new(GAIN_QUEUE);
        let (_, w) = analyser();
        let r = Renderer::new(shared.clone(), cc, gc, w);
        Rig { shared, chunks: cp, gains: gp, r }
    }

    fn dc(epoch: u32, pos: f64, v: f32, eof: bool) -> Chunk {
        Chunk { epoch, pos, frames: CHUNK_FRAMES as u32, eof, data: [v; CHUNK_FRAMES * CHANNELS] }
    }

    #[test]
    fn plays_current_epoch_and_drops_stale() {
        let mut t = rig();
        t.shared.epoch.store(2, Ordering::Release);
        t.shared.playing.store(true, Ordering::Relaxed);
        t.chunks.push(dc(1, 0.0, 0.9, false)).ok().unwrap();
        t.chunks.push(dc(2, 10.0, 0.5, false)).ok().unwrap();
        let mut out = vec![0.0; 512 * 2];
        t.r.render(&mut out);
        // Declicked in, then the epoch-2 level only.
        assert!(out[0].abs() < 0.01);
        assert!((out[out.len() - 1] - 0.5).abs() < 1e-6);
        assert!(out.iter().all(|s| *s <= 0.5 + 1e-6));
        let p = t.shared.position();
        assert!((p - (10.0 + 512.0 / 48000.0)).abs() < 1e-9, "{p}");
    }

    #[test]
    fn eof_marks_ended_after_the_last_frame() {
        let mut t = rig();
        t.shared.epoch.store(1, Ordering::Release);
        t.shared.playing.store(true, Ordering::Relaxed);
        t.chunks.push(dc(1, 0.0, 0.1, true)).ok().unwrap();
        let mut out = vec![0.0; 1000 * 2];
        t.r.render(&mut out);
        assert_eq!(t.shared.ended_epoch.load(Ordering::Relaxed), 0);
        t.r.render(&mut out);
        assert_eq!(t.shared.ended_epoch.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn pause_declicks_and_holds_position() {
        let mut t = rig();
        t.shared.epoch.store(1, Ordering::Release);
        t.shared.playing.store(true, Ordering::Relaxed);
        for i in 0..4 {
            t.chunks.push(dc(1, i as f64 * CHUNK_FRAMES as f64 / 48000.0, 0.5, false)).ok().unwrap();
        }
        let mut out = vec![0.0; 480 * 2];
        t.r.render(&mut out);
        t.shared.playing.store(false, Ordering::Relaxed);
        t.r.render(&mut out);
        // Ramps down, no step bigger than the declick slope allows.
        assert!(out.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01));
        assert_eq!(out[out.len() - 1], 0.0);
        let held = t.shared.position();
        t.r.render(&mut out);
        assert_eq!(t.shared.position(), held, "paused: no frames consumed");
        assert!((held - (480.0 + 240.0) / 48000.0).abs() < 1e-3, "{held}");
    }

    #[test]
    fn gated_gain_waits_for_its_epoch() {
        let mut t = rig();
        t.shared.epoch.store(1, Ordering::Release);
        t.shared.playing.store(true, Ordering::Relaxed);
        t.gains.push(GainMsg { after_epoch: 0, cmd: GainCmd::Set(0.0) }).unwrap();
        t.gains.push(GainMsg { after_epoch: 1, cmd: GainCmd::Linear { to: 1.0, secs: 0.1, retarget: true } }).unwrap();
        let mut out = vec![0.0; 480 * 2];
        // No chunk yet: the fade-in must not start in the gap.
        t.r.render(&mut out);
        t.r.render(&mut out);
        assert_eq!(t.shared.gain(), 0.0);
        t.chunks.push(dc(1, 0.0, 0.5, false)).ok().unwrap();
        t.r.render(&mut out);
        let g = t.shared.gain();
        assert!((g - 0.1).abs() < 0.01, "fade-in starts with the track: {g}");
    }

    #[test]
    fn analyser_sees_post_gain_mono_and_silent_mode_writes_zeros() {
        let shared = Arc::new(Shared::new(true));
        let (cp, cc) = rtrb::RingBuffer::new(CHUNK_QUEUE);
        let (mut gp, gc) = rtrb::RingBuffer::new(GAIN_QUEUE);
        let (a, w) = analyser();
        let mut r = Renderer::new(shared.clone(), cc, gc, w);
        let mut cp = cp;
        shared.epoch.store(1, Ordering::Release);
        shared.playing.store(true, Ordering::Relaxed);
        gp.push(GainMsg { after_epoch: 0, cmd: GainCmd::Set(0.5) }).unwrap();
        let mut c = dc(1, 0.0, 0.0, false);
        for f in c.data.chunks_exact_mut(2) {
            f[0] = 0.8;
            f[1] = 0.4;
        }
        cp.push(c).ok().unwrap();
        let mut out = vec![1.0; 1024 * 2];
        r.render(&mut out);
        assert!(out.iter().all(|s| *s == 0.0), "silent mode emits zeros");
        let mut f = [0f32; FFT_SIZE];
        a.time_domain_f32(&mut f);
        // (0.8 + 0.4) / 2 * 0.5
        assert!((f[FFT_SIZE - 1] - 0.3).abs() < 1e-6, "{}", f[FFT_SIZE - 1]);
        let mut b = [0u8; FFT_SIZE];
        a.time_domain(&mut b);
        assert_eq!(b[FFT_SIZE - 1], (128.0f32 * 1.3).floor() as u8);
    }
}
