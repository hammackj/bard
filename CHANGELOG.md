# Changelog

All notable changes to Bard are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bard uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) — on a `0.x` line,
so minor bumps are where breaking changes land.

## [Unreleased]

### Fixed

- **A damaged stats, album-flag or loved file is no longer silently replaced.**
  Any read failure used to load as empty, and the next play, flag or love
  wrote that empty store over the file. Now a file that is not valid JSON is
  set aside as `<name>.corrupt-<time>` with its bytes intact, and one that
  cannot be read is left alone for the session. Either way Bard says so and
  keeps playing. Saves that fail are retried, with a notice, and every save
  reaches the disk before it replaces the old file.

## [0.2.4] — 2026-11-12

### Fixed

- **Volume applies at once during a crossfade.** Turning it down, or to
  zero, during a fade-out used to be ignored until the fade finished, up to
  12 seconds; it was also delayed during a fade-in. The fade and the volume
  are now separate stages, so neither can undo the other.
- **Streamer mode cuts the right track after several skips in one fade.**
  Skipping twice during a crossfade left the first track fading out, and
  revoking it did not stop it.
- **Previous no longer returns to a removed folder's track** that was let
  finish, once something else has started.

## [0.2.3] — 2026-11-12

### Fixed

- **Streamer mode takes a track off the air at once**, even with a
  crossfade set. Before, a track whose album stopped being safe (or that was
  playing when the mode was armed) faded out over the whole crossfade, up to
  12 seconds of exactly what must not be heard. The same applies to a track
  still fading out under a safe one.
- **A removed music folder's tracks do not come back** through Repeat All,
  Repeat One or Previous. The track playing when its folder is removed still
  finishes.
- **A seek during a crossfade applies to the incoming track**, which starts
  there. It used to move the outgoing track and be lost.
- **nowplaying.json no longer pairs a new track with the previous track's
  position and duration**; the new track's own timing is written as soon as
  it starts.

## [0.2.2] — 2026-11-12

### Fixed

- **Pausing during a crossfade stays paused.** Before, playback started again
  when the fade finished.
- **A track that will not open is skipped** instead of leaving playback stuck.
  After three failures in a row Bard stops rather than skipping through the
  whole queue.
- **Progress from the outgoing track no longer moves the seek bar** of the
  incoming one during a crossfade or a restart.
- **Streamer mode rechecks a track when its album's Content ID flag changes**,
  and again just before it plays, so a newly claimed album cannot play live.
- **Settings change only once they are saved.** A failed write no longer
  leaves the app using a value that is not on disk.
- **Removing a music folder removes its tracks** from the library, and when
  scans overlap only the newest one's result is kept.

### Changed

- **The album grid is much faster on large libraries.** Covers are decoded in
  the background and only visible rows are drawn. On a 2,000-album library the
  first frame went from about 2.5s to 0.4s and memory from about 1.5GB to
  260MB.

## [0.2.1] — 2026-11-12

### Fixed

- **The output device chosen in the Electron app now carries over.** Electron
  saved it as Chromium's hashed device id, which the Rust app could not match,
  so music went to the system default until the device was picked again. Bard
  now recomputes that id for each PipeWire sink, using the salt Electron left
  in the data directory, and rewrites the setting to the sink's name — at
  startup, or when the device appears later.

## [0.2.0] — 2026-11-12

Bard is a native Rust app.

### Changed

- **Rewritten in Rust with GPUI**, replacing Electron. The window looks and
  behaves as before, and plays through PipeWire directly. Your library, queue, stats, loved tracks, Content ID flags
  and settings carry over unchanged from `~/.config/bard`. Linux only.
- **The tray preview is placed by the compositor** in the corner chosen in
  Settings, so the KWin window rule script is gone.
- **A saved output device that appears after startup is picked up** without a
  restart.
- **Clicking the tray icon brings the window to the front** on Wayland, using
  the activation token the panel provides.

See `docs/PORT.md` for the small deliberate differences.

## [0.1.3] — 2026-11-11

The lock screen knows what is playing.

### Fixed

- **The lock screen, media flyout and MPRIS showed "Bard / Bard"** for every
  track. Bard never set the media session's metadata, so Chromium filled the
  title from the window name and left artist, album and cover empty. The
  current track's title, artist, album and cover art are now published, along
  with play state and position, and the lock screen's play, pause, next,
  previous and seek controls drive Bard's own queue rather than only the bare
  audio element.

## [0.1.2] — 2026-08-09

Streamer mode, and the window stops looking flat.

### Added

