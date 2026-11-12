//! Read-only checks against the developer's real install and library.
//!
//! Ignored by default -- they need ~/.config/bard and ~/Music. Run with
//! `cargo test -p bard-core --test real_install -- --ignored`. Nothing here
//! writes to the real data directory: scans cache covers into a temp dir.

use bard_core::library::{find_audio_files, scan, track_id};
use bard_core::types::{AlbumFlags, Config};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

fn data_dir() -> PathBuf {
    dirs::config_dir().unwrap().join("bard")
}

fn config() -> Config {
    let raw = std::fs::read_to_string(data_dir().join("config.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
#[ignore]
fn ids_match_the_real_stats_and_artwork_cache() {
    let roots = config().library_paths;
    let files = find_audio_files(&roots);
    assert!(!files.is_empty());
    let ids: HashSet<String> = files.iter().map(track_id).collect();
    assert_eq!(ids.len(), files.len(), "ids collide");

    let stats: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir().join("stats.json")).unwrap())
            .unwrap();
    let stat_ids: Vec<&String> = stats["tracks"].as_object().unwrap().keys().collect();
    let known = stat_ids.iter().filter(|id| ids.contains(id.as_str())).count();
    println!("stats.json: {known}/{} ids match a file in the library", stat_ids.len());
    assert_eq!(known, stat_ids.len());

    let art: Vec<String> = std::fs::read_dir(data_dir().join("artwork"))
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".jpg").map(str::to_string))
        .collect();
    let art_known = art.iter().filter(|id| ids.contains(id.as_str())).count();
    println!("artwork/: {art_known}/{} covers match a file in the library", art.len());
    assert_eq!(art_known, art.len());
}

#[test]
#[ignore]
fn scan_reads_the_real_library() {
    let roots = config().library_paths;
    let tmp = tempfile::tempdir().unwrap();
    let tracks = scan(&roots, tmp.path(), &|_| {});
    assert!(!tracks.is_empty());
    let flags: AlbumFlags = serde_json::from_value(
        serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(data_dir().join("albums.json")).unwrap(),
        )
        .unwrap()["albums"]
            .clone(),
    )
    .unwrap();
    let albums: HashSet<&str> = tracks.iter().map(|t| t.album.as_str()).collect();
    for name in flags.keys() {
        assert!(albums.contains(name.as_str()), "albums.json names {name}, not in the scan");
    }

    // Cached covers come out the size the Electron build wrote.
    let with_art = tracks.iter().filter(|t| t.has_artwork).count();
    println!("{} tracks, {with_art} with covers", tracks.len());
    for t in tracks.iter().filter(|t| t.has_artwork).take(3) {
        let ours = image::image_dimensions(tmp.path().join(format!("{}.jpg", t.id))).unwrap();
        let theirs =
            image::image_dimensions(data_dir().join("artwork").join(format!("{}.jpg", t.id)))
                .unwrap();
        assert_eq!(ours, theirs, "{}", t.path);
    }

    // When a music-metadata dump is supplied (see the port notes), compare
    // every field the Electron scan produced, and its album order.
    let Ok(mm) = std::env::var("BARD_MM_JSON") else { return };
    let reference: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(mm).unwrap()).unwrap();
    let ours: HashMap<&str, _> = tracks.iter().map(|t| (t.id.as_str(), t)).collect();
    assert_eq!(reference.len(), tracks.len());
    for (i, r) in reference.iter().enumerate() {
        let id = r["id"].as_str().unwrap();
        let t = ours[id];
        assert_eq!(tracks[i].id, id, "album order differs at {i}");
        assert_eq!(t.path, r["path"].as_str().unwrap());
        assert_eq!(Some(t.title.as_str()), r["title"].as_str());
        assert_eq!(Some(t.artist.as_str()), r["artist"].as_str());
        assert_eq!(Some(t.album.as_str()), r["album"].as_str());
        assert_eq!(t.duration_sec, r["durationSec"].as_f64().unwrap(), "{}", t.path);
        assert_eq!(t.track_gain_db, r["trackGainDb"].as_f64(), "{}", t.path);
        assert_eq!(t.album_gain_db, r["albumGainDb"].as_f64(), "{}", t.path);
        assert_eq!(t.track_peak, r["trackPeak"].as_f64(), "{}", t.path);
        assert_eq!(t.album_peak, r["albumPeak"].as_f64(), "{}", t.path);
        assert_eq!(t.has_artwork, r["picLen"].as_u64().unwrap() > 0, "{}", t.path);
    }
}

#[test]
#[ignore]
fn a_copy_of_the_real_data_dir_opens_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    for f in ["config.json", "stats.json", "albums.json", "loved.json"] {
        let src = data_dir().join(f);
        if src.exists() {
            std::fs::copy(&src, tmp.path().join(f)).unwrap();
        }
    }
    let before = std::fs::read(tmp.path().join("config.json")).unwrap();
    let (mut bard, _rx) = bard_core::Bard::open(tmp.path().to_path_buf()).unwrap();
    assert_eq!(std::fs::read(tmp.path().join("config.json")).unwrap(), before, "config rewritten");
    assert_eq!(bard.config(), &config());
    println!(
        "{} stats, {} album flags, {} loved",
        bard.stats().len(),
        bard.album_flags().len(),
        bard.loved().len()
    );
    assert!(!bard.stats().is_empty());

    // Writing loved.json back unchanged reproduces the Electron file exactly.
    bard.toggle_loved("x");
    bard.toggle_loved("x");
    bard.flush();
    let loved_after = std::fs::read_to_string(tmp.path().join("loved.json")).unwrap();
    let loved_before = std::fs::read_to_string(data_dir().join("loved.json")).unwrap_or_default();
    if !loved_before.is_empty() {
        assert_eq!(loved_after, loved_before);
    }
    bard.shutdown();
}
