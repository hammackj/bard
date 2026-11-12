//! The MP4 edit list, which symphonia's isomp4 reader does not apply yet.
//!
//! AAC encoders prime the decoder with ~1024 frames that are not part of the
//! track, and record that in the audio track's `elst` box. Ignoring it puts
//! every M4A 21 ms late -- positions, seeks and the end of the track all
//! drift by the priming. This reads just enough of `moov` to recover it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The audio track's edit: frames to drop from the start, in the track's
/// media timescale, and the playable length in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Edit {
    pub media_start: i64,
    pub duration_secs: Option<f64>,
}

const MAX_MOOV: u64 = 64 << 20;

pub(crate) fn audio_edit(path: &Path) -> Option<Edit> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut pos = 0u64;
    while pos + 8 <= len {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut hdr = [0u8; 16];
        f.read_exact(&mut hdr[..8]).ok()?;
        let mut size = u32::from_be_bytes(hdr[..4].try_into().ok()?) as u64;
        let mut head = 8;
        if size == 1 {
            f.read_exact(&mut hdr[8..16]).ok()?;
            size = u64::from_be_bytes(hdr[8..16].try_into().ok()?);
            head = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < head {
            return None;
        }
        if &hdr[4..8] == b"moov" {
            if size > MAX_MOOV {
                return None;
            }
            let mut moov = vec![0u8; (size - head) as usize];
            f.read_exact(&mut moov).ok()?;
            return parse_moov(&moov);
        }
        pos += size;
    }
    None
}

/// Iterates the child boxes of a box body.
fn boxes(mut body: &[u8]) -> impl Iterator<Item = (&[u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        if body.len() < 8 {
            return None;
        }
        let mut size = u32::from_be_bytes(body[..4].try_into().ok()?) as usize;
        let kind: &[u8; 4] = body[4..8].try_into().ok()?;
        let mut head = 8;
        if size == 1 {
            size = u64::from_be_bytes(body.get(8..16)?.try_into().ok()?) as usize;
            head = 16;
        } else if size == 0 {
            size = body.len();
        }
        if size < head || size > body.len() {
            return None;
        }
        let inner = &body[head..size];
        body = &body[size..];
        Some((kind, inner))
    })
}

fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(body).find(|(k, _)| *k == kind).map(|(_, b)| b)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Timescale of an `mvhd` or `mdhd` body.
fn timescale(b: &[u8]) -> Option<u32> {
    match b.first()? {
        0 => be32(b, 12),
        _ => be32(b, 20),
    }
}

fn parse_moov(moov: &[u8]) -> Option<Edit> {
    let movie_scale = timescale(child(moov, b"mvhd")?)?;
    for (kind, trak) in boxes(moov) {
        if kind != b"trak" {
            continue;
        }
        let Some(mdia) = child(trak, b"mdia") else { continue };
        let is_audio = child(mdia, b"hdlr").and_then(|h| h.get(8..12)) == Some(b"soun");
        if !is_audio {
            continue;
        }
        let elst = child(child(trak, b"edts")?, b"elst")?;
        let version = *elst.first()?;
        let count = be32(elst, 4)? as usize;
        let mut at = 8;
        for _ in 0..count {
            let (seg, media) = if version == 1 {
                let s = be64(elst, at)?;
                let m = be64(elst, at + 8)? as i64;
                at += 20;
                (s, m)
            } else {
                let s = be32(elst, at)? as u64;
                let m = be32(elst, at + 4)? as i32 as i64;
                at += 12;
                (s, m)
            };
            // -1 is an empty edit (a gap before the media starts); skip it.
            if media < 0 {
                continue;
            }
            let duration_secs =
                (seg > 0 && movie_scale > 0).then(|| seg as f64 / movie_scale as f64);
            return Some(Edit { media_start: media, duration_secs });
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::tests::fixture;

    #[test]
    fn reads_aac_priming() {
        let e = audio_edit(&fixture("bursts.aac.m4a")).expect("edit list");
        assert_eq!(e.media_start, 1024);
        assert!((e.duration_secs.unwrap() - 3.0).abs() < 0.01, "{e:?}");
    }

    #[test]
    fn non_mp4_has_none() {
        assert_eq!(audio_edit(&fixture("bursts.flac")), None);
    }
}
