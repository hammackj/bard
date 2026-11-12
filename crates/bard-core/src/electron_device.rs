//! Output devices saved by the Electron app.
//!
//! Electron stored `config.audio.outputDeviceId` as Chromium's `setSinkId`
//! id: not the sink's name but HMAC-SHA256 of it, keyed by the page's origin
//! (`file://`) and salted with a per-profile value Electron kept in its
//! `Preferences` file, which sits in the same data directory. The Rust app
//! names a device by its PipeWire `node.name`, so a saved Electron id never
//! matches one. With the salt still on disk, the id can be recomputed for each
//! sink and the setting carried over instead of asking the user to pick again.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::path::Path;

/// The origin Electron's renderer was loaded from, the HMAC key.
const ORIGIN: &str = "file://";

/// Whether `id` looks like a Chromium device id: 64 lowercase hex digits.
/// A PipeWire node name never does.
pub fn is_chromium_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The salt from Electron's `Preferences` in `data_dir`, if it is there.
pub fn salt(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(data_dir.join("Preferences")).ok()?;
    let prefs: serde_json::Value = serde_json::from_str(&text).ok()?;
    prefs.pointer("/electron/media/device_id_salt")?.as_str().map(str::to_string)
}

/// The id Chromium gave the sink named `sink`.
pub fn chromium_id(salt: &str, sink: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(ORIGIN.as_bytes()).expect("HMAC takes any key length");
    mac.update(sink.as_bytes());
    mac.update(salt.as_bytes());
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// The sink among `sinks` that Chromium called `id`.
pub fn resolve<'a>(id: &str, salt: &str, sinks: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    sinks.into_iter().find(|s| chromium_id(salt, s) == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The formula was checked against an id a real Electron profile saved;
    // this pair uses a made-up salt, as a profile's salt is its own.
    const SALT: &str = "0123456789ABCDEF0123456789ABCDEF";
    const MUSIC: &str = "32a4d3659630b76b54cf5fbd3f0b501e775ad52f988b3c9653cbe8521cc81436";

    #[test]
    fn recomputes_chromiums_id() {
        assert_eq!(chromium_id(SALT, "Music"), MUSIC);
    }

    #[test]
    fn resolves_among_sinks() {
        assert_eq!(resolve(MUSIC, SALT, ["Game", "Music", "Chat"]), Some("Music"));
        assert_eq!(resolve(MUSIC, SALT, ["Game", "Chat"]), None);
    }

    #[test]
    fn tells_chromium_ids_from_node_names() {
        assert!(is_chromium_id(MUSIC));
        assert!(!is_chromium_id("Music"));
        assert!(!is_chromium_id("default"));
        assert!(!is_chromium_id(&MUSIC.to_uppercase()));
    }

    #[test]
    fn reads_the_salt_from_preferences() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(salt(dir.path()), None);
        std::fs::write(
            dir.path().join("Preferences"),
            format!(r#"{{"electron":{{"media":{{"device_id_salt":"{SALT}"}}}}}}"#),
        )
        .unwrap();
        assert_eq!(salt(dir.path()).as_deref(), Some(SALT));
    }
}
