//! File decoding: any supported file in, interleaved stereo f32 at
//! [`OUT_RATE`] out, with a media position for every frame.
//!
//! symphonia handles MP3, FLAC, WAV, Ogg Vorbis and M4A (AAC / ALAC). Opus is
//! demuxed by symphonia's Ogg reader and decoded by libopus (see `opus.rs`).
//! Anything not already at 48 kHz goes through rubato.

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};

use crate::opus::{OPUS_RATE, OpusDecoder, OpusHead};

/// The engine's one output format: stereo, 48 kHz, f32.
pub const OUT_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

/// Opus needs ~80 ms of decoded audio before a seek point to converge
/// (RFC 7845 §4.6); that much is decoded and thrown away on every seek.
const OPUS_PREROLL_SEC: f64 = 0.08;

/// Resampler input block, in frames.
const RESAMPLE_CHUNK: usize = 1024;

enum Codec {
    Symphonia(Box<dyn AudioDecoder>),
    Opus(OpusDecoder),
}

/// A sample-rate converter to [`OUT_RATE`] with its delay compensated, so the
/// output lines up with the input frame for frame.
struct Resample {
    rs: Fft<f32>,
    ratio: f64,
    input: Vec<f32>,
    output: Vec<f32>,
    /// Output frames still to discard: the filter's startup delay.
    delay_left: usize,
    in_total: u64,
    out_total: u64,
}

impl Resample {
    fn new(from: u32) -> Result<Resample> {
        let rs = Fft::<f32>::new(
            from as usize,
            OUT_RATE as usize,
            RESAMPLE_CHUNK,
            CHANNELS,
            FixedSync::Input,
        )
        .map_err(|e| anyhow!("resampler {from} -> {OUT_RATE}: {e}"))?;
        let output = vec![0.0; rs.output_frames_max() * CHANNELS];
        let delay_left = rs.output_delay();
        Ok(Resample {
            rs,
            ratio: OUT_RATE as f64 / from as f64,
            input: Vec::with_capacity(RESAMPLE_CHUNK * CHANNELS * 2),
            output,
            delay_left,
            in_total: 0,
            out_total: 0,
        })
    }

    fn reset(&mut self) {
        self.rs.reset();
        self.input.clear();
        self.delay_left = self.rs.output_delay();
        self.in_total = 0;
        self.out_total = 0;
    }

    fn push(&mut self, stereo: &[f32], out: &mut Vec<f32>) -> Result<()> {
        self.input.extend_from_slice(stereo);
        self.in_total += (stereo.len() / CHANNELS) as u64;
        while self.input.len() / CHANNELS >= self.rs.input_frames_next() {
            self.run(None, out)?;
        }
        Ok(())
    }

    /// Pushes out what is left in the filter at end of stream.
    fn flush(&mut self, out: &mut Vec<f32>) -> Result<()> {
        let expected = (self.in_total as f64 * self.ratio).round() as u64;
        // Bounded: each pass is at least a few hundred frames.
        for _ in 0..64 {
            if self.out_total >= expected {
                break;
            }
            let have = self.input.len() / CHANNELS;
            self.run(Some(have), out)?;
        }
        if self.out_total > expected {
            let extra = (self.out_total - expected) as usize * CHANNELS;
            out.truncate(out.len().saturating_sub(extra));
            self.out_total = expected;
        }
        Ok(())
    }

    fn run(&mut self, partial: Option<usize>, out: &mut Vec<f32>) -> Result<()> {
        let need = self.rs.input_frames_next();
        let have = self.input.len() / CHANNELS;
        // A partial block is padded by rubato, but the slice still has to be
        // a full block long.
        if have < need {
            self.input.resize(need * CHANNELS, 0.0);
        }
        let indexing = partial.map(|n| Indexing::new().partial_len(n));
        let out_max = self.rs.output_frames_max();
        let (nin, nout) = {
            let inp = InterleavedSlice::new(&self.input[..], CHANNELS, need)
                .map_err(|e| anyhow!("resampler input: {e:?}"))?;
            let mut outp = InterleavedSlice::new_mut(&mut self.output[..], CHANNELS, out_max)
                .map_err(|e| anyhow!("resampler output: {e:?}"))?;
            self.rs
                .process_into_buffer(&inp, &mut outp, indexing.as_ref())
                .map_err(|e| anyhow!("resample: {e}"))?
        };
        let consumed = match partial {
            Some(n) => n.min(nin),
            None => nin,
        };
        self.input.drain(..(consumed * CHANNELS).min(self.input.len()));
        if partial.is_some() {
            self.input.clear();
        }
        let skip = self.delay_left.min(nout);
        self.delay_left -= skip;
        out.extend_from_slice(&self.output[skip * CHANNELS..nout * CHANNELS]);
        self.out_total += (nout - skip) as u64;
        Ok(())
    }
}

