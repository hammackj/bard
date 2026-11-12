//! The obs-websocket password, kept out of config.json. A port of
//! `src/main/secrets.ts`.
//!
//! config.json is plain text the user is invited to open -- Settings shows its
//! path and offers a button to reveal it -- so a credential does not belong
//! there, even a localhost one. This goes through the Secret Service (KWallet
//! / ksecretd on Plasma, gnome-keyring elsewhere) via the `keyring` crate.
//!
//! Where there is none, [`available`] is false and we refuse to store rather
//! than quietly writing plaintext under a name that implies otherwise. Losing
//! the convenience of auto-arm is a smaller harm than a file that looks
//! encrypted and is not.
//!
//! # Blocking
//!
//! Every call here is a blocking D-Bus round trip, and may wait on a keyring
//! unlock prompt. zbus runs these on its own runtime, which **panics if called
//! from inside a tokio runtime** (including `spawn_blocking` threads), so call
//! them from a plain thread -- [`Services::set_obs_password`] does that for the
//! app. The OBS watcher reads the password through [`get_on_thread`].
//!
//! # Migrating the Electron build's password
//!
//! The Electron build kept the password in `<data dir>/obs-password.bin`,
//! encrypted by Chromium's OSCrypt (`safeStorage`). [`migrate_electron`]
//! decrypts it once and moves it into the keyring. See that function for the
//! format and where its key lives.
//!
//! [`Services::set_obs_password`]: crate::Services::set_obs_password

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The keyring entry: service and user.
pub const SERVICE: &str = "bard";
pub const USER: &str = "obs-password";

/// The Electron build's file, relative to the data directory.
pub const ELECTRON_FILE: &str = "obs-password.bin";
/// Written next to it once the migration has run, so a password the user
/// later clears is not resurrected from the old file on the next launch.
pub const MIGRATED_MARKER: &str = "obs-password.bin.migrated";

/// A keyring slot. [`ObsPassword::default`] is the real one; tests use their
/// own service name.
#[derive(Debug, Clone)]
pub struct ObsPassword {
    service: String,
}

impl Default for ObsPassword {
    fn default() -> Self {
        ObsPassword { service: SERVICE.into() }
    }
}

/// Whether there is a Secret Service to store into.
pub fn available() -> bool {
    keyring::Entry::store_status().is_ok()
}

impl ObsPassword {
    pub fn with_service(service: impl Into<String>) -> Self {
        ObsPassword { service: service.into() }
    }

    fn entry(&self) -> Option<keyring::Entry> {
        keyring::Entry::new(&self.service, USER).ok()
    }

    /// True when a password is stored. Never returns the password itself.
    pub fn has(&self) -> bool {
        !self.get().is_empty()
    }

    /// Some(true/false) when the keyring answered; None when it could not be
    /// asked (unavailable, locked, refused) -- so a flag is not cleared just
    /// because the keyring was briefly out of reach.
    pub fn stored(&self) -> Option<bool> {
        match self.entry()?.get_password() {
            Ok(p) => Some(!p.is_empty()),
            Err(keyring::Error::NoEntry) => Some(false),
            Err(_) => None,
        }
    }

    /// The stored password, or "" when absent or unreadable (a keyring that
    /// changed under us, or one that stayed locked).
    pub fn get(&self) -> String {
        self.entry().and_then(|e| e.get_password().ok()).unwrap_or_default()
    }

    /// Stores, or clears when given an empty string.
    ///
    /// Returns false when no Secret Service is available, so the caller can
    /// say why rather than reporting a success that did not happen.
    pub fn set(&self, password: &str) -> bool {
        if password.is_empty() {
            self.clear();
            return true;
        }
        if !available() {
            return false;
        }
        self.entry().is_some_and(|e| e.set_password(password).is_ok())
    }

    /// Removes the stored password. Nothing useful to do on failure; treated
    /// as cleared, as the Electron build did.
    pub fn clear(&self) {
        if let Some(e) = self.entry() {
            let _ = e.delete_credential();
        }
    }
}

/// Reads the password on a fresh OS thread and hands it back to async code.
///
/// The keyring's blocking zbus calls panic inside a tokio runtime, so the OBS
/// watcher (which runs on one) cannot call [`ObsPassword::get`] directly.
pub async fn get_on_thread(slot: ObsPassword) -> String {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(slot.get());
    });
    rx.await.unwrap_or_default()
}

