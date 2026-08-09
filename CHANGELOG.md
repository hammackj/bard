# Changelog

All notable changes to Bard are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bard uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) — on a `0.x` line,
so minor bumps are where breaking changes land.

## [Unreleased]

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
