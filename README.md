# Bard

A desktop music player for streamers. It plays music from your own library and
takes song requests from Twitch chat — restricted to tracks you already have.

![Bard](resources/icon.svg)

## Why it works this way

**Chat can only request tracks already in your library.** Nothing is fetched
from the internet, nothing streams from YouTube, and there is no search box
pointed at the outside world. You curate every file that can possibly play.

That is the whole copyright-control decision. A request bot that can pull
arbitrary audio is a DMCA strike waiting to happen; one that can only play
files you deliberately added is not. If a viewer asks for something you do not
have, they get "not in library" and nothing plays.

The same reasoning shapes two other choices:

- **The overlay server binds to `127.0.0.1`, never `0.0.0.0`.** It is for your
  OBS instance on this machine. Exposing it to the network would put your
  library — and its artwork — on the LAN.
- **Twitch is read-only and anonymous.** Bard connects as a `justinfan` guest.
  There is no OAuth, no token stored on disk, and nothing for a viewer to
  capture off a screen share. It cannot reply in chat, and that is deliberate.

If your library is licensed music (StreamBeats and similar), check the licence
terms — most royalty-free catalogues permit use *within* your stream but forbid
redistributing the audio separately. Keeping the overlay on localhost is part
of staying inside that line.

## Features

- Plays MP3, FLAC, WAV, OGG, M4A and Opus from folders you choose
- Twitch song requests with per-user cooldowns, queue limits and a block list
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

If Electron fails to start with *"Electron failed to install correctly"*, its
postinstall was blocked — npm 10+ can skip install scripts. Approve it and
reinstall:

```bash
npm install-scripts approve electron esbuild
rm -rf node_modules/electron && npm install
```

Useful scripts:

| Command | What it does |
| --- | --- |
| `npm run dev` | Run with hot reload |
| `npm test` | Unit tests (vitest, no Electron) |
| `npm run typecheck` | Type-check main, preload, renderer and tests |
| `npm run build` | Type-check and bundle |
| `npm run build:linux` / `:win` / `:mac` | Build installers |

Two development-only environment variables:

- `BARD_SHOT=/path/to.png` — captures the app window to a PNG after load,
  which is easier than screenshotting a whole desktop
- `VITE_BARD_SELFTEST=1` — runs an audio self-test at startup. It renders the
  graph at full gain into no output device, then asserts the analyser sees a
  non-zero signal. This exists because a cross-origin-tainted media source
  emits *pure silence* while still reporting that it is playing, so every other
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

### Taking requests

Settings → Twitch, enter your channel name, and press **Connect**. Viewers
request with:

```
!sr ambient gold
```

`!queue` and `!song` are recognised and logged, but Bard cannot reply — the
connection is read-only.

Matching is deliberately conservative. It tolerates typos (`ambent gold`
finds `Ambient Gold`) but refuses vague queries: `!sr lofi` returns "not in
library" rather than starting whichever track scored least badly.

Requests are rejected, in this order, for: blocked user, full queue, per-user
limit, cooldown, no match, blocked track, already queued. A rejected request
does **not** consume the requester's cooldown, so a typo does not cost them two
minutes. Broadcaster and moderators bypass cooldown and the per-user cap.

## Shortcuts

| Key | Action |
| --- | --- |
| `Space` | Play / pause |
| `←` `→` | Seek ±5s |
| `↑` `↓` | Volume |
| `/` | Focus the library filter |
| `Enter` | Queue the selected track |
| `Shift+Enter` | Play it now |
| `Ctrl/Cmd+,` | Settings |
| `?` | Shortcut list |

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

macOS builds are **unsigned**. Gatekeeper will refuse to open them on first
launch; right-click the app and choose **Open** to get the override prompt.
Signing and notarisation need an Apple Developer identity, which is not
configured here.

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
  main/       library scanning, queue, twitch, overlay server, config
  preload/    the contextBridge surface — the only path into the renderer
  renderer/   React UI and the audio engine
resources/
  overlay/    the OBS browser source page
  fonts/      self-hosted woff2, so the app works offline
```

Some deliberate structure worth knowing before changing things:

- **The queue lives in the main process.** Chat requests and UI clicks mutate
  one source of truth; the renderer sends intents and renders what comes back.
- **Audio lives in the renderer**, because `setSinkId` and the Web Audio API
  only exist there. Output routing sets the sink on the *AudioContext*, not the
  media element — once an element feeds a `MediaElementAudioSourceNode`, its
  own `setSinkId` no longer affects what you hear.
- **Local audio is served over a `bard://` scheme**, not `file://`, which the
  renderer cannot load. That scheme needs `corsEnabled` *and* a matching
  `Access-Control-Allow-Origin` header *and* `crossOrigin` on the element. Miss
  any one and playback is silent with no error anywhere.
- **Every channel the renderer can reach is named in `shared/ipc.ts`.** There
  is no generic `ipcRenderer` passthrough.

## Licence

The application code is MIT. Music you play through it is governed by whatever
licence that music carries — Bard neither supplies nor relicenses any audio.
