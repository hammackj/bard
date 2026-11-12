//! System-wide media keys, and what `shortcuts.globalMediaKeys` means now.
//! Replaces `src/main/shortcuts.ts`.
//!
//! # Electron
//!
//! The Electron build registered MediaPlayPause / MediaNextTrack /
//! MediaPreviousTrack with `globalShortcut`, only when the user opted in:
//! once taken, the keys stopped reaching every other player on the machine,
//! so grabbing them silently at first launch would have been hostile.
//!
//! # KDE Plasma on Wayland
//!
//! There is nothing to grab. Wayland gives clients no global key grabs, and
//! Plasma already binds the media keys itself (kglobalaccel's "Media
//! Controller" component) and forwards them to the active MPRIS player --
//! which is Bard whenever Bard is the player in use, through
//! [`crate::mpris`]. That is both what the user expects and the polite
//! behaviour the opt-in existed to protect: the keys go to whichever player
//! is active, never captured from the others.
//!
//! So **the setting grabs nothing in either position**. Media keys reach Bard
//! through MPRIS, which is always on (it is also what the lock screen and the
//! panel's media controls read). `globalMediaKeys` stays in config.json for
//! compatibility with the Electron build sharing it, and defaults to off; the
//! settings UI should describe it with [`describe`] rather than imply a grab.
//!
//! Re-binding keys exclusively to Bard would mean registering through the
//! XDG GlobalShortcuts portal, which on Plasma conflicts with the Media
//! Controller's own bindings; not done.

/// How media keys reach Bard on this desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKeyRouting {
    /// The desktop sends them to the active MPRIS player; nothing is grabbed.
    ViaMpris,
}

/// What `shortcuts.globalMediaKeys` does here: nothing is grabbed whatever
/// its value, so this ignores it. Kept as a function so a future desktop
/// that does need a grab has one place to change.
pub fn routing(_global_media_keys: bool) -> MediaKeyRouting {
    MediaKeyRouting::ViaMpris
}

/// A sentence for the settings UI.
pub fn describe(routing: MediaKeyRouting) -> &'static str {
    match routing {
        MediaKeyRouting::ViaMpris => {
            "Media keys reach Bard through the desktop's media controls (MPRIS) whenever Bard is the active player; no keys are grabbed from other players."
        }
    }
}
