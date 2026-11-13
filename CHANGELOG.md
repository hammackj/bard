# Changelog

All notable changes to Bard are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bard uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) — on a `0.x` line,
so minor bumps are where breaking changes land.

## [Unreleased]

### Added

- **Experimental macOS and Windows support.** Bard builds and runs on macOS
  (Apple Silicon) and Windows (x86_64). Each release includes a macOS disk
  image and a Windows installer (plus a portable zip), both with uninstallers
  that can also remove Bard's data. Audio goes through CoreAudio or WASAPI
  at the device's own sample rate, with the same output-device picking as on
  Linux. The OBS password is kept in the Keychain or Credential Manager, and
  shortcuts use Cmd on macOS. There is no tray icon or media-key support on either yet.
  Please report problems as GitHub issues.
- **A Debian package** (`bard-player`) on each release, for Debian and
  Ubuntu 24.04 or newer.
- `resources/bard.desktop`, a standard desktop entry for packages, and an
  AUR PKGBUILD (`bard-player-bin`) under `packaging/aur/`.

## [0.2.15] — 2026-11-12

The first release published on GitHub, with a prebuilt Linux binary.

### Added

- **Prebuilt releases.** Each version is published on GitHub Releases as a
  Linux x86_64 tarball with the install script and icons.
- **Licence.** Bard is MIT-licensed; the bundled fonts (Inter, Fira Code)
  and two adapted Feather icons keep their own licences, listed in
  `THIRD-PARTY.md`.
- `CONTRIBUTING.md` and `SECURITY.md`, with private vulnerability reporting.

### Changed

- The README covers installing from a release or from source, with the build
  packages for Debian/Ubuntu and Arch, and a screenshot.
- `scripts/make-album-videos.sh` now asks for the album folder with
  `--library` rather than defaulting to one particular artist's folder.

## [0.2.14] — 2026-11-12

### Fixed

- **Repeat All loops a single track.** With one track played and nothing
  queued, Repeat All stopped at its end (or, with auto-fill on, moved on
  to unrelated music) instead of playing it again.

## [0.2.13] — 2026-11-12

### Fixed

- **Streamer auto-arm notices an OBS that connects but never finishes the
  handshake.** A hung OBS, or the wrong service on its port, left the
  watcher waiting forever with no notice and no retry. It now gives up
  after four seconds, says so once, and keeps retrying.
- **Dragging in the queue moves the song you dragged.** If a track ended
  while you were dragging, the drop moved whichever song had shifted into
  the dragged row instead -- and dropping a song back on itself could move
  its neighbour.

## [0.2.12] — 2026-11-12

### Fixed

- **A rescan that retags the playing track updates nowplaying.json and
  the overlay.** Only a cover change did; a new title, artist or album
  reached the window but the file kept the old tags until the next track
  or pause.
- **A track loaded paused waits at exactly 0:00.** Replacing an audible
  track with one loaded paused could consume its first frame, so it
  started a frame late.

## [0.2.11] — 2026-11-12

### Fixed

- **A rescan updates the now-playing card and the queue.** Retagged titles,
  albums and covers showed in the library but not for the playing or
  queued tracks until they changed; media keys' and the tray's track info
  kept the old tags too. Playback is not interrupted.
- **Changing normalisation during a crossfade applies to the incoming
  track.** It started at the old level instead, and the outgoing track
  jumped to the incoming one's level for the rest of the fade.
- **Shuffle with loved-track weighting no longer repeats the last track**
  at the start of a new pass through the library.

## [0.2.10] — 2026-11-12

### Fixed

- **Streamer mode rechecks tracks a rescan retagged.** A track retagged into
  a claimed or unchecked album at the same path kept playing after a
  rescan while streamer mode was on, and stayed in the queue. A rescan now
  treats it as if its album had been flagged: taken off air at once (or
  left paused if you had paused), dropped from the queue, and silence if
  nothing safe can follow.

## [0.2.9] — 2026-11-12

### Fixed

- **The OBS overlay shows a replaced cover.** Replacing a track's embedded
  art and rescanning updated the window, but the overlay kept the old image
  for as long as the track played, and could bring it back for an hour
  after. The cover's address now changes with the cover, so the overlay
  loads the new one at once; an unchanged cover is still loaded only once.

