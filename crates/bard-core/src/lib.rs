//! Bard's core: library, queue, config and the persisted stores.
//!
//! Everything the Electron main process owned, with no UI and no audio. The
//! GPUI app drives it; the audio engine reports back to it.

pub mod types;
