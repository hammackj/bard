//! The policy half of streamer mode: when to arm, and what happens then.
//! A port of `src/main/streamer.ts`.
//!
//! Kept apart from the OBS connection (another crate), which only knows how to
//! talk to a websocket, and from the queue, which only knows how to enforce.
//! This is the part that decides.
//!
//! Streamer mode: while armed, only albums flagged `safe` may play. Stricter
//! than `playback.excludeClaimed` in two ways. It is an allow-list rather than
//! a block-list, so `untested` is withheld too -- an album nobody has checked
//! is not evidence of safety. And it applies to deliberate play as well as
//! auto-selection, because the risk it guards against does not care whether a
//! human picked the track.

use crate::CoreEvent;
use crate::bard::Bard;
use crate::types::StreamerSource;

impl Bard {
    /// Armed means: only `safe` plays, whoever asked for it.
    pub fn is_streamer_armed(&self) -> bool {
        self.config.get().streamer.enabled
    }

    /// What armed streamer mode; None when armed by hand (or not armed).
    pub fn streamer_source(&self) -> Option<StreamerSource> {
        self.streamer_source
    }

    pub(crate) fn set_streamer_source(&mut self, next: Option<StreamerSource>) {
        if next == self.streamer_source {
            return;
        }
        self.streamer_source = next;
        self.emit(CoreEvent::StreamerSource(next));
    }

    /// Arms streamer mode.
    ///
    /// `Some(source)` is the OBS watcher reporting an output went live.
    /// It is re-announced even when already armed -- switching from recording
    /// to streaming should update what the banner says -- and arming says so
    /// with a notice, because the config changed without the user touching it.
    ///
    /// One direction only. OBS reporting "not live" is deliberately not wired
    /// to anything: a stream ending, a websocket dropping or OBS quitting must
    /// never silently remove protection, and the user may well still be
    /// recording something the watcher cannot see. Disarming stays a
    /// deliberate act ([`Bard::disarm`]).
    ///
    /// `None` is arming by hand: the same as the settings toggle, which clears
    /// any OBS source (there is no output to name) and applies the mode to the
    /// queue and the track on air.
    pub fn arm(&mut self, source: Option<StreamerSource>) {
        match source {
            Some(src) => {
                self.set_streamer_source(Some(src));
                if self.is_streamer_armed() {
                    return;
                }
                let mut next = self.config.get().clone();
                next.streamer.enabled = true;
                // Sends ConfigChanged: the toggle would otherwise keep reading
                // "off" while the mode was enforcing.
                self.write_config(next);
                self.emit(CoreEvent::Notice(
                    "OBS went live \u{2014} streamer mode armed: only albums marked safe will play".into(),
                ));
                self.refresh_streamer_mode();
            }
            None => self.set_armed_by_hand(true),
        }
    }

    /// Disarms streamer mode. Always deliberate; nothing calls this on its own.
    pub fn disarm(&mut self) {
        self.set_armed_by_hand(false);
    }

    /// `setConfig({ streamer: { enabled } })` from the settings toggle.
    fn set_armed_by_hand(&mut self, enabled: bool) {
        let mut next = self.config.get().clone();
        next.streamer.enabled = enabled;
        self.write_config(next);
        // Turning it off by hand clears what armed it; turning it on by hand
        // means there is no OBS output to name. Arming has to reach the queue
        // and the track on air, not just future picks.
        self.set_streamer_source(None);
        self.refresh_streamer_mode();
    }
}