## [0.2.8] — 2026-11-12

### Fixed

- **Bard again refuses to start where it cannot lock its data folder.**
  0.2.7 let it start without the lock on filesystems that do not support
  locking, which also let a second copy start and overwrite the first one's
  play counts, loves and flags. It now stops with a message; setting
  `BARD_DATA_DIR` to a folder on a local disk runs it.
- **A cover the cache could not save is tried again.** A failed write to
  the cover cache was recorded as the file having no art, so later scans
  never looked again. Now nothing is recorded, an older cover stays, and
  the next scan saves it.

## [0.2.7] — 2026-11-12

### Fixed

- **Streamer mode no longer leaves the stream silent.** When a track had
  just ended and was flagged or blocked before Bard moved on, the track
  replacing it was loaded paused, as if you had paused. It now plays.
- **The sleep timer pauses even between tracks.** Running out while one
  track had ended and the next had not yet started, it did nothing and the
  music played on. Play, from media keys or the tray, right after a pause
  now resumes instead of being ignored.
- **Saving play counts and flags no longer holds up the window.** A save in
  progress kept counting a play, or reading the counts, waiting until it
  was on disk. The now-playing file for OBS and the cover cache's records
  are no longer forced to disk on every write, which on a busy disk could
  stall each track change and pause.
- **A stats, flags or loved file damaged into non-text bytes is set aside**
  like any other damaged file, instead of leaving every session unsaved.
- **Bard starts on filesystems without file locking** (some network home
  folders), without the single-instance lock, instead of refusing to.

## [0.2.6] — 2026-11-12

### Fixed

- **A pause from the window holds too.** 0.2.5's fix for a pause lost when
  the incoming track fails to open covered the tray and media keys but not
  Space or the play button, which paused the engine directly. The engine
  itself now remembers what a pause holds, so every way of pausing counts.
- **Moving a slider while settings are being saved no longer stalls the
  window.** A slider step waited for a save already in progress; on a slow
  disk that was most of a second. Saves still happen one at a time and in
  order.
- **A cover decoded just before a rescan no longer brings back the old
  art.** A decode that finished after its file changed was kept until the
  next scan; it is now dropped and the cover decoded again.

## [0.2.5] — 2026-11-12

### Fixed

- **A damaged stats, album-flag or loved file is no longer silently replaced.**
  Any read failure used to load as empty, and the next play, flag or love
  wrote that empty store over the file. Now a file that is not valid JSON is
  set aside as `<name>.corrupt-<time>` with its bytes intact, and one that
  cannot be read is left alone for the session. Either way Bard says so and
  keeps playing. Saves that fail are retried, with a notice, and every save
  reaches the disk before it replaces the old file.
- **Two Bards can no longer run on the same data at once.** Launching twice
  in the same instant, or without a session bus, could start a second copy
  that wrote the same stats and settings. Bard now takes a lock on its data
  directory before opening anything; a second launch raises the running
  window and exits, and exits even when the raise fails.
- **Covers follow the files.** Replacing or removing a file's embedded art
  never reached the cover cache, and a damaged cached cover was never
  rebuilt. Each cached cover now records the file it came from, and a scan
  reads the art again only when that file changed; files without art are no
  longer re-read on every scan either. The first scan after updating reads
  every cover once.
- **A pause holds when the track it paused can no longer play.** Paused
  during a crossfade onto a file that then failed to open, paused just as a
  track ended, or paused while streamer mode replaced a file that had
  already failed: in each case the next track used to start on its own. It
  now loads paused, as a replacement made while paused already did.

### Changed

- **Dragging the volume or crossfade slider no longer writes settings on
  every step.** The change is heard at once and saved in the background, one
  write per pause in the drag; a failed save says so and is retried, and the
  last value is saved on quit.

### Tests

- Seeded random sequences of playback actions -- transport, queue, repeat,
  crossfade, arm and revoke, folder removal and rescan, files that fail to
  open, late engine reports -- run against the real core and event handling
  on a simulated engine, checking after every step that nothing forbidden
  reaches the engine or is heard, a pause is never overridden, and the
  window shows the newest request's track and timing. A failure prints its
  seed and a shortened sequence. A second set runs the real engine at volume
  0 through generated fades, cuts, pauses and seeks and checks every
  rendered sample is silent.

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