// --- Electron migration ----------------------------------------------------

/// What [`migrate_electron`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// No obs-password.bin, or the migration already ran.
    NothingToDo,
    /// The keyring already held a password; the old file was left alone.
    AlreadyStored,
    /// Decrypted and stored in the keyring.
    Migrated,
    /// The file could not be decrypted (no key in the keyring, a locked
    /// keyring, a file from another machine). Retried on the next launch.
    Failed(String),
}

/// Moves the Electron build's password into the keyring, once.
///
/// # The format
///
/// Chromium OSCrypt on Linux, which Electron's safeStorage is:
///
/// - `"v11"` + AES-128-CBC ciphertext, PKCS#7 padded, IV of 16 spaces, key =
///   PBKDF2-HMAC-SHA1(secret, salt `"saltysalt"`, 1 iteration, 16 bytes).
///   The secret is a random string Chromium keeps in the OS keyring.
/// - `"v10"`: the same with the hard-coded secret `"peanuts"`, used when no
///   keyring was reachable.
///
/// # Where the v11 secret lives
///
/// On Plasma the Electron build used the KWallet backend, whose secret sits in
/// folder "Chromium Keys" under the key **"Chromium Safe Storage"** -- not
/// "bard Safe Storage": that name is only used by the libsecret backend.
/// ksecretd exposes KWallet entries over the Secret Service with attributes
/// `server = "Chromium Keys"`, `user = "Chromium Safe Storage"`, so one Secret
/// Service lookup covers it. The libsecret (GNOME) location is
/// `xdg:schema = chrome_libsecret_os_crypt_password_v2, application = bard`
/// (or `_v1`); those are tried too.
///
/// Verified against a real install: the KWallet secret decrypts
/// obs-password.bin to a printable 16-character password.
///
/// # Leaves the old file alone
///
/// The Electron build on `master` shares this data directory and still reads
/// obs-password.bin, so it is not deleted. A marker file stops the migration
/// from running again -- otherwise a password cleared in the Rust build would
/// come back from the old file on the next launch.
pub fn migrate_electron(data_dir: &Path, slot: &ObsPassword) -> Migration {
    let file = data_dir.join(ELECTRON_FILE);
    let marker = data_dir.join(MIGRATED_MARKER);
    if marker.exists() || !file.exists() {
        return Migration::NothingToDo;
    }
    if slot.has() {
        mark(&marker);
        return Migration::AlreadyStored;
    }
    let password = match read_electron(&file) {
        Ok(p) => p,
        Err(e) => return Migration::Failed(e),
    };
    if password.is_empty() || !slot.set(&password) {
        return Migration::Failed("could not store the password in the keyring".into());
    }
    mark(&marker);
    Migration::Migrated
}

/// Decrypts the Electron build's obs-password.bin without storing anything.
/// Blocking, like the rest of this module.
pub fn read_electron(file: &Path) -> Result<String, String> {
    let blob = std::fs::read(file).map_err(|e| format!("read {}: {e}", file.display()))?;
    let password = match blob.get(..3) {
        Some(b"v10") => decrypt_safe_storage(&blob, b"peanuts"),
        Some(b"v11") => chromium_safe_storage_secrets()?.iter().find_map(|s| decrypt_safe_storage(&blob, s)),
        _ => return Err("not a Chromium v10/v11 blob".into()),
    };
    password.ok_or_else(|| "no keyring secret decrypts obs-password.bin".into())
}

/// Records that the Electron password should not be migrated any more --
/// called when the user sets or clears the password in this build.
pub fn mark_migrated(data_dir: &Path) {
    if data_dir.join(ELECTRON_FILE).exists() {
        mark(&data_dir.join(MIGRATED_MARKER));
    }
}

fn mark(marker: &PathBuf) {
    let _ =
        std::fs::write(marker, "The OBS password in obs-password.bin was moved to the system keyring by Bard 0.2.\n");
}