/// An open file, decoding on demand.
pub struct Source {
    reader: Box<dyn FormatReader>,
    track_id: u32,
    codec: Codec,
    time_base: TimeBase,
    src_rate: u32,
    /// Subtracted from a packet timestamp to give media time: Opus timestamps
    /// count the pre-skip, which is not part of the track.
    ts_offset: i64,
    duration: f64,
    resample: Option<Resample>,
    /// Decoded, resampled, stereo audio not yet handed out.
    out: Vec<f32>,
    out_read: usize,
    /// Decoded frames before this media time are dropped (seek target,
    /// Opus pre-skip and pre-roll).
    skip_until: f64,
    /// Media time of the first frame handed out since the last seek, and the
    /// number of frames handed out since: the position is derived from these
    /// rather than from packet timestamps, so it is exact at the output rate.
    anchor: f64,
    emitted: u64,
    eof: bool,
    scratch: Vec<f32>,
    stereo: Vec<f32>,
}

impl Source {
    pub fn open(path: &Path) -> Result<Source> {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let reader = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .with_context(|| format!("unsupported or unreadable file: {}", path.display()))?;

        let track = reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| anyhow!("no audio track in {}", path.display()))?;
        let track_id = track.id;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| anyhow!("no codec parameters for {}", path.display()))?
            .clone();

        let (codec, src_rate, ts_offset) = if params.codec == CODEC_ID_OPUS {
            let extra = params.extra_data.as_deref().ok_or_else(|| anyhow!("Opus without OpusHead"))?;
            let head = OpusHead::parse(extra)?;
            let pre_skip = head.pre_skip as i64;
            (Codec::Opus(OpusDecoder::new(head)?), OPUS_RATE, pre_skip)
        } else {
            let dec = symphonia::default::get_codecs()
                .make_audio_decoder(&params, &AudioDecoderOptions::default())
                .with_context(|| format!("no decoder for {}", path.display()))?;
            let rate = params.sample_rate.ok_or_else(|| anyhow!("unknown sample rate"))?;
            (Codec::Symphonia(dec), rate, 0)
        };
        let is_mp4 = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "m4a" | "mp4" | "m4b"));
        let edit = if is_mp4 { crate::mp4::audio_edit(path) } else { None };
        let ts_offset = ts_offset + edit.map_or(0, |e| e.media_start);
        if src_rate == 0 {
            bail!("sample rate of 0");
        }
        let time_base = track
            .time_base
            .or_else(|| TimeBase::try_from_recip(src_rate))
            .ok_or_else(|| anyhow!("no time base"))?;

        let duration = match (track.num_frames, track.duration) {
            _ if edit.and_then(|e| e.duration_secs).is_some() => {
                edit.and_then(|e| e.duration_secs).unwrap_or(0.0)
            }
            // num_frames excludes encoder delay and padding, which is what is
            // actually heard.
            (Some(n), _) if n > 0 => n as f64 / src_rate as f64,
            (_, Some(d)) => time_base
                .calc_duration(d)
                .map(|t| t.as_secs_f64() - ts_offset as f64 / src_rate as f64)
                .unwrap_or(0.0),
            _ => 0.0,
        }
        .max(0.0);

        let resample = if src_rate != OUT_RATE { Some(Resample::new(src_rate)?) } else { None };

        Ok(Source {
            reader,
            track_id,
            codec,
            time_base,
            src_rate,
            ts_offset,
            duration,
            resample,
            out: Vec::with_capacity(8192),
            out_read: 0,
            skip_until: 0.0,
            anchor: 0.0,
            emitted: 0,
            eof: false,
            scratch: Vec::new(),
            stereo: Vec::new(),
        })
    }

    /// Track length in seconds; 0 when the container does not say.
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// Media time of the next frame [`fill`](Self::fill) will return.
    pub fn position(&self) -> f64 {
        self.anchor + self.emitted as f64 / OUT_RATE as f64
    }

    /// Moves to `secs`. The next frame out is the one at `secs`, to within a
    /// frame of the output rate, regardless of where the container could land.
    pub fn seek(&mut self, secs: f64) -> Result<()> {
        let secs = secs.max(0.0);
        let preroll = if matches!(self.codec, Codec::Opus(_)) { OPUS_PREROLL_SEC } else { 0.0 };
        let land = (secs - preroll).max(0.0);
        let ts = Time::try_from_secs_f64(land)
            .and_then(|t| self.time_base.calc_timestamp(t))
            .ok_or_else(|| anyhow!("seek target {secs}s out of range"))?;
        let ts = Timestamp::new(ts.get() + self.ts_offset);

        self.out.clear();
        self.out_read = 0;
        if let Some(r) = &mut self.resample {
            r.reset();
        }
        match &mut self.codec {
            Codec::Symphonia(d) => d.reset(),
            Codec::Opus(d) => d.reset()?,
        }
        self.skip_until = secs;
        self.anchor = secs;
        self.emitted = 0;
        self.eof = false;

        match self.reader.seek(SeekMode::Accurate, SeekTo::Timestamp { ts, track_id: self.track_id }) {
            Ok(_) => Ok(()),
            // Past the end: there is nothing left to play, which is not an error.
            Err(SymError::SeekError(_)) if self.duration > 0.0 && secs >= self.duration - 0.05 => {
                self.eof = true;
                Ok(())
            }
            Err(e) => Err(anyhow!("seek to {secs:.2}s: {e}")),
        }
    }

    /// Fills `dst` (interleaved stereo) and returns the number of frames
    /// written. Fewer than requested means end of stream.
    pub fn fill(&mut self, dst: &mut [f32]) -> Result<usize> {
        let want = dst.len() / CHANNELS;
        while self.buffered() < want && !self.eof {
            self.decode_next()?;
        }
        let n = self.buffered().min(want);
        let start = self.out_read;
        dst[..n * CHANNELS].copy_from_slice(&self.out[start..start + n * CHANNELS]);
        self.out_read += n * CHANNELS;
        if self.out_read == self.out.len() {
            self.out.clear();
            self.out_read = 0;
        } else if self.out_read > 64 * 1024 {
            self.out.drain(..self.out_read);
            self.out_read = 0;
        }
        self.emitted += n as u64;
        Ok(n)
    }

    fn buffered(&self) -> usize {
        (self.out.len() - self.out_read) / CHANNELS
    }

    fn finish(&mut self) -> Result<()> {
        self.eof = true;
        if let Some(r) = &mut self.resample {
            r.flush(&mut self.out)?;
        }
        Ok(())
    }

    /// Decodes one packet into `self.out`, or marks end of stream.
    fn decode_next(&mut self) -> Result<()> {
        let packet = match self.reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return self.finish(),
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return self.finish();
            }
            // Chained Ogg streams: treat the change of stream as the end.
            Err(SymError::ResetRequired) => return self.finish(),
            Err(e) => return Err(anyhow!("demux: {e}")),
        };
        if packet.track_id != self.track_id {
            return Ok(());
        }

        let channels;
        match &mut self.codec {
            Codec::Symphonia(dec) => match dec.decode(&packet) {
                Ok(buf) => {
                    channels = buf.spec().channels().count().max(1);
                    buf.copy_to_vec_interleaved(&mut self.scratch);
                    self.scratch.truncate(buf.frames() * channels);
                }
                // A damaged packet is skipped, as a browser would.
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => return Ok(()),
                Err(e) => return Err(anyhow!("decode: {e}")),
            },
            Codec::Opus(dec) => {
                channels = dec.channels();
                match dec.decode(&packet.data) {
                    Ok(pcm) => {
                        self.scratch.clear();
                        self.scratch.extend_from_slice(pcm);
                    }
                    Err(_) => return Ok(()),
                }
                // symphonia's own decoders apply the packet trims; libopus
                // knows nothing about them.
                let frames = self.scratch.len() / channels;
                let start = (packet.trim_start.get() as usize).min(frames);
                let end = frames.saturating_sub(packet.trim_end.get() as usize).max(start);
                self.scratch.truncate(end * channels);
                self.scratch.drain(..start * channels);
            }
        }

        // Media time of the first frame in `scratch`. The packet timestamp
        // counts trimmed frames, so the trim is added back.
        let first_ts = packet.pts.get() + packet.trim_start.get() as i64 - self.ts_offset;
        let t0 = ts_to_secs(self.time_base, first_ts);

        let frames = self.scratch.len() / channels;
        let drop = if self.skip_until > t0 {
            (((self.skip_until - t0) * self.src_rate as f64).round() as usize).min(frames)
        } else {
            0
        };
        if drop == frames {
            return Ok(());
        }

        to_stereo(&self.scratch[drop * channels..], channels, matches!(self.codec, Codec::Opus(_)), &mut self.stereo);
        match &mut self.resample {
            Some(r) => r.push(&self.stereo, &mut self.out)?,
            None => self.out.extend_from_slice(&self.stereo),
        }
        Ok(())
    }
}

