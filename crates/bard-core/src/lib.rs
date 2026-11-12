//! Bard's core: library, queue, config and the persisted stores.
//!
//! Everything the Electron main process owned, with no UI and no audio. The
//! GPUI app drives it through [`Bard`]; the audio engine reports back to it
//! with [`Bard::report_progress`] and [`Bard::report_ended`]; everything the
//! core has to say arrives on the [`CoreEvent`] channel returned by
//! [`Bard::open`].
//!
//! # Data compatibility
//!
//! The data directory is `~/.config/bard` ([`default_data_dir`]), shared with
//! the Electron build: config.json (tab-indented, electron-store's layout),
//! stats.json, albums.json, loved.json, nowplaying.json and
//! `artwork/<trackId>.jpg`, all in the shapes the Electron build wrote. Track
//! ids are `sha1(path.resolve(file)).hex[..16]`, verified against a real
//! install (`tests/real_install.rs`). The directory is a parameter of
//! [`Bard::open`] so tests never touch the real one.
//!
//! # API
//!
//! [`Bard`] replaces `BardApi` from `src/shared/ipc.ts`:
//!
//! | BardApi | Bard |
//! |---|---|
//! | `scanLibrary` | [`Bard::scan_library`], or [`Bard::scanner`] + [`Bard::install_library`] |
//! | `addLibraryPath` | [`Bard::add_library_path`] / [`Bard::add_library_paths`] |
//! | `getArtwork` | [`Bard::artwork_path`] |
//! | `enqueue`, `enqueueMany`, `playNow` | [`Bard::enqueue`], [`Bard::enqueue_many`], [`Bard::play_now`] |
//! | `removeFromQueue`, `reorderQueue`, `clearQueue`, `shuffleQueue`, `fillQueue` | same names, snake_case |
//! | `skip`, `previous` | [`Bard::skip`], [`Bard::previous`] |
//! | `reportProgress`, `reportEnded` | [`Bard::report_progress`], [`Bard::report_ended`] |
//! | `getConfig`, `setConfig` | [`Bard::config`], [`Bard::update_config`] |
//! | `getPaths` | [`Bard::paths`] |
//! | `getStreamerSource` | [`Bard::streamer_source`]; arming is [`Bard::arm`] / [`Bard::disarm`] |
//! | `getStats`, `clearStats` | [`Bard::stats`], [`Bard::clear_stats`] |
//! | `getAlbumFlags`, `setAlbumFlag` | [`Bard::album_flags`], [`Bard::set_album_flag`] |
//! | `getLoved`, `toggleLoved` | [`Bard::loved`], [`Bard::toggle_loved`] |
//! | `onX` subscriptions | [`CoreEvent`] variants |
//!
//! Plus what the UI and services need that IPC did not carry:
//! [`Bard::tracks`], [`Bard::track`], [`Bard::search`] (the library filter),
//! [`Bard::best_match`] (match.ts), [`Bard::queue`], [`Bard::now_playing`],
//! [`Bard::now_playing_snapshot`] and [`Bard::set_overlay_port`] for the
//! overlay server, [`Bard::transport`] and [`Bard::notice`] for tray/MPRIS to
//! relay through, [`Bard::can_reveal`], [`Bard::flush`] and [`Bard::shutdown`]
//! (before-quit), and [`gain_multiplier`] for the audio engine.
//!
//! Not here, because other crates own them: `getOverlayUrl` (overlay
//! server), `getAppInfo`, `revealPath` itself (the guard is
//! [`Bard::can_reveal`]), `secureStorageAvailable` / `setObsPassword`
//! (keyring; record the result with `update_config(|c|
//! c.streamer.has_obs_password = ..)`), media keys and the OBS watcher (react
//! to [`CoreEvent::ConfigChanged`]; the watcher calls [`Bard::arm`]).
//!
//! # Deviations from the TypeScript
//!
//! - **`update_config` takes a closure, and its side effects key off what
//!   changed.** setConfig took a patch and re-ran a side effect whenever the
//!   patch *named* a field, even with an unchanged value. [`Bard::arm`] and
//!   [`Bard::disarm`] keep the old always-run behaviour for the streamer
//!   toggle (clear the OBS source, re-apply the mode), where it mattered.
//! - **`ConfigChanged` is sent for every config change**, not just changes
//!   main made on its own, so services can follow settings from one place.
//! - **`StatsChanged`, `AlbumFlagsChanged`, `LovedChanged` carry no payload**;
//!   re-read with the getter.
//! - **`getArtwork` returned a data URL; `artwork_path` returns the cached
//!   file's path**, and refuses ids that are not hex.
//! - **`addLibraryPath` showed the folder dialog**; the UI does that now and
//!   passes the folder. Paths are resolved to absolute before being stored.
//! - **The scan can run without `&mut Bard`** ([`Bard::scanner`]), so a
//!   mutex-held Bard is not locked for the seconds a scan takes.
//! - **Auto-play fallbacks stay inside the playable pool.** The shuffle
//!   fallback in auto-fill and `fillQueue` excluded unplayable ids by passing
//!   them to `pickRandom`, which ignores its exclusion once it covers every
//!   track -- so with streamer mode armed and nothing safe (or everything
//!   playable already queued), they reached past the allow-list and played or
//!   queued an unchecked track. They now pick from the playable tracks, and
//!   come up empty when there are none.
//! - **nowplaying.json writes are synchronous.** The Electron build chained
//!   async writes so they could not interleave; `&mut self` serialises them
//!   here. The throttling rules (write on track change, play/pause, and the
//!   stopped state at shutdown; never on plain progress) are unchanged.
//! - **Store debouncing runs on short-lived threads** instead of timers;
//!   [`Bard::shutdown`] flushes synchronously, as before-quit did.
//! - **`gain_multiplier` takes `&Track`**, not `Track | null`; with no track
//!   the multiplier is simply 1.0.
//! - **`reorder_queue` takes `usize`**, so the negative-index case is gone.
//! - **Album order (`byPathOrder`) approximates ICU collation** (`numeric`,
//!   `sensitivity: 'base'`) without ICU; it matches Electron's order on the
//!   real 585-track library and on ASCII punctuation, see
//!   [`playlist::locale_compare`].
//! - **Durations**: FLAC is exact (STREAMINFO, as music-metadata computed it);
//!   other formats come from lofty in whole milliseconds.
//! - **fuse.js is replaced by a line-for-line port** of its Bitap search and
//!   scoring ([`search::Matcher`]), checked against fuse.js's own output.
//! - **Not ported:** `url.ts` and `protocol.ts` (no bard:// scheme any more;
//!   the audio engine opens files directly), and so their tests (`url`,
//!   `range`). `paths.ts` survives for the reveal guard.
//! - **A corrupt config.json is an error from [`Bard::open`]**, as it was
//!   from electron-store, rather than being silently replaced by defaults. A
//!   section holding a value of the wrong type falls back to that section's
//!   defaults.

pub mod artwork;
mod bard;
mod config;
pub mod deck;
pub mod electron_device;
pub mod gain;
pub mod history;
pub mod library;
mod nowplaying;
pub mod paths;
pub mod playlist;
pub mod progress;
mod queue;
pub mod search;
mod stores;
mod streamer;
pub mod types;
pub mod util;

pub use bard::{Bard, CoreEvent, LibraryScanner, Options, default_data_dir};
pub use gain::{gain_db_for, gain_multiplier};
pub use stores::{PLAY_THRESHOLD_RATIO, PLAY_THRESHOLD_SEC};
