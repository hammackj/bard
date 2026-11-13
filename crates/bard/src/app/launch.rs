//! How this process was started, and the single-instance check.

use bard_core::DataDirLock;
use bard_services::ServicesOptions;
use std::path::PathBuf;

/// What this launch does.
#[derive(Debug)]
pub enum Start {
    /// This process owns the data directory: run, holding the lock for life.
    Run(DataDirLock),
    /// Another Bard owns it. `raised` is whether its window was asked to
    /// come forward; either way this one exits.
    AlreadyRunning { raised: bool },
}

/// The decision, apart from the bus and the file system.
///
/// The lock decides who may write; raising is only a courtesy to the user, so
/// a failed raise -- no session bus, a hung instance -- still exits. The one
/// case where holding the lock is not enough is a Bard from before the lock
/// (0.2.4 and earlier) still running: it holds no lock, but it does own the
/// MPRIS name, so a raise that succeeds means it is there.
fn decide(lock: Option<DataDirLock>, raise: impl FnOnce() -> bool) -> Start {
    match lock {
        None => Start::AlreadyRunning { raised: raise() },
        Some(lock) => {
            if raise() {
                Start::AlreadyRunning { raised: true }
            } else {
                Start::Run(lock)
            }
        }
    }
}

pub struct Launch {
    pub data_dir: PathBuf,
    pub silent: bool,
    /// Running on a scratch data dir (`BARD_DATA_DIR`): the services must not
    /// touch what the installed Bard owns -- its keyring slot, the Electron
    /// password file, OBS -- and take their own MPRIS name.
    pub dev: bool,
}

impl Launch {
    pub fn from_env() -> Self {
        let custom = std::env::var_os("BARD_DATA_DIR");
        Self {
            dev: custom.is_some(),
            data_dir: custom.map(PathBuf::from).unwrap_or_else(bard_core::default_data_dir),
            silent: std::env::var_os("BARD_SILENT").is_some(),
        }
    }

    /// The MPRIS name this instance owns, and a second launch looks for.
    #[cfg(target_os = "linux")]
    /// Dev runs take one per data dir: one instance per set of files is what
    /// single-instance protects, and two scratch copies may run side by side.
    fn mpris_suffix(&self) -> String {
        if !self.dev {
            return bard_services::mpris::BUS_SUFFIX.into();
        }
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.data_dir.hash(&mut h);
        // A bus name element may not start with a digit.
        format!("bard.dev_{:08x}", h.finish() as u32)
    }

    /// Single instance, as Electron's `requestSingleInstanceLock`: one Bard
    /// per data directory, decided by [`DataDirLock`] before the library is
    /// opened or audio started -- two processes writing the same stats and
    /// queue would lose one's writes. A second launch raises the first.
    pub fn start(&self) -> std::io::Result<Start> {
        Ok(decide(DataDirLock::acquire(&self.data_dir)?, || self.raise_running()))
    }

    /// Asks a running Bard to show its window; true if one was there and
    /// took the request.
    ///
    /// The running instance is found by its MPRIS name, which it owns for as
    /// long as it runs, and raised with MPRIS `Raise` -- the same request the
    /// desktop's media controls make.
    ///
    /// MPRIS is D-Bus, so on macOS and Windows there is nothing to ask: the
    /// lock alone keeps a second copy from running.
    #[cfg(not(target_os = "linux"))]
    fn raise_running(&self) -> bool {
        false
    }

    #[cfg(target_os = "linux")]
    fn raise_running(&self) -> bool {
        let name = format!("org.mpris.MediaPlayer2.{}", self.mpris_suffix());
        let raise = || -> zbus::Result<bool> {
            let conn = zbus::blocking::Connection::session()?;
            let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
            let bus_name = zbus::names::BusName::try_from(name.as_str())?;
            if !dbus.name_has_owner(bus_name.clone())? {
                return Ok(false);
            }
            // The launcher gave this process an activation token; MPRIS Raise
            // cannot carry it, so it goes to the running instance's tray item,
            // as a tray click's would, and the window comes to the front.
            if let Some(token) = std::env::var("XDG_ACTIVATION_TOKEN").ok().filter(|t| !t.is_empty()) {
                let pid = dbus.get_connection_unix_process_id(bus_name)?;
                let tray = format!("org.kde.StatusNotifierItem-{pid}-1");
                let sni = Some("org.kde.StatusNotifierItem");
                let raised = conn
                    .call_method(
                        Some(tray.as_str()),
                        "/StatusNotifierItem",
                        sni,
                        "ProvideXdgActivationToken",
                        &(token,),
                    )
                    .and_then(|_| {
                        conn.call_method(Some(tray.as_str()), "/StatusNotifierItem", sni, "Activate", &(0i32, 0i32))
                    });
                if raised.is_ok() {
                    return Ok(true);
                }
            }
            conn.call_method(
                Some(name.as_str()),
                "/org/mpris/MediaPlayer2",
                Some("org.mpris.MediaPlayer2"),
                "Raise",
                &(),
            )?;
            Ok(true)
        };
        // No session bus, or the call failed: carry on and start normally.
        raise().unwrap_or(false)
    }

    pub(super) fn services(&self) -> ServicesOptions {
        if self.dev {
            ServicesOptions {
                #[cfg(target_os = "linux")]
                mpris_bus_suffix: self.mpris_suffix(),
                obs_password: bard_services::secrets::ObsPassword::with_service("bard-dev"),
                migrate_obs_password: false,
                obs: std::env::var_os("BARD_OBS").is_some(),
                ..Default::default()
            }
        } else {
            ServicesOptions::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held() -> (tempfile::TempDir, DataDirLock) {
        let dir = tempfile::tempdir().unwrap();
        let lock = DataDirLock::acquire(dir.path()).unwrap().unwrap();
        (dir, lock)
    }

    #[test]
    fn without_the_lock_this_launch_exits_whether_or_not_the_raise_works() {
        // Raised; and no bus, a failed Raise, a hung instance: still exits.
        assert!(matches!(decide(None, || true), Start::AlreadyRunning { raised: true }));
        assert!(matches!(decide(None, || false), Start::AlreadyRunning { raised: false }));
    }

    #[test]
    fn with_the_lock_this_launch_runs() {
        let (_d, lock) = held();
        assert!(matches!(decide(Some(lock), || false), Start::Run(_)));
    }

    /// A pre-lock Bard owns the MPRIS name but no lock.
    #[test]
    fn an_older_bard_without_the_lock_is_raised_not_joined() {
        let (_d, lock) = held();
        assert!(matches!(decide(Some(lock), || true), Start::AlreadyRunning { raised: true }));
    }

    #[test]
    fn a_launch_against_a_held_directory_does_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let _first = DataDirLock::acquire(dir.path()).unwrap().unwrap();
        let launch = Launch { data_dir: dir.path().to_path_buf(), silent: true, dev: true };
        assert!(matches!(launch.start().unwrap(), Start::AlreadyRunning { .. }));
    }
}