fn ts_to_secs(tb: TimeBase, ts: i64) -> f64 {
    let neg = ts < 0;
    let t = tb.calc_time(Timestamp::new(ts.abs())).map(|t| t.as_secs_f64()).unwrap_or(0.0);
    if neg { -t } else { t }
}

/// Down- or up-mixes interleaved audio to stereo.
///
/// Surround is folded to its front pair (plus centre, where the layout is
/// known) -- this is a music player for stereo libraries, and the odd 5.1 file
/// only needs to be audible, not mastered.
fn to_stereo(src: &[f32], channels: usize, vorbis_order: bool, dst: &mut Vec<f32>) {
    dst.clear();
    match channels {
        1 => {
            for &s in src {
                dst.push(s);
                dst.push(s);
            }
        }
        2 => dst.extend_from_slice(src),
        n => {
            // WAVE order (symphonia): FL FR FC ...; Vorbis/Opus order: FL FC FR ...
            let (l, r, c) = if vorbis_order { (0, 2, Some(1)) } else { (0, 1, Some(2)) };
            for f in src.chunks_exact(n) {
                let cv = c.map(|i| f[i] * std::f32::consts::FRAC_1_SQRT_2).unwrap_or(0.0);
                let scale = 1.0 / (1.0 + std::f32::consts::FRAC_1_SQRT_2);
                dst.push((f[l] + cv) * scale);
                dst.push((f[r] + cv) * scale);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    pub fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    pub const ALL: &[&str] = &[
        "bursts.flac",
        "bursts.wav",
        "bursts.mp3",
        "bursts.ogg",
        "bursts.aac.m4a",
        "bursts.alac.m4a",
        "bursts.opus",
        "bursts.mono.opus",
    ];

    /// The fixtures are 3 s of silence with a 30 ms 1 kHz burst at amplitude
    /// 0.5 starting every 0.5 s from 0.5 s on.
    pub const BURSTS: &[f64] = &[0.5, 1.0, 1.5, 2.0, 2.5];

    pub fn decode_all(src: &mut Source) -> Vec<f32> {
        let mut all = Vec::new();
        let mut buf = vec![0.0; 1000 * CHANNELS];
        loop {
            let n = src.fill(&mut buf).unwrap();
            all.extend_from_slice(&buf[..n * CHANNELS]);
            if n < 1000 {
                return all;
            }
        }
    }

    /// Start times of the bursts in interleaved stereo audio.
    pub fn onsets(stereo: &[f32]) -> Vec<f64> {
        let mut found = Vec::new();
        let mut quiet_since = 0usize;
        let mut last = None::<usize>;
        for (i, f) in stereo.chunks_exact(2).enumerate() {
            let loud = f[0].abs().max(f[1].abs()) > 0.2;
            if loud {
                if last.is_none_or(|l| i - l > OUT_RATE as usize / 10) && i >= quiet_since {
                    found.push(i as f64 / OUT_RATE as f64);
                }
                last = Some(i);
            } else {
                quiet_since = i;
            }
        }
        found
    }

    #[test]
    fn decodes_every_format_with_bursts_on_time() {
        for name in ALL {
            let mut src = Source::open(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            let dur = src.duration();
            assert!((dur - 3.0).abs() < 0.03, "{name}: duration {dur}");
            let pcm = decode_all(&mut src);
            let secs = pcm.len() as f64 / 2.0 / OUT_RATE as f64;
            assert!((secs - 3.0).abs() < 0.03, "{name}: decoded {secs}s");
            let on = onsets(&pcm);
            assert_eq!(on.len(), BURSTS.len(), "{name}: onsets {on:?}");
            for (got, want) in on.iter().zip(BURSTS) {
                assert!((got - want).abs() < 0.006, "{name}: burst at {got}, want {want} ({on:?})");
            }
            let peak = pcm.iter().fold(0f32, |a, s| a.max(s.abs()));
            assert!((0.4..0.7).contains(&peak), "{name}: peak {peak}");
            assert!((src.position() - secs).abs() < 1e-6, "{name}: position tracks output");
        }
    }

    #[test]
    fn seek_lands_on_the_requested_time() {
        for name in ALL {
            for target in [0.25, 1.2, 1.75, 2.3] {
                let mut src = Source::open(&fixture(name)).unwrap();
                // Read a little first, as playback would have.
                let mut buf = vec![0.0; 4800 * CHANNELS];
                src.fill(&mut buf).unwrap();
                src.seek(target).unwrap();
                assert!((src.position() - target).abs() < 1e-9);
                let pcm = decode_all(&mut src);
                let on = onsets(&pcm);
                let expect: Vec<f64> =
                    BURSTS.iter().filter(|b| **b > target).map(|b| b - target).collect();
                assert_eq!(on.len(), expect.len(), "{name} @{target}: {on:?} vs {expect:?}");
                for (got, want) in on.iter().zip(&expect) {
                    assert!(
                        (got - want).abs() < 0.010,
                        "{name} @{target}: burst at +{got}, want +{want}"
                    );
                }
                let end = target + pcm.len() as f64 / 2.0 / OUT_RATE as f64;
                assert!((end - 3.0).abs() < 0.03, "{name} @{target}: ends at {end}");
            }
        }
    }

    #[test]
    fn seek_past_the_end_is_end_of_stream() {
        let mut src = Source::open(&fixture("bursts.flac")).unwrap();
        src.seek(3.5).ok();
        let mut buf = vec![0.0; 1024 * CHANNELS];
        assert_eq!(src.fill(&mut buf).unwrap(), 0);
    }

    #[test]
    fn missing_or_bogus_files_error() {
        assert!(Source::open(Path::new("/nonexistent/x.flac")).is_err());
        let not_audio = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(Source::open(&not_audio).is_err());
    }

    #[test]
    fn mono_is_duplicated_and_surround_folds() {
        let mut out = Vec::new();
        to_stereo(&[0.1, 0.2], 1, false, &mut out);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2]);
        to_stereo(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 6, false, &mut out);
        assert!(out[0] > 0.5 && out[1] == 0.0);
    }

    /// Real-world decode of the user's library, if there is one. Decode only.
    #[test]
    #[ignore = "reads ~/Music; run with --ignored"]
    fn decodes_real_library_files() {
        let home = std::env::var("HOME").unwrap();
        let mut files = Vec::new();
        let mut stack = vec![PathBuf::from(home).join("Music")];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if matches!(
                    p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
                    Some("flac" | "mp3" | "ogg" | "opus" | "m4a" | "wav")
                ) {
                    files.push(p);
                }
            }
        }
        files.sort();
        let step = (files.len() / 12).max(1);
        for p in files.iter().step_by(step) {
            let mut src = Source::open(p).unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
            let dur = src.duration();
            assert!(dur > 1.0, "{}: duration {dur}", p.display());
            src.seek(dur / 2.0).unwrap();
            let mut buf = vec![0.0; 48000 * CHANNELS];
            let n = src.fill(&mut buf).unwrap();
            assert_eq!(n, 48000, "{}", p.display());
            let peak = buf.iter().fold(0f32, |a, s| a.max(s.abs()));
            assert!(peak > 0.0, "{}: silent at midpoint", p.display());
            eprintln!("ok {:>7.1}s peak {peak:.2} {}", dur, p.display());
        }
    }
}
