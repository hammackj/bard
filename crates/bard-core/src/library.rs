//! Finding and reading audio files. A port of `src/main/library.ts`.
//!
//! Tags, ReplayGain, duration and embedded covers come from `lofty` where the
//! Electron build used `music-metadata`. FLAC durations are read straight from
//! STREAMINFO (total samples / sample rate) so they match music-metadata to the
//! last digit; lofty only reports whole milliseconds.

use crate::artwork;
use crate::paths::resolve;
use crate::playlist::by_path_order;
use crate::search::search_text_for;
use crate::types::{ScanProgress, Track};
use lofty::config::ParseOptions;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::Tag;
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "flac", "wav", "ogg", "m4a", "opus"];

/// Parsing many files at once exhausts file handles; a small pool is plenty.
pub const PARSE_CONCURRENCY: usize = 8;

/// First 16 hex chars of sha1(resolved path).
///
/// Resolved the way Node's `path.resolve` does it -- made absolute and `.`/`..`
/// folded, but symlinks left alone -- and hashed as the path's bytes (UTF-8,
/// as Node hashed the string). This keys stats.json and the artwork cache, so
/// it must never change: a different id detaches every play count and cover.
pub fn track_id(path: impl AsRef<Path>) -> String {
    let resolved = resolve(path);
    let digest = Sha1::digest(resolved.as_os_str().as_bytes());
    let mut hex = String::with_capacity(16);
    for b in &digest[..8] {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

/// Every audio file under `roots`, de-duplicated.
///
/// Mirrors the fast-glob call it replaces: `**/*.{mp3,flac,...}`, extensions
/// matched case-insensitively, hidden files and directories skipped (glob's
/// `dot: false`), symlinks not followed and not returned, unreadable
/// directories skipped silently.
pub fn find_audio_files<S: AsRef<str>>(roots: &[S]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for root in roots {
        let root = resolve(root.as_ref());
        let walker = walkdir::WalkDir::new(&root).follow_links(false).into_iter();
        let entries = walker.filter_entry(|e| e.depth() == 0 || !e.file_name().as_bytes().starts_with(b"."));
        for entry in entries.flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let p = entry.path();
            let ok = p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)));
            // Track paths are strings; a name that is not UTF-8 cannot be one.
            if ok && p.to_str().is_some() && seen.insert(p.to_path_buf()) {
                out.push(p.to_path_buf());
            }
        }
    }
    out
}

/// Reads every audio file under `roots`, caching covers into `artwork_dir`.
///
/// Blocking. Parses on [`PARSE_CONCURRENCY`] threads pulling from a shared
/// cursor; `on_progress` is called once per file, serialised, with `scanned`
/// counting up to `total`. The result is in album order (path order).
pub fn scan<S: AsRef<str>>(roots: &[S], artwork_dir: &Path, on_progress: &(dyn Fn(ScanProgress) + Sync)) -> Vec<Track> {
    let _ = std::fs::create_dir_all(artwork_dir);
    let files = find_audio_files(roots);
    let total = files.len();
    let cursor = AtomicUsize::new(0);
    let state: Mutex<(usize, Vec<Track>)> = Mutex::new((0, Vec::with_capacity(total)));

    std::thread::scope(|s| {
        for _ in 0..PARSE_CONCURRENCY.min(total) {
            s.spawn(|| {
                loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(i) else { return };
                    let track = read_track(path, artwork_dir);
                    let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                    st.0 += 1;
                    st.1.push(track);
                    on_progress(ScanProgress {
                        scanned: st.0,
                        total,
                        current_path: path.to_string_lossy().into_owned(),
                    });
                }
            });
        }
    });

    let mut found = state.into_inner().unwrap_or_else(|e| e.into_inner()).1;
    found.sort_by(by_path_order);
    found
}

