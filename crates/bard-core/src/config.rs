//! config.json. A port of `src/main/config.ts` and the parts of electron-store
//! it relied on.
//!
//! Same file, same shape: tab-indented, keys in DEFAULT_CONFIG order, no
//! trailing newline, written atomically. Reading fills in anything an older
//! version did not write (every field has a default), and anything this
//! version no longer has is dropped.

use crate::types::Config;
use crate::util::{Indent, to_js_json, write_atomic};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) struct ConfigStore {
    path: PathBuf,
    config: Config,
}

impl ConfigStore {
    /// Loads config.json, rewriting it once if it needs it.
    ///
    /// Two cases rewrite at startup, as the Electron build did:
    /// - the file is missing, or lacks a top-level section (electron-store
    ///   wrote its defaults in at construction), and
    /// - the file carries settings this version no longer has. Without this a
    ///   removed setting sits in the file until some unrelated change happens
    ///   to trigger a write, which for the Twitch and request-rule blocks meant
    ///   months after the feature was deleted.
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let raw: Option<Value> = match std::fs::read_to_string(&path) {
            Ok(body) => Some(serde_json::from_str(&body).map_err(|e| {
                // electron-store refuses to start on a corrupt file as well;
                // overwriting it with defaults would lose every setting.
                anyhow::anyhow!("{} is not valid JSON: {e}", path.display())
            })?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let config = raw.as_ref().map(parse_lenient).unwrap_or_default();
        let store = ConfigStore { path, config };
        if raw.as_ref().is_none_or(needs_rewrite) {
            store.write()?;
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> &Config {
        &self.config
    }

    /// Replaces the config and writes it. Returns whether anything changed.
    pub fn set(&mut self, next: Config) -> anyhow::Result<bool> {
        if next == self.config {
            return Ok(false);
        }
        self.config = next;
        self.write()?;
        Ok(true)
    }

    fn write(&self) -> anyhow::Result<()> {
        write_atomic(&self.path, &to_js_json(&self.config, Indent::Tab)?)?;
        Ok(())
    }
}

/// Reads a config value, section by section.
///
/// A section with a value of the wrong type falls back to its defaults rather
/// than failing the whole file -- electron-store would have carried the bad
/// value along, but the app could not have used it either.
fn parse_lenient(raw: &Value) -> Config {
    if let Ok(c) = serde_json::from_value::<Config>(raw.clone()) {
        return c;
    }
    let mut out = serde_json::to_value(Config::default()).expect("Config serialises");
    if let (Some(dst), Some(src)) = (out.as_object_mut(), raw.as_object()) {
        for (key, slot) in dst.iter_mut() {
            let Some(v) = src.get(key) else { continue };
            let mut probe = serde_json::to_value(Config::default()).expect("Config serialises");
            probe[key] = v.clone();
            if serde_json::from_value::<Config>(probe).is_ok() {
                *slot = v.clone();
            }
        }
    }
    serde_json::from_value(out).unwrap_or_default()
}

fn needs_rewrite(raw: &Value) -> bool {
    let shape = serde_json::to_value(Config::default()).expect("Config serialises");
    let Some(obj) = raw.as_object() else { return true };
    let missing_section = shape.as_object().unwrap().keys().any(|k| !obj.contains_key(k));
    missing_section || prune(&shape, raw) != *raw
}

/// Reshapes `value` to the keys of `shape`, recursively.
///
/// Arrays and primitives are taken wholesale -- only object keys are filtered,
/// and only where the shape says an object belongs.
fn prune(shape: &Value, value: &Value) -> Value {
    match (shape, value) {
        (Value::Object(s), Value::Object(v)) => {
            Value::Object(s.iter().filter_map(|(k, sv)| v.get(k).map(|vv| (k.clone(), prune(sv, vv)))).collect())
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real ~/.config/bard/config.json, byte for byte.
    const REAL: &str = "{\n\t\"libraryPaths\": [\n\t\t\"/home/user/Music\"\n\t],\n\t\"audio\": {\n\t\t\"volume\": 1,\n\t\t\"outputDeviceId\": \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",\n\t\t\"normalize\": \"track\",\n\t\t\"crossfadeSec\": 0\n\t},\n\t\"overlay\": {\n\t\t\"port\": 4747,\n\t\t\"writeNowPlayingJson\": true\n\t},\n\t\"shortcuts\": {\n\t\t\"globalMediaKeys\": false\n\t},\n\t\"window\": {\n\t\t\"closeToTray\": true,\n\t\t\"trayPopupCorner\": \"bottom-right\",\n\t\t\"trayPopupSeconds\": 8\n\t},\n\t\"playback\": {\n\t\t\"autoFill\": \"shuffle\",\n\t\t\"repeat\": \"off\",\n\t\t\"avoidRepeat\": 50,\n\t\t\"fillCount\": 10,\n\t\t\"maxQueueLength\": 200,\n\t\t\"endless\": true,\n\t\t\"endlessLookahead\": 10,\n\t\t\"excludeClaimed\": true,\n\t\t\"lovedWeight\": 3\n\t},\n\t\"streamer\": {\n\t\t\"enabled\": true,\n\t\t\"autoArm\": true,\n\t\t\"obsUrl\": \"ws://127.0.0.1:4455\",\n\t\t\"hasObsPassword\": true\n\t}\n}";

    #[test]
    fn a_current_file_loads_unchanged_and_round_trips_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, REAL).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let store = ConfigStore::open(path.clone()).unwrap();
        assert_eq!(store.get().library_paths, ["/home/user/Music"]);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before, "not rewritten");
        assert_eq!(to_js_json(store.get(), Indent::Tab).unwrap(), REAL);
    }

    #[test]
    fn a_missing_file_is_written_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.json");
        let store = ConfigStore::open(path.clone()).unwrap();
        assert_eq!(*store.get(), Config::default());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("{\n\t\"libraryPaths\": [],\n\t\"audio\": {\n\t\t\"volume\": 0.8,"));
    }