- **Streamer mode.** While armed, only albums checked and marked `safe` for
  Content ID will play — `untested` is withheld along with `claimed`, on the
  grounds that an album nobody has checked is not evidence of anything. It
  applies to tracks you pick deliberately as well as to shuffle, fill and
  endless, because the risk does not care who chose the track. Arming also acts
  on the present: the queue is swept and the track on air is skipped if it is
  not safe.
- **Auto-arming from OBS.** Bard connects to the WebSocket server built into
  OBS 28+ and arms itself when streaming, recording, the replay buffer or the
  virtual camera goes active. Arming is one-directional by design — a stream
  ending, a websocket dropping or OBS quitting never disarms it, so a lost
  connection cannot silently remove the protection. Turning it off is always
  deliberate. The OBS password is held by the OS keyring, not `config.json`.
- **A banner** across the top while armed, naming what armed it — Streaming,
  Recording, Replay buffer, Virtual camera — with its own Turn off button.
  Blocked tracks always state a reason; a player that silently refuses is
  indistinguishable from a broken one.
- Every refusal and status message now reaches the notice strip.

### Changed

- The now-playing pane is washed with the current cover, blurred out to its own
  colours, so each track changes the room. Tracks without artwork keep the
  previous flat pane.
- The library and queue panes get light from the top and a corner vignette, with
  faint film grain over the window. The grain is a generated inline SVG rather
  than a shipped asset, and it is static — animated grain is a full-window
  repaint every frame in a player left running for hours.
- Streamer mode's control moved out of the transport row, which was cramped, to
  the banner and a Settings checkbox.

### Fixed

- The OBS password field saved only when it lost focus, so typing a password and
  pressing Enter, or closing Settings with Escape, discarded it silently — and
  the field looked identical either way. Enter now saves, and saving says so.

## [0.1.1] — 2026-08-09

Follow-ups to the first release, mostly found by using the app rather than
reading it.

### Fixed

- Endless play started with an empty queue. The top-up only ever ran in
  reaction to something — a track ending, an item removed, the setting being
  changed — and none of those happen at startup, so the only way to fill the
  queue was to toggle the setting off and on. It now stocks the queue once the
  library scan completes, without starting playback.
- `Space`, the tray menu and the media keys did nothing when the queue was full
  but no track was loaded — the state endless produces. The fallback that
  starts playback in that case existed only on the Play button; all four now
  share one path.
- A transient output-device failure permanently erased the saved device. A
  device that is merely late to enumerate was indistinguishable from one that
  is gone, and the fallback was written to config. The fallback is now
  session-only, the configured device is retried on the next launch, and a
  device that appears later is retried on `devicechange`.
- `nowplaying.json` was never reset on quit, so it kept asserting playback
  after Bard exited and any OBS source reading it showed a phantom now-playing
  card indefinitely. The stopped state is now written synchronously on the way
  out.
- Settings the app no longer has were carried in `config.json` forever — the
  Twitch and song-request blocks were still being read and rewritten months
  after the feature was removed. Config is now shaped to the current schema and
  rewritten once at startup when it finds keys that do not belong.
- A refused second instance now quits rather than exiting hard, so the message
  that raises the already-running window is not cut off in flight.

### Added

- The commit each build came from, shown in the startup log, the Settings About
  row and the avatar menu, with `-dirty` when the build carried uncommitted
  changes. The version alone only moves at release, so it could not tell two
  builds apart.
- `npm run build` warns when the packaged build in `dist/` is older than the
  code just built. The installed app runs the package, not `out/`, so building
  and testing could leave it on old code with nothing to say so.
- App version in the avatar menu, alongside the existing About section.

### Changed

- Endless play's description said it would not repeat a track "until the whole
  library has played", which read as a limit. It plays indefinitely — the deck
  reshuffles when exhausted — and now says so.

## [0.1.0] — 2026-08-09

First versioned release. Everything below already existed; this is the point
the project started tracking versions rather than carrying the scaffold's
`1.0.0` forever.

### Added

- Local library scanning with album grouping, artwork extraction and search.
- Queue with drag-to-reorder, shuffle, fill, and an endless auto-fill mode that
  draws from a weighted shuffle deck.
- Playback with crossfade, ReplayGain-based loudness normalisation (per track
  or per album), and output-device selection.
- Play history with Previous/step-back, repeat modes, and a sleep timer.
- Loved tracks, weighted into the shuffle deck.
- Content ID flags per album, so claimed music stays off stream.
- OBS overlay served over HTTP, plus a `nowplaying.json` for other tools.
- Tray icon with a now-playing preview popup, and optional global media keys.
- About section in Settings reporting the app, Electron, Chrome and Node
  versions.

### Removed

- Twitch chat integration and song requests. Bard is a local player; the
  request flow was dropped deliberately in `d81fd47`.
