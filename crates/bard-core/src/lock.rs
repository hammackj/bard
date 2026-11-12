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
//!
//! No lock, no start: a filesystem that cannot lock (ENOLCK, EOPNOTSUPP --
//! some network homes) is an error, not a pass. Running unlocked there would
//! admit every launch as a writer, which is the data loss this exists to
//! stop. `BARD_DATA_DIR` on a local disk is the way round it.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// Held for as long as this process may write the data directory.
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
    path: PathBuf,
}

impl DataDirLock {
    /// Takes the lock for `data_dir`, creating the directory if needed.
    /// `Ok(None)` when another process holds it; an error -- never a lock --
    /// when it cannot be taken at all.
    pub fn acquire(data_dir: &Path) -> std::io::Result<Option<DataDirLock>> {
        Self::acquire_with(data_dir, |f| f.try_lock())
    }

    fn acquire_with(
        data_dir: &Path,
        try_lock: impl FnOnce(&File) -> Result<(), TryLockError>,
    ) -> std::io::Result<Option<DataDirLock>> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("bard.lock");
        // Never truncated or written: the lock is the point, not the contents.
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        match try_lock(&file) {
            Ok(()) => Ok(Some(DataDirLock { _file: file, path })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(std::io::Error::new(
                e.kind(),
                format!(
                    "{} cannot be locked ({e}); another Bard could be writing the same data. \
                     Set BARD_DATA_DIR to a folder on a local disk to run",
                    path.display()
                ),
            )),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A filesystem that cannot lock, for whatever reason, admits no one --
    /// not even the first -- rather than everyone.
    #[test]
    fn a_lock_that_cannot_be_taken_admits_no_writer() {
        let dir = tempfile::tempdir().unwrap();
        for errno in [37 /* ENOLCK */, 95 /* EOPNOTSUPP */, 5 /* EIO */] {
            let fail = || Err(TryLockError::Error(std::io::Error::from_raw_os_error(errno)));
            let first = DataDirLock::acquire_with(dir.path(), |_| fail());
            let second = DataDirLock::acquire_with(dir.path(), |_| fail());
            let e = first.expect_err("no writer");
            assert!(second.is_err());
            assert_eq!(e.raw_os_error(), None, "wrapped with the way round it");
            assert!(e.to_string().contains("BARD_DATA_DIR"), "{e}");
        }
        assert!(DataDirLock::acquire(dir.path()).unwrap().is_some(), "a working lock still admits one");
    }
}