/// Reads one file's tags into a Track, caching its cover on the way.
///
/// An unreadable or malformed file does not abort the scan: it falls back to
/// what the path alone says, exactly as the Electron build did (including a
/// searchText of just the lowercased file name).
pub fn read_track(path: &Path, artwork_dir: &Path) -> Track {
    let id = track_id(path);
    let path_str = path.to_string_lossy().into_owned();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // Read the cover only when the file changed since it was last read (or
    // the cached one is damaged); an unchanged library skips the decode.
    let source = artwork::Fingerprint::of(path);
    let fresh = artwork::cached(artwork_dir, &id, source);
    let read_art = fresh == artwork::Cached::Stale;
    // What to say without reading: the cache, or for a file that cannot be
    // parsed, whatever cover is already there.
    let cached = match fresh {
        artwork::Cached::Fresh { has_art } => has_art,
        artwork::Cached::Stale => artwork::is_whole_jpeg(&artwork::cache_path(artwork_dir, &id)),
    };

    let parsed = Probe::open(path)
        .and_then(|p| Ok(p.options(ParseOptions::new().read_cover_art(read_art)).guess_file_type()?))
        .and_then(|p| p.read());
    let Ok(file) = parsed else {
        return Track {
            id,
            path: path_str,
            title: stem.clone(),
            artist: "Unknown Artist".into(),
            album: "Unknown Album".into(),
            duration_sec: 0.0,
            has_artwork: cached,
            search_text: stem.to_lowercase(),
            track_gain_db: None,
            album_gain_db: None,
            track_peak: None,
            album_peak: None,
        };
    };

    // Primary tag first, then whatever else the file carries.
    let primary = file.primary_tag_type();
    let mut tags: Vec<&Tag> = file.tags().iter().collect();
    tags.sort_by_key(|t| t.tag_type() != primary);
    let get = |key: ItemKey| -> Option<String> { tags.iter().find_map(|t| clean(t.get_string(key))) };

    let mut has_artwork = cached;
    if read_art {
        let saved = match tags.iter().find_map(|t| t.pictures().first()) {
            Some(pic) => artwork::save_artwork(artwork_dir, &id, pic.data()),
            None => artwork::Saved::NotAnImage,
        };
        match saved {
            artwork::Saved::Cover => has_artwork = true,
            artwork::Saved::NotAnImage => {
                // Removed from the file, or no longer decodable: so is the cover.
                artwork::forget(artwork_dir, &id);
                has_artwork = false;
            }
            // The cache could not store it. Nothing is recorded, so the next
            // scan reads the art again; a good cover from before stays.
            artwork::Saved::WriteFailed => {
                has_artwork = artwork::is_whole_jpeg(&artwork::cache_path(artwork_dir, &id));
            }
        }
        if let Some(fp) = source
            && saved != artwork::Saved::WriteFailed
        {
            artwork::record(artwork_dir, &id, fp, has_artwork);
        }
    }

    let title = get(ItemKey::TrackTitle).unwrap_or(stem);
    let artist =
        get(ItemKey::TrackArtist).or_else(|| get(ItemKey::AlbumArtist)).unwrap_or_else(|| "Unknown Artist".into());
    let album = get(ItemKey::AlbumTitle).unwrap_or_else(|| "Unknown Album".into());
    let duration_sec = flac_duration(path).unwrap_or_else(|| file.properties().duration().as_secs_f64());

    Track {
        search_text: search_text_for(&title, &artist, &album),
        id,
        path: path_str,
        title,
        artist,
        album,
        duration_sec,
        has_artwork,
        track_gain_db: get(ItemKey::ReplayGainTrackGain).and_then(|v| parse_float(&v)),
        album_gain_db: get(ItemKey::ReplayGainAlbumGain).and_then(|v| parse_float(&v)),
        track_peak: get(ItemKey::ReplayGainTrackPeak).and_then(|v| parse_float(&v)),
        album_peak: get(ItemKey::ReplayGainAlbumPeak).and_then(|v| parse_float(&v)),
    }
}

