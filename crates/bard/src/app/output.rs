//! Restoring the saved output device at startup.

use super::AppEvent;
use bard_audio::Player;
use bard_core::types::Config;
use bard_core::{Bard, electron_device};
use futures::channel::mpsc::UnboundedSender;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Routes playback to the saved device and returns the config as it then
/// stands (the setting is rewritten when it came from Electron).
pub(super) fn restore(
    player: &Arc<Player>,
    bard: &Arc<Mutex<Bard>>,
    tx: &UnboundedSender<AppEvent>,
    mut config: Config,
    config_dir: &str,
) -> anyhow::Result<Config> {
    // A saved device that is gone falls back to the default for this
    // session only, and the setting is kept: a sink can be late to appear at
    // startup, and forgetting the user's choice over that would quietly move
    // music onto the wrong recording track. The engine reports the fallback
    // as an Error event, which becomes the notice.
    //
    // A device missing at startup can appear later (Electron retried on
    // `devicechange`), so a watcher applies it when it does, for as long as
    // it is still the saved choice.
    //
    // A device saved by the Electron app is named by Chromium's hashed id,
    // which is recomputed from each sink's name to find it (see
    // `electron_device`); once found, the setting is rewritten to the name.
    if config.audio.output_device_id != "default" {
        let saved = config.audio.output_device_id.clone();
        let salt =
            electron_device::is_chromium_id(&saved).then(|| electron_device::salt(Path::new(config_dir))).flatten();
        let find = move |player: &Player| -> Option<String> {
            let outputs = player.outputs();
            let ids = outputs.iter().map(|o| o.id.as_str());
            match &salt {
                Some(salt) => electron_device::resolve(&saved, salt, ids).map(str::to_string),
                None => ids.into_iter().find(|id| *id == saved).map(str::to_string),
            }
        };
        let found = find(player);
        if let Some(id) = found.as_ref().filter(|id| **id != config.audio.output_device_id) {
            eprintln!("[bard] output device: carried over the Electron setting as {id}");
            config = bard.lock().expect("core lock").update_config(|c| c.audio.output_device_id = id.clone());
        }
        let want = found.unwrap_or_else(|| config.audio.output_device_id.clone());
        if player.set_output(&want).is_err() {
            let (player, bard, tx) = (player.clone(), bard.clone(), tx.clone());
            std::thread::Builder::new().name("bard-output-watch".into()).spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    if bard.lock().map(|b| b.config().audio.output_device_id != want).unwrap_or(true) {
                        break;
                    }
                    if let Some(id) = find(&player) {
                        let _ = tx.unbounded_send(AppEvent::OutputAppeared { saved: want, id });
                        break;
                    }
                }
            })?;
        }
    }
    Ok(config)
}