    #[test]
    fn removed_settings_are_pruned_and_missing_ones_filled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let old = REAL.replace(
            "\t\"streamer\": {",
            "\t\"twitch\": { \"channel\": \"x\" },\n\t\"streamer\": {\n\t\t\"requestRules\": 1,",
        );
        std::fs::write(&path, &old).unwrap();
        let store = ConfigStore::open(path.clone()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), REAL);
        assert!(store.get().streamer.enabled);

        // An older file without some keys: filled in on read, and not
        // rewritten just for that (only whole sections trigger a write).
        std::fs::write(&path, r#"{"libraryPaths":["/m"],"audio":{"volume":0.5},"overlay":{},"shortcuts":{},"window":{},"playback":{},"streamer":{}}"#).unwrap();
        let store = ConfigStore::open(path.clone()).unwrap();
        assert_eq!(store.get().audio.volume, 0.5);
        assert_eq!(store.get().audio.normalize, crate::types::NormalizeMode::Track);
        assert_eq!(store.get().playback.endless_lookahead, 10);
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("{\"libraryPaths\""));
    }

    #[test]
    fn a_bad_section_falls_back_without_losing_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let bad = REAL.replace("\"volume\": 1", "\"volume\": \"loud\"");
        std::fs::write(&path, bad).unwrap();
        let store = ConfigStore::open(path).unwrap();
        assert_eq!(store.get().audio.volume, 0.8);
        assert_eq!(store.get().library_paths, ["/home/user/Music"]);
        assert!(store.get().streamer.has_obs_password);
    }

    #[test]
    fn corrupt_json_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ nope").unwrap();
        assert!(ConfigStore::open(path.clone()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ nope");
    }

    #[test]
    fn set_writes_only_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = ConfigStore::open(path.clone()).unwrap();
        let mut next = store.get().clone();
        assert!(!store.set(next.clone()).unwrap());
        next.audio.volume = 1.0;
        assert!(store.set(next).unwrap());
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"volume\": 1,"));
    }
}