fn clean(v: Option<&str>) -> Option<String> {
    let t = v?.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// JavaScript's `parseFloat`: the longest numeric prefix, so "-6.54 dB" is
/// -6.54. None when there is no number at all.
fn parse_float(s: &str) -> Option<f64> {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut end = 0;
    let mut best = None;
    let mut seen_dot = false;
    let mut seen_e = false;
    if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
        end += 1;
    }
    while end < b.len() {
        let c = b[end];
        let ok = c.is_ascii_digit()
            || (c == b'.' && !seen_dot && !seen_e)
            || ((c == b'e' || c == b'E') && !seen_e && best.is_some())
            || ((c == b'+' || c == b'-') && matches!(b[end - 1], b'e' | b'E'));
        if !ok {
            break;
        }
        seen_dot |= c == b'.';
        seen_e |= c == b'e' || c == b'E';
        end += 1;
        if let Ok(v) = s[..end].parse::<f64>() {
            best = Some(v);
        }
    }
    best.filter(|v| v.is_finite())
}

/// Exact FLAC duration from STREAMINFO, skipping a leading ID3v2 tag.
fn flac_duration(path: &Path) -> Option<f64> {
    if !path.extension()?.to_str()?.eq_ignore_ascii_case("flac") {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 10];
    f.read_exact(&mut head[..4]).ok()?;
    if &head[..3] == b"ID3" {
        f.read_exact(&mut head[4..10]).ok()?;
        let size = head[6..10].iter().fold(0u64, |acc, b| (acc << 7) | u64::from(b & 0x7f));
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        use std::io::{Seek, SeekFrom};
        f.seek(SeekFrom::Start(10 + size + footer)).ok()?;
        f.read_exact(&mut head[..4]).ok()?;
    }
    if &head[..4] != b"fLaC" {
        return None;
    }
    let mut block = [0u8; 4 + 34];
    f.read_exact(&mut block).ok()?;
    if block[0] & 0x7f != 0 {
        return None; // STREAMINFO must come first
    }
    let si = &block[4..];
    let sample_rate = (u32::from(si[10]) << 12) | (u32::from(si[11]) << 4) | (u32::from(si[12]) >> 4);
    let total = (u64::from(si[13] & 0x0f) << 32)
        | (u64::from(si[14]) << 24)
        | (u64::from(si[15]) << 16)
        | (u64::from(si[16]) << 8)
        | u64::from(si[17]);
    (sample_rate > 0 && total > 0).then(|| total as f64 / f64::from(sample_rate))
}

/// The scanned library, with an id index.
#[derive(Debug, Default, Clone)]
pub struct Library {
    tracks: Vec<Track>,
    by_id: HashMap<String, usize>,
}

impl Library {
    pub fn new(tracks: Vec<Track>) -> Self {
        let by_id = tracks.iter().enumerate().map(|(i, t)| (t.id.clone(), i)).collect();
        Library { tracks, by_id }
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn get(&self, id: &str) -> Option<&Track> {
        self.by_id.get(id).map(|i| &self.tracks[*i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_sha1_of_the_resolved_path() {
        // The first 16 hex digits of sha1(path), as the Electron build's ids
        // were, so stats.json carries over.
        let p = "/home/user/Music/Artist/Album/01 - Track.flac";
        assert_eq!(track_id(p), "50f36209b3904081");
        // Dots fold, as path.resolve does.
        let dotted = "/home/user/Music/./Artist/x/../Album/01 - Track.flac";
        assert_eq!(track_id(dotted), "50f36209b3904081");
    }

    #[test]
    fn parse_float_takes_the_numeric_prefix() {
        assert_eq!(parse_float("-6.54 dB"), Some(-6.54));
        assert_eq!(parse_float(" +2 dB"), Some(2.0));
        assert_eq!(parse_float("0.988"), Some(0.988));
        assert_eq!(parse_float("1e-3x"), Some(0.001));
        assert_eq!(parse_float("dB"), None);
        assert_eq!(parse_float(""), None);
    }

    #[test]
    fn finds_audio_files_like_the_glob_did() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("A/B")).unwrap();
        std::fs::create_dir_all(r.join(".hidden")).unwrap();
        for f in ["A/1.flac", "A/B/2.MP3", "A/cover.jpg", ".hidden/3.flac", "A/.4.ogg", "5.Opus"] {
            std::fs::write(r.join(f), b"x").unwrap();
        }
        std::os::unix::fs::symlink(r.join("A/1.flac"), r.join("link.flac")).unwrap();
        let root = r.to_str().unwrap().to_string();
        // Listing a root twice does not list its files twice.
        let mut found: Vec<String> = find_audio_files(&[root.clone(), root.clone()])
            .iter()
            .map(|p| p.strip_prefix(r).unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, ["5.Opus", "A/1.flac", "A/B/2.MP3"]);
        assert!(find_audio_files(&["/nonexistent/bard"]).is_empty());
    }

    #[test]
    fn unreadable_files_fall_back_to_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("Some Song.flac");
        std::fs::write(&f, b"definitely not flac").unwrap();
        let art = dir.path().join("artwork");
        let tracks = scan(&[dir.path().to_str().unwrap()], &art, &|_| {});
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.title, "Some Song");
        assert_eq!(t.artist, "Unknown Artist");
        assert_eq!(t.album, "Unknown Album");
        assert_eq!(t.search_text, "some song");
        assert_eq!(t.duration_sec, 0.0);
        assert!(!t.has_artwork);
        assert_eq!(t.id, track_id(&f));
        assert!(art.is_dir());
    }

