# Bard

A local music player for a curated, copyright-free library. Think a deliberately
dumb Spotify: it plays the files you own, and nothing else.

![Bard](resources/icon.svg)

## Why it works this way

**Bard only ever plays files you added.** Nothing is fetched from the internet,
nothing streams from a third party, and there is no search box pointed at the
outside world. That is the whole design: a player you can leave running over a
stream without wondering whether the next track is licensed.

It was briefly a song-request bot as well. That was removed. With a library of
several hundred instrumental tracks, nobody in chat knows a title to ask for,
and listing them is not possible inside a chat message — the feature could not
do the job it existed for. The player, the queue and the overlay stand on their
own.

The overlay server binds to `127.0.0.1`, never `0.0.0.0`. It is for your OBS
instance on this machine; exposing it would put your library, and its artwork,
on the network.

If your library is licensed music (StreamBeats and similar), check the terms —
most royalty-free catalogues permit use _within_ your stream but forbid
redistributing the audio separately. Keeping the overlay on localhost is part
of staying inside that line.

## Features

- Plays MP3, FLAC, WAV, OGG, M4A and Opus from folders you choose
- **Routes audio to a specific output device**, so music can go to a dedicated
  stream mix instead of your monitors
- Live OBS overlay over server-sent events, plus `nowplaying.json` for
  file-based setups
- Embedded album art and a live waveform visualiser
- Shuffle and album modes that keep playing when the queue runs dry
- Tray icon with a now-playing preview; closing the window hides rather than
  quits

## Requirements

Node 20+ and npm. Linux, macOS or Windows.

## Development

```bash
npm install
npm run dev
```

If Electron fails to start with _"Electron failed to install correctly"_, its
postinstall was blocked — npm 10+ can skip install scripts. Approve it and
reinstall:

```bash
npm install-scripts approve electron esbuild
rm -rf node_modules/electron && npm install
```

Useful scripts:

| Command                                 | What it does                                 |
| --------------------------------------- | -------------------------------------------- |
| `npm run dev`                           | Run with hot reload                          |
| `npm test`                              | Unit tests (vitest, no Electron)             |
| `npm run typecheck`                     | Type-check main, preload, renderer and tests |
| `npm run build`                         | Type-check and bundle                        |
| `npm run build:linux` / `:win` / `:mac` | Build installers                             |

Two development-only environment variables:

- `BARD_SHOT=/path/to.png` — captures the app window to a PNG after load,
  which is easier than screenshotting a whole desktop
- `VITE_BARD_SELFTEST=1` — runs an audio self-test at startup. It renders the
  graph at full gain into no output device, then asserts the analyser sees a
  non-zero signal. This exists because a cross-origin-tainted media source
  emits _pure silence_ while still reporting that it is playing, so every other
  check passes against nothing.

## First run

1. Launch Bard and click **Choose a music folder**.
2. Wait for the scan. Tags and embedded art are read once and cached;
   rescans skip art that is already cached.
3. Open **Settings** (`Ctrl+,`).

### Adding the OBS overlay

Settings shows the overlay URL — by default `http://127.0.0.1:4747`. In OBS add
a **Browser Source** with that URL, roughly 470×210. It updates the instant a
track changes; no polling and no refresh needed.

If port 4747 is taken, Bard walks upward and shows the port it actually bound.
Use the **Copy** button rather than assuming 4747.

For a file-based workflow, `nowplaying.json` is written to the app's config
directory (`~/.config/bard` on Linux). It is written atomically, so OBS never
reads a half-written file, and only when the track, play state or queue length
changes — not on every progress tick.

### Choosing an output device

Settings → Audio → Output device. To send music to a virtual cable while your
own monitoring stays separate, pick that device here.

**Device names are blank until you open the picker.** Reading them requires
microphone permission, because that is what unlocks labels in
`enumerateDevices()`. Bard asks only for that, records nothing, and closes the
stream immediately. It is deferred until you open the picker because opening a
capture stream makes the audio pipeline renegotiate, which is audible.

## Shortcuts

| Key           | Action                   |
| ------------- | ------------------------ |
| `Space`       | Play / pause             |
| `←` `→`       | Seek ±5s                 |
| `↑` `↓`       | Volume                   |
| `/`           | Focus the library filter |
| `Enter`       | Queue the selected track |
| `Shift+Enter` | Play it now              |
| `Ctrl/Cmd+,`  | Settings                 |
| `?`           | Shortcut list            |

System-wide media keys are available but **off by default** — once taken, they
stop reaching every other player on your machine. Enable them in Settings.

## Building installers

```bash
npm run build:linux    # AppImage + deb
npm run build:win      # NSIS installer
npm run build:mac      # dmg (arm64 + x64)
```

