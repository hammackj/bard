//! One Bard per data directory.
//!
//! Two processes on the same `~/.config/bard` each load the stores, then
//! write them back: whichever flushes last erases the other's plays, loves
//! and flags, and both write through the same `.tmp` names. The single-
//! instance check used to be "is the MPRIS name taken?", which two launches
//! in the same instant can both answer "no", and which a missing session bus
//! answers by letting everyone in.
//!
//! This is an exclusive `flock` on `bard.lock` in the data directory, held for
//! the life of the process:
//! - atomic: of any number of simultaneous launches, exactly one gets it;
//! - released by the kernel when the holder exits or crashes, so there is no
//!   stale lock to clean up;
//! - on the file, not the name, so `~/.config/bard`, a symlink to it and a
//!   path with `..` in it are all the same lock; other directories are not.
//!
//! Raising the running window is a separate courtesy (see the app's launch);
//! failing to raise it never grants a second writer.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// Held for as long as this process may write the data directory.
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
    path: PathBuf,
}

/// The filesystem does not do locks, as opposed to the lock being refused.
fn locks_unsupported(e: &std::io::Error) -> bool {
    const ENOLCK: i32 = 37;
    const EOPNOTSUPP: i32 = 95;
    e.kind() == std::io::ErrorKind::Unsupported || matches!(e.raw_os_error(), Some(ENOLCK | EOPNOTSUPP))
}

impl DataDirLock {
    /// Takes the lock for `data_dir`, creating the directory if needed.
    /// `Ok(None)` when another process holds it.
    pub fn acquire(data_dir: &Path) -> std::io::Result<Option<DataDirLock>> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("bard.lock");
        // Never truncated or written: the lock is the point, not the contents.
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(DataDirLock { _file: file, path })),
            Err(TryLockError::WouldBlock) => Ok(None),
            // A filesystem with no locks at all (some network homes): run as
            // before the lock existed rather than not at all. The window
            // raise over the session bus still stops most second launches.
            Err(TryLockError::Error(e)) if locks_unsupported(&e) => {
                eprintln!("[bard] {} cannot be locked ({e}); running without the single-instance lock", path.display());
                Ok(Some(DataDirLock { _file: file, path }))
            }
            Err(TryLockError::Error(e)) => Err(e),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_filesystem_without_locks_runs_unlocked() {
        assert!(locks_unsupported(&std::io::Error::from_raw_os_error(37)));
        assert!(locks_unsupported(&std::io::Error::from_raw_os_error(95)));
        assert!(locks_unsupported(&std::io::Error::from(std::io::ErrorKind::Unsupported)));
        assert!(!locks_unsupported(&std::io::Error::from_raw_os_error(13)), "permission denied still stops");
        assert!(!locks_unsupported(&std::io::Error::from_raw_os_error(5)), "an I/O error still stops");
    }
}
