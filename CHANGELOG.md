# Changelog

All notable changes to Bard are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bard uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) — on a `0.x` line,
so minor bumps are where breaking changes land.

## [Unreleased]

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