/// Decrypts a Chromium OSCrypt `v10` / `v11` blob with the given keyring
/// secret. None when the padding does not check out (the wrong secret, almost
/// always) or the result is not UTF-8.
pub fn decrypt_safe_storage(blob: &[u8], secret: &[u8]) -> Option<String> {
    let body = blob.strip_prefix(b"v11").or_else(|| blob.strip_prefix(b"v10"))?;
    if body.is_empty() || body.len() % 16 != 0 {
        return None;
    }
    let key = pbkdf2::pbkdf2_hmac_array::<sha1::Sha1, 16>(secret, b"saltysalt", 1);
    let iv = [b' '; 16];
    let mut buf = body.to_vec();
    let plain = cbc::Decryptor::<aes::Aes128>::new(&key.into(), &iv.into()).decrypt_padded::<Pkcs7>(&mut buf).ok()?;
    String::from_utf8(plain.to_vec()).ok()
}

/// Every candidate OSCrypt secret the Secret Service holds for this app.
fn chromium_safe_storage_secrets() -> Result<Vec<Vec<u8>>, String> {
    use secret_service::EncryptionType;
    use secret_service::blocking::SecretService;

    let ss = SecretService::connect(EncryptionType::Dh).map_err(|e| format!("secret service: {e}"))?;
    let searches: [&[(&str, &str)]; 3] = [
        // KWallet backend, through ksecretd.
        &[("server", "Chromium Keys"), ("user", "Chromium Safe Storage")],
        // libsecret backend.
        &[("xdg:schema", "chrome_libsecret_os_crypt_password_v2"), ("application", "bard")],
        &[("xdg:schema", "chrome_libsecret_os_crypt_password_v1"), ("application", "bard")],
    ];
    let mut out = Vec::new();
    for attrs in searches {
        let map: HashMap<&str, &str> = attrs.iter().copied().collect();
        let Ok(found) = ss.search_items(map) else { continue };
        for item in found.unlocked.iter().chain(found.locked.iter()) {
            if item.ensure_unlocked().is_err() {
                continue;
            }
            if let Ok(secret) = item.get_secret() {
                out.push(secret);
            }
        }
    }
    if out.is_empty() {
        return Err("no Chromium Safe Storage secret in the keyring".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    // Vectors made with `openssl enc -aes-128-cbc` and Python's
    // hashlib.pbkdf2_hmac, independently of this code.
    #[test]
    fn decrypts_v11_with_the_keyring_secret() {
        let mut blob = b"v11".to_vec();
        blob.extend(hex("126f244ef90d6a8ba45e6f7234b1a2aa"));
        assert_eq!(decrypt_safe_storage(&blob, b"test-secret").as_deref(), Some("hunter2"));
    }

    #[test]
    fn decrypts_v10_with_peanuts() {
        let mut blob = b"v10".to_vec();
        blob.extend(hex("58186cf88abd515a1cd36afe2d4d93ca"));
        assert_eq!(decrypt_safe_storage(&blob, b"peanuts").as_deref(), Some("hunter2"));
    }

    #[test]
    fn wrong_secret_or_garbage_is_none() {
        let mut blob = b"v11".to_vec();
        blob.extend(hex("126f244ef90d6a8ba45e6f7234b1a2aa"));
        // A wrong key can still produce valid padding by chance (1 in ~256);
        // this one does not.
        assert_eq!(decrypt_safe_storage(&blob, b"wrong"), None);
        assert_eq!(decrypt_safe_storage(b"v11", b"x"), None);
        assert_eq!(decrypt_safe_storage(b"v11abc", b"x"), None);
        assert_eq!(decrypt_safe_storage(b"plain text pw", b"x"), None);
    }

    #[test]
    fn migration_skips_without_a_file_or_after_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let slot = ObsPassword::with_service("bard-test-unused");
        assert_eq!(migrate_electron(dir.path(), &slot), Migration::NothingToDo);
        std::fs::write(dir.path().join(ELECTRON_FILE), b"v11....").unwrap();
        std::fs::write(dir.path().join(MIGRATED_MARKER), b"").unwrap();
        assert_eq!(migrate_electron(dir.path(), &slot), Migration::NothingToDo);
    }

    /// Writes to the real Secret Service under a throwaway service name and
    /// removes it again. `cargo test -p bard-services -- --ignored keyring`.
    #[test]
    #[ignore = "touches the session keyring"]
    fn keyring_roundtrip() {
        let slot = ObsPassword::with_service(format!("bard-test-{}", std::process::id()));
        assert!(available());
        assert!(!slot.has());
        assert!(slot.set("correct horse"));
        assert_eq!(slot.get(), "correct horse");
        assert!(slot.set(""));
        assert!(!slot.has());
    }
}
