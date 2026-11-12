//! Opus decoding through the system libopus.
//!
//! symphonia demuxes Ogg Opus (it reads the OpusHead and gives packets
//! timestamps) but has no Opus decoder, so packets are handed to libopus here.
//! Everything goes through the multistream decoder: with one stream it is the
//! plain decoder, and it also covers surround files (mapping family 1).

use anyhow::{Context, Result, bail};
use opusic_sys as ffi;

/// Opus always decodes at 48 kHz, which is also the engine's output rate.
pub(crate) const OPUS_RATE: u32 = 48_000;

/// The longest Opus packet is 120 ms.
const MAX_FRAME: usize = 5760;

/// The parts of the identification header (RFC 7845 §5.1) decoding needs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OpusHead {
    pub channels: u8,
    pub pre_skip: u16,
    /// Q7.8 dB, to be applied to every decoded sample.
    pub output_gain: i16,
    pub mapping_family: u8,
    pub streams: u8,
    pub coupled: u8,
    pub mapping: Vec<u8>,
}

impl OpusHead {
    pub fn parse(buf: &[u8]) -> Result<OpusHead> {
        if buf.len() < 19 || &buf[..8] != b"OpusHead" {
            bail!("not an OpusHead packet");
        }
        let channels = buf[9];
        if channels == 0 {
            bail!("OpusHead declares zero channels");
        }
        let pre_skip = u16::from_le_bytes([buf[10], buf[11]]);
        let output_gain = i16::from_le_bytes([buf[16], buf[17]]);
        let mapping_family = buf[18];
        let (streams, coupled, mapping) = if mapping_family == 0 {
            if channels > 2 {
                bail!("mapping family 0 with {channels} channels");
            }
            (1, channels - 1, (0..channels).collect())
        } else {
            let need = 21 + channels as usize;
            if buf.len() < need {
                bail!("truncated OpusHead channel mapping");
            }
            (buf[19], buf[20], buf[21..need].to_vec())
        };
        Ok(OpusHead { channels, pre_skip, output_gain, mapping_family, streams, coupled, mapping })
    }

    /// The header's output gain as a linear multiplier.
    pub fn gain_multiplier(&self) -> f32 {
        10f32.powf(self.output_gain as f32 / (20.0 * 256.0))
    }
}

pub(crate) struct OpusDecoder {
    st: *mut ffi::OpusMSDecoder,
    head: OpusHead,
    gain: f32,
    pcm: Vec<f32>,
}

// The decoder state is plain heap memory owned by this struct alone; libopus
// keeps no thread affinity.
unsafe impl Send for OpusDecoder {}

impl OpusDecoder {
    pub fn new(head: OpusHead) -> Result<OpusDecoder> {
        let mut err: std::ffi::c_int = 0;
        let st = unsafe {
            ffi::opus_multistream_decoder_create(
                OPUS_RATE as i32,
                head.channels as std::ffi::c_int,
                head.streams as std::ffi::c_int,
                head.coupled as std::ffi::c_int,
                head.mapping.as_ptr(),
                &mut err,
            )
        };
        if st.is_null() || err != ffi::OPUS_OK {
            bail!("libopus refused the stream (error {err})");
        }
        let gain = head.gain_multiplier();
        let pcm = vec![0.0; MAX_FRAME * head.channels as usize];
        Ok(OpusDecoder { st, head, gain, pcm })
    }

    pub fn channels(&self) -> usize {
        self.head.channels as usize
    }

    /// Drops all decoder state, for after a seek.
    pub fn reset(&mut self) -> Result<()> {
        let fresh = OpusDecoder::new(self.head.clone()).context("resetting opus decoder")?;
        *self = fresh;
        Ok(())
    }

    /// Decodes one packet; returns interleaved samples (`channels()` wide).
    pub fn decode(&mut self, packet: &[u8]) -> Result<&[f32]> {
        let n = unsafe {
            ffi::opus_multistream_decode_float(
                self.st,
                packet.as_ptr(),
                packet.len() as i32,
                self.pcm.as_mut_ptr(),
                MAX_FRAME as std::ffi::c_int,
                0,
            )
        };
        if n < 0 {
            bail!("opus decode error {n}");
        }
        let len = n as usize * self.channels();
        if self.gain != 1.0 {
            for s in &mut self.pcm[..len] {
                *s *= self.gain;
            }
        }
        Ok(&self.pcm[..len])
    }
}

impl Drop for OpusDecoder {
    fn drop(&mut self) {
        unsafe { ffi::opus_multistream_decoder_destroy(self.st) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_family_zero_head() {
        let mut h = b"OpusHead".to_vec();
        h.extend_from_slice(&[1, 2]);
        h.extend_from_slice(&312u16.to_le_bytes());
        h.extend_from_slice(&48000u32.to_le_bytes());
        h.extend_from_slice(&(-256i16).to_le_bytes());
        h.push(0);
        let head = OpusHead::parse(&h).unwrap();
        assert_eq!(head.channels, 2);
        assert_eq!(head.pre_skip, 312);
        assert_eq!((head.streams, head.coupled), (1, 1));
        // -1 dB
        assert!((head.gain_multiplier() - 0.891_25).abs() < 1e-4);
        assert!(OpusDecoder::new(head).is_ok());
    }

    #[test]
    fn rejects_garbage() {
        assert!(OpusHead::parse(b"OpusTags........................").is_err());
    }
}
