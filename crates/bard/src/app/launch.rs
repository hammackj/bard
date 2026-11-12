//! How this process was started, and the single-instance check.

use bard_services::ServicesOptions;
use std::path::PathBuf;

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

    /// Single instance, as Electron's `requestSingleInstanceLock`: when a
    /// Bard is already running, ask it to show its window and report true, so
    /// this launch can exit before it opens the library -- two processes
    /// writing the same stats and queue would lose one's writes.
    ///
    /// The running instance is found by its MPRIS name, which it owns for as
    /// long as it runs, and raised with MPRIS `Raise` -- the same request the
    /// desktop's media controls make.
    pub fn raise_running(&self) -> bool {
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