Output lands in `dist/`. `build:linux` also refreshes `dist/linux-unpacked`,
which the desktop launcher points at, so one command keeps the shortcut, the
AppImage and the deb in step. All of them type-check first, so a broken build
cannot be packaged.

**`npm run build` does not update the installed app.** It writes `out/`, which
is what `npm run dev` and `npm start` run. The launcher in your application
menu runs the _packaged_ build in `dist/`, and only the `build:*` commands
above refresh that. You can build and test all day while the installed app
stays on month-old code — that is not hypothetical, a packaged build once ran
for a day without the loudness-normalisation code while the source had it, and
the symptom looked like broken normalisation rather than a stale package.

`npm run build` now warns when `dist/` is behind `out/`, and every build stamps
in its commit: the startup log, the About section in Settings and the avatar
menu all show it next to the version, with `-dirty` when the build carried
uncommitted changes. If the app is not behaving like the source, check that
first.

> **Debugging a packaged build:** plain `grep` against an `.asar` reports **no
> matches regardless of content** — it detects binary and suppresses counting,
> which produces a very convincing false negative. Use `strings file | grep`,
> `grep -a`, or `npx asar extract` first. Note also that `replaygain_track_gain`
> appears inside the asar even when Bard's own gain code is absent, because
> music-metadata ships those strings; it is not a usable probe.

macOS builds are **unsigned**. Gatekeeper will refuse to open them on first
launch; right-click the app and choose **Open** to get the override prompt.
Signing and notarisation need an Apple Developer identity, which is not
configured here.

### Cutting a release

The version in `package.json` is the single source of truth: electron-builder
interpolates it into every artifact name, and `app.getVersion()` reads it back
at runtime for the About row and the startup log.

Write the release's entry in `CHANGELOG.md` first, then:

```bash
npm version patch --no-git-tag-version   # or minor / major; bumps package.json only
git add -A && git commit -m "Bard 0.1.1"
git tag -a v0.1.1 -m "Bard 0.1.1"
npm run build:linux
git push && git push --tags
```

`--no-git-tag-version` is deliberate. Plain `npm version` commits _and_ tags in
one step, so writing the changelog afterwards and amending it in would leave the
tag pointing at the commit as it was before the amend — orphaned, and no longer
the release. Bumping without tagging lets the changelog and the version bump
land as one commit that the tag then points at.

Bard is on a `0.x` line: minor bumps carry breaking changes, patch bumps do
not. `npm version` refuses to move a version backwards, which is only a problem
if you ever need to undo a bump by hand.

### Pinning the tray preview (KDE/Wayland)

Wayland compositors place windows themselves and ignore an application's
request — verified: a request for (4700, 2660) lands at (0, 0). The corner
setting in Bard therefore only takes effect on X11, Windows and macOS.

On KDE the native fix is a KWin window rule:

```bash
./scripts/install-kwin-rule.sh --corner bottom-right --screen DP-3 --margin 70
./scripts/install-kwin-rule.sh --remove   # undo
```

It matches on a substring of the window caption (exact caption matching turned
out to be brittle), backs up `kwinrulesrc` before editing, and leaves any rules
you already had alone. Raise `--margin` past the thickness of a panel that
overlaps the chosen corner.

### Desktop launcher for a local build

Packaged installers register their own launcher. For a locally built binary:

```bash
npm run build && npx electron-builder --linux --dir
./scripts/install-desktop.sh
```

That installs a `.desktop` entry and the full icon set into your per-user
XDG directories.

## Where things live

```
src/
  shared/     types, the IPC contract, and the pure logic the tests cover
  main/       library scanning, queue, overlay server, config, tray
  preload/    the contextBridge surface — the only path into the renderer
  renderer/   React UI and the audio engine
resources/
  overlay/    the OBS browser source page
  fonts/      self-hosted woff2, so the app works offline
```

Some deliberate structure worth knowing before changing things:

- **The queue lives in the main process.** The renderer sends intents and
  renders what comes back, so there is one source of truth for what plays next.
- **Audio lives in the renderer**, because `setSinkId` and the Web Audio API
  only exist there. Output routing sets the sink on the _AudioContext_, not the
  media element — once an element feeds a `MediaElementAudioSourceNode`, its
  own `setSinkId` no longer affects what you hear.
- **Local audio is served over a `bard://` scheme**, not `file://`, which the
  renderer cannot load. That scheme needs `corsEnabled` _and_ a matching
  `Access-Control-Allow-Origin` header _and_ `crossOrigin` on the element. Miss
  any one and playback is silent with no error anywhere.
- **Every channel the renderer can reach is named in `shared/ipc.ts`.** There
  is no generic `ipcRenderer` passthrough.

## Licence

The application code is MIT. Music you play through it is governed by whatever
licence that music carries — Bard neither supplies nor relicenses any audio.