    /// A copy of the bundled FLAC with a solid `color` cover, or none.
    fn flac_with_art(path: &Path, color: Option<u8>) {
        use lofty::config::WriteOptions;
        use lofty::picture::{MimeType, Picture, PictureType};
        use lofty::tag::Tag;
        if !path.exists() {
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bard-audio/tests/fixtures/bursts.flac");
            std::fs::copy(fixture, path).unwrap();
        }
        let mut f = lofty::read_from_path(path).unwrap();
        if f.primary_tag_mut().is_none() {
            f.insert_tag(Tag::new(f.primary_tag_type()));
        }
        let tag = f.primary_tag_mut().unwrap();
        tag.remove_picture_type(PictureType::CoverFront);
        if let Some(c) = color {
            let img = image::RgbImage::from_pixel(64, 64, image::Rgb([c, c, c]));
            let mut png = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png, image::ImageFormat::Png).unwrap();
            tag.push_picture(
                Picture::unchecked(png.into_inner()).pic_type(PictureType::CoverFront).mime_type(MimeType::Png).build(),
            );
        }
        f.save_to_path(path, WriteOptions::default()).unwrap();
    }

    fn cover_shade(art: &Path, id: &str) -> Option<u8> {
        let img = image::open(artwork::cache_path(art, id)).ok()?.to_rgb8();
        Some(img.get_pixel(32, 32)[0])
    }

    fn modified(p: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(p).and_then(|m| m.modified()).ok()
    }

    #[test]
    fn replaced_or_removed_art_at_the_same_path_is_picked_up() {
        let dir = tempfile::tempdir().unwrap();
        let (f, art) = (dir.path().join("song.flac"), dir.path().join("artwork"));
        flac_with_art(&f, Some(40));
        let t = read_track(&f, &art);
        assert!(t.has_artwork);
        assert!(cover_shade(&art, &t.id).unwrap().abs_diff(40) < 4);

        flac_with_art(&f, Some(200));
        let t = read_track(&f, &art);
        assert!(t.has_artwork);
        assert!(cover_shade(&art, &t.id).unwrap().abs_diff(200) < 4, "the new art, not the cached");

        flac_with_art(&f, None);
        let t = read_track(&f, &art);
        assert!(!t.has_artwork, "art removed from the file");
        assert!(!artwork::cache_path(&art, &t.id).exists());
    }

    #[test]
    fn a_damaged_cached_cover_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let (f, art) = (dir.path().join("song.flac"), dir.path().join("artwork"));
        flac_with_art(&f, Some(90));
        let id = read_track(&f, &art).id;
        let jpg = artwork::cache_path(&art, &id);
        let whole = std::fs::read(&jpg).unwrap();
        std::fs::write(&jpg, &whole[..whole.len() / 2]).unwrap(); // cut short
        assert!(!artwork::is_whole_jpeg(&jpg));
        let t = read_track(&f, &art);
        assert!(t.has_artwork);
        assert!(artwork::is_whole_jpeg(&jpg), "rewritten");
        assert!(cover_shade(&art, &id).unwrap().abs_diff(90) < 4);
    }

    /// The cache failing to store a cover says nothing about the file: it
    /// is not recorded as having no art, and the next scan stores it once
    /// the disk is usable. On a replacement, the old cover stays meanwhile.
    #[test]
    fn a_cover_the_cache_could_not_store_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let (f, art) = (dir.path().join("song.flac"), dir.path().join("artwork"));
        flac_with_art(&f, Some(128));
        let id = track_id(&f);
        // The temp file's name taken by a directory: the write fails.
        let tmp = artwork::cache_path(&art, &id).with_extension("jpg.tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!read_track(&f, &art).has_artwork, "first time: no cover yet");
        std::fs::remove_dir(&tmp).unwrap();
        assert!(read_track(&f, &art).has_artwork, "stored on the next scan of the unchanged file");
        assert!(cover_shade(&art, &id).unwrap().abs_diff(128) < 4);

        // New art, and the cache fails again: the old cover is kept and
        // shown, and replaced once it can be.
        flac_with_art(&f, Some(220));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(read_track(&f, &art).has_artwork, "the old cover still stands");
        assert!(cover_shade(&art, &id).unwrap().abs_diff(128) < 4);
        std::fs::remove_dir(&tmp).unwrap();
        assert!(read_track(&f, &art).has_artwork);
        assert!(cover_shade(&art, &id).unwrap().abs_diff(220) < 4, "replaced once the disk allows");
    }

    #[test]
    fn art_added_after_a_miss_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let (f, art) = (dir.path().join("song.flac"), dir.path().join("artwork"));
        flac_with_art(&f, None);
        assert!(!read_track(&f, &art).has_artwork);
        flac_with_art(&f, Some(120));
        assert!(read_track(&f, &art).has_artwork);
    }

    /// An unchanged file is not read for art again, with art or without:
    /// neither the cover nor its record is rewritten.
    #[test]
    fn an_unchanged_file_is_not_decoded_again() {
        let dir = tempfile::tempdir().unwrap();
        let art = dir.path().join("artwork");
        let (with, without) = (dir.path().join("a.flac"), dir.path().join("b.flac"));
        flac_with_art(&with, Some(10));
        flac_with_art(&without, None);
        let (a, b) = (read_track(&with, &art), read_track(&without, &art));
        let stamp = |id: &str| art.join(format!("{id}.src"));
        let before = (modified(&artwork::cache_path(&art, &a.id)), modified(&stamp(&a.id)), modified(&stamp(&b.id)));
        assert!(before.0.is_some() && before.1.is_some() && before.2.is_some());
        std::thread::sleep(std::time::Duration::from_millis(20));
        let (a2, b2) = (read_track(&with, &art), read_track(&without, &art));
        assert!(a2.has_artwork && !b2.has_artwork);
        let after = (modified(&artwork::cache_path(&art, &a.id)), modified(&stamp(&a.id)), modified(&stamp(&b.id)));
        assert_eq!(before, after, "nothing rewritten");
    }

    /// A cache from before the records (Electron, 0.2.x) is refreshed once.
    #[test]
    fn an_old_cache_without_records_is_refreshed_once() {
        let dir = tempfile::tempdir().unwrap();
        let (f, art) = (dir.path().join("song.flac"), dir.path().join("artwork"));
        flac_with_art(&f, Some(70));
        let id = track_id(&f);
        std::fs::create_dir_all(&art).unwrap();
        // A stale cover, as an old build left it.
        let old = image::RgbImage::from_pixel(8, 8, image::Rgb([250, 250, 250]));
        old.save(artwork::cache_path(&art, &id)).unwrap();
        assert!(read_track(&f, &art).has_artwork);
        assert!(cover_shade(&art, &id).unwrap().abs_diff(70) < 4);
    }

    #[test]
    fn library_indexes_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.flac");
        std::fs::write(&f, b"").unwrap();
        let lib = Library::new(vec![read_track(&f, dir.path())]);
        assert!(lib.get(&track_id(&f)).is_some());
        assert!(lib.get("nope").is_none());
    }
}
