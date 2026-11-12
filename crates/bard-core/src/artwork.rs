//! The cover cache: `artwork/<trackId>.jpg`. A port of `src/main/artwork.ts`.
//!
//! Cover art deliberately never travels inside a Track -- a 1.2 MB JPEG times
//! several hundred tracks is not something to hold in memory or push through
//! the UI's state. Only the id crosses the boundary; whoever draws a cover
//! reads the cached file by path.
//!
//! The id comes from the path alone, so the cache cannot tell by name that a
//! file's art changed. Beside each entry is `<trackId>.src`: the source file's
//! size and modification time when it was read, and whether it had art. A
//! scan reads the art again only when those differ, or the JPEG is missing or
//! cut short; an unchanged file -- with art or without -- costs a stat. The
//! Electron build kept a cover forever once written, so replaced art never
//! showed and a truncated file blocked its own repair.

use image::ImageReader;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Longest edge, in px, that a cached cover is stored at.
pub const MAX_EDGE: u32 = 512;

/// JPEG quality the Electron build wrote with (`nativeImage.toJPEG(88)`).
pub const JPEG_QUALITY: u8 = 88;

pub fn cache_path(artwork_dir: &Path, track_id: &str) -> PathBuf {
    artwork_dir.join(format!("{track_id}.jpg"))
}

/// A track id is 16 lowercase hex chars; anything else is refused before it
/// is joined onto a path. (The overlay's /art route checked
/// `/^[a-f0-9]{6,64}$/` for the same reason.)
pub fn is_valid_id(id: &str) -> bool {
    (6..=64).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn stamp_path(artwork_dir: &Path, track_id: &str) -> PathBuf {
    artwork_dir.join(format!("{track_id}.src"))
}

/// What identifies a version of a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub mtime_ns: u128,
}

impl Fingerprint {
    pub fn of(path: &Path) -> Option<Fingerprint> {
        let m = std::fs::metadata(path).ok()?;
        let mtime_ns = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
        Some(Fingerprint { len: m.len(), mtime_ns })
    }
}

/// Whether the cache can be trusted for a source file as it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cached {
    /// Read from this version of the file; `has_art` says what it found.
    Fresh { has_art: bool },
    /// Never read, or read from another version, or the JPEG is damaged.
    Stale,
}

pub fn cached(artwork_dir: &Path, track_id: &str, source: Option<Fingerprint>) -> Cached {
    let Some(fp) = source else { return Cached::Stale };
    let Ok(stamp) = std::fs::read_to_string(stamp_path(artwork_dir, track_id)) else { return Cached::Stale };
    let mut f = stamp.split_whitespace();
    let (Some("1"), Some(len), Some(mtime), Some(art)) = (f.next(), f.next(), f.next(), f.next()) else {
        return Cached::Stale;
    };
    if len.parse() != Ok(fp.len) || mtime.parse() != Ok(fp.mtime_ns) {
        return Cached::Stale;
    }
    match art {
        "art" if is_whole_jpeg(&cache_path(artwork_dir, track_id)) => Cached::Fresh { has_art: true },
        "none" if !cache_path(artwork_dir, track_id).exists() => Cached::Fresh { has_art: false },
        _ => Cached::Stale,
    }
}

/// Records what was found in `source`'s current version. After the cover
/// itself, so a crash between the two leaves the entry stale, not wrong.
pub fn record(artwork_dir: &Path, track_id: &str, source: Fingerprint, has_art: bool) {
    let body = format!("1 {} {} {}", source.len, source.mtime_ns, if has_art { "art" } else { "none" });
    let _ = crate::util::write_atomic(&stamp_path(artwork_dir, track_id), &body);
}

/// The file no longer has art: drop the cover.
pub fn forget(artwork_dir: &Path, track_id: &str) {
    let _ = std::fs::remove_file(cache_path(artwork_dir, track_id));
}

