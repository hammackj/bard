//! Bard's core: library, queue, config and the persisted stores.
//!
//! Everything the Electron main process owned, with no UI and no audio. The
//! GPUI app drives it; the audio engine reports back to it.

pub mod deck;
pub mod gain;
pub mod history;
pub mod paths;
pub mod playlist;
pub mod progress;
pub mod search;
pub mod types;
pub mod util;

pub use gain::{gain_db_for, gain_multiplier};
