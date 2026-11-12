//! The visualiser's view of the signal: the equivalent of a Web Audio
//! `AnalyserNode` with `fftSize = 2048`, read with `getByteTimeDomainData`.
//!
//! The audio thread writes post-gain samples (what the user hears) into a
//! ring of atomics; the UI thread copies the most recent 2048 out. Neither
//! side ever waits for the other. A read that races a write can see a few
//! samples from the next block, which is invisible in a 60 fps squiggle and
//! far better than a lock on the audio thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Samples per read, as `AnalyserNode.fftSize`.
pub const FFT_SIZE: usize = 2048;

/// Twice the window, so a reader copying the newest 2048 samples is not
/// immediately overtaken by the writer.
const RING: usize = FFT_SIZE * 2;

struct Ring {
    samples: Box<[AtomicU32]>,
    /// Total samples ever written; the newest is at `(written - 1) % RING`.
    written: AtomicU64,
}

/// A cheap, clonable handle on the analyser. Reads never block.
#[derive(Clone)]
pub struct Analyser {
    ring: Arc<Ring>,
}

impl std::fmt::Debug for Analyser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Analyser").field("written", &self.ring.written.load(Ordering::Relaxed)).finish()
    }
}

/// The audio thread's end.
pub(crate) struct AnalyserWriter {
    ring: Arc<Ring>,
    pos: u64,
}

pub(crate) fn analyser() -> (Analyser, AnalyserWriter) {
    let samples = (0..RING).map(|_| AtomicU32::new(0f32.to_bits())).collect();
    let ring = Arc::new(Ring { samples, written: AtomicU64::new(0) });
    (Analyser { ring: ring.clone() }, AnalyserWriter { ring, pos: 0 })
}

impl AnalyserWriter {
    /// Stages one mono sample. Visible to readers after [`commit`](Self::commit).
    #[inline]
    pub fn push(&mut self, s: f32) {
        self.ring.samples[(self.pos % RING as u64) as usize].store(s.to_bits(), Ordering::Relaxed);
        self.pos += 1;
    }

    pub fn commit(&mut self) {
        self.ring.written.store(self.pos, Ordering::Release);
    }
}

impl Analyser {
    /// `AnalyserNode.fftSize`: how many samples a full read returns.
    pub fn fft_size(&self) -> usize {
        FFT_SIZE
    }

    /// `getByteTimeDomainData`: the newest [`FFT_SIZE`] samples, oldest
    /// first, as bytes where 128 is silence. Fills `min(out.len(), 2048)`
    /// bytes, as the Web API does for a short array.
    ///
    /// Downmixed to mono like an AnalyserNode, and taken after the gain, so
    /// it follows volume and fades exactly as the user hears them.
    pub fn time_domain(&self, out: &mut [u8]) {
        self.read(out.len(), |i, s| out[i] = to_byte(s));
    }

    /// `getFloatTimeDomainData`: as [`time_domain`](Self::time_domain), in -1..1.
    pub fn time_domain_f32(&self, out: &mut [f32]) {
        self.read(out.len(), |i, s| out[i] = s);
    }

    fn read(&self, len: usize, mut put: impl FnMut(usize, f32)) {
        let n = len.min(FFT_SIZE);
        let end = self.ring.written.load(Ordering::Acquire);
        let start = end as i64 - FFT_SIZE as i64;
        for i in 0..n {
            let at = start + i as i64;
            let s = if at < 0 {
                0.0
            } else {
                f32::from_bits(self.ring.samples[(at as u64 % RING as u64) as usize].load(Ordering::Relaxed))
            };
            put(i, s);
        }
    }
}

/// The Web Audio conversion: `floor(128 * (1 + x))`, clamped to a byte.
#[inline]
fn to_byte(s: f32) -> u8 {
    (128.0 * (1.0 + s)).floor().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_128() {
        let (a, _w) = analyser();
        let mut out = [0u8; FFT_SIZE];
        a.time_domain(&mut out);
        assert!(out.iter().all(|&b| b == 128));
    }

    #[test]
    fn byte_mapping_matches_web_audio() {
        assert_eq!(to_byte(0.0), 128);
        assert_eq!(to_byte(1.0), 255);
        assert_eq!(to_byte(-1.0), 0);
        assert_eq!(to_byte(0.5), 192);
        assert_eq!(to_byte(-0.5), 64);
        assert_eq!(to_byte(4.0), 255);
        assert_eq!(to_byte(-0.001), 127);
    }

    #[test]
    fn returns_the_newest_window_oldest_first() {
        let (a, mut w) = analyser();
        // Write 3000 samples: a ramp, so order and window are visible.
        for i in 0..3000 {
            w.push(i as f32 / 10_000.0);
        }
        // Uncommitted writes are not visible yet.
        let mut f = [0f32; FFT_SIZE];
        a.time_domain_f32(&mut f);
        assert!(f.iter().all(|&s| s == 0.0));
        w.commit();
        a.time_domain_f32(&mut f);
        assert_eq!(f[0], (3000 - 2048) as f32 / 10_000.0);
        assert_eq!(f[FFT_SIZE - 1], 2999.0 / 10_000.0);
        // A short read gets the start of the window, like the Web API.
        let mut short = [0u8; 4];
        a.time_domain(&mut short);
        assert_eq!(short[0], to_byte(f[0]));
    }

    #[test]
    fn readers_never_block_on_a_busy_writer() {
        let (a, mut w) = analyser();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = stop.clone();
        let t = std::thread::spawn(move || {
            while !s2.load(Ordering::Relaxed) {
                for _ in 0..256 {
                    w.push(0.25);
                }
                w.commit();
            }
        });
        let mut out = [0u8; FFT_SIZE];
        let t0 = std::time::Instant::now();
        for _ in 0..1000 {
            a.time_domain(&mut out);
        }
        stop.store(true, Ordering::Relaxed);
        t.join().unwrap();
        assert!(t0.elapsed().as_millis() < 500);
        assert!(out.iter().all(|&b| b == 160 || b == 128));
    }
}