/// Starts with the JPEG start marker and ends with the end marker -- what a
/// cut-short write lacks. Reads four bytes, not the image: this runs for
/// every track on every scan.
pub fn is_whole_jpeg(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let (mut head, mut tail) = ([0u8; 2], [0u8; 2]);
    f.read_exact(&mut head).is_ok()
        && f.seek(SeekFrom::End(-2)).is_ok()
        && f.read_exact(&mut tail).is_ok()
        && head == [0xFF, 0xD8]
        && tail == [0xFF, 0xD9]
}

/// The cached cover's path, or None when the track has none.
pub fn artwork_path(artwork_dir: &Path, track_id: &str) -> Option<PathBuf> {
    if !is_valid_id(track_id) {
        return None;
    }
    let p = cache_path(artwork_dir, track_id);
    p.is_file().then_some(p)
}

/// Downscales an embedded cover and writes it to the artwork cache.
///
/// Same output as the Electron build: the longest edge is brought down to
/// [`MAX_EDGE`] with the aspect ratio kept (the other edge rounded), smaller
/// images are left at their size, and everything is re-encoded as a JPEG at
/// quality 88 -- so a cached PNG cover is still `<id>.jpg`. Returns false when
/// the data is not a decodable image; the scan carries on without a cover.
pub fn save_artwork(artwork_dir: &Path, track_id: &str, data: &[u8]) -> bool {
    let Ok(reader) = ImageReader::new(Cursor::new(data)).with_guessed_format() else {
        return false;
    };
    let Ok(img) = reader.decode() else { return false };
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return false;
    }

    let longest = w.max(h);
    let img = if longest > MAX_EDGE {
        let (nw, nh) = if w >= h {
            (MAX_EDGE, ((h as f64 * MAX_EDGE as f64 / w as f64).round() as u32).max(1))
        } else {
            (((w as f64 * MAX_EDGE as f64 / h as f64).round() as u32).max(1), MAX_EDGE)
        };
        img.resize_exact(nw, nh, FilterType::CatmullRom)
    } else {
        img
    };

    // JPEG has no alpha; flatten as toJPEG does.
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    if JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY).encode_image(&rgb).is_err() {
        return false;
    }
    if std::fs::create_dir_all(artwork_dir).is_err() {
        return false;
    }
    // Replaced whole: anything reading the cover sees the old one or the new.
    let path = cache_path(artwork_dir, track_id);
    let tmp = path.with_extension("jpg.tmp");
    std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, &path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, ImageFormat, Rgb};

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = ImageBuffer::from_fn(w, h, |x, _| Rgb([(x % 256) as u8, 40, 90]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn downscales_the_longest_edge_and_writes_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_artwork(dir.path(), "abcdef0123456789", &png(1000, 600)));
        let p = artwork_path(dir.path(), "abcdef0123456789").unwrap();
        let img = image::open(&p).unwrap();
        assert_eq!((img.width(), img.height()), (512, 307));
        assert_eq!(image::ImageFormat::from_path(&p).unwrap(), ImageFormat::Jpeg);
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn portrait_scales_by_height_and_small_images_keep_their_size() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_artwork(dir.path(), "aaaaaa", &png(300, 900)));
        let img = image::open(cache_path(dir.path(), "aaaaaa")).unwrap();
        assert_eq!((img.width(), img.height()), (171, 512));

        assert!(save_artwork(dir.path(), "bbbbbb", &png(200, 100)));
        let img = image::open(cache_path(dir.path(), "bbbbbb")).unwrap();
        assert_eq!((img.width(), img.height()), (200, 100));
    }

    #[test]
    fn refuses_garbage_and_hostile_ids() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!save_artwork(dir.path(), "cccccc", b"not an image"));
        assert!(artwork_path(dir.path(), "cccccc").is_none());
        assert!(artwork_path(dir.path(), "../../etc/passwd").is_none());
        assert!(artwork_path(dir.path(), "ABCDEF").is_none());
    }
}
