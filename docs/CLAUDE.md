# Bard — Electron Stream Music Player
## CLAUDE.md — Full Build Guide for Claude Code

> Complete implementation spec for **Bard**, a desktop music player for streamers.
> Work through each phase in order. Each phase ends in a state that runs (`npm run dev`) without errors.
> Do not skip ahead — later phases assume earlier ones type-check cleanly.
>
> **This spec supersedes the previous Rust/ratatui spec.** See "Migration notes" at the bottom for what changed and why.

---

## Project Overview

**What we're building:**

A cross-platform desktop music player for live streamers that:

- Plays local audio files (MP3, FLAC, WAV, OGG, M4A, Opus)
- Connects to Twitch IRC (read-only, anonymous — no OAuth) to receive song requests from chat
- **Chat can only request tracks already in the streamer's local library.** This is the core copyright-control decision: nothing is fetched from the internet, nothing streams from YouTube, the streamer curates every file that can possibly play
- Enforces per-user cooldowns, queue limits, and a block list
- Serves a live OBS overlay from a built-in local HTTP server, and also writes `nowplaying.json` for anyone who prefers a file-based workflow
- Routes audio to a **specific output device** so the streamer can send music to a dedicated stream mix instead of their monitors
- Displays embedded album art and a live waveform visualizer

**Non-goals for MVP:** playlists beyond the queue, YouTube chat, library editing/tagging, network streaming of any kind.

---

## Tech Stack

| Concern | Choice | Notes |
|---|---|---|
| Shell | Electron | Latest stable |
| Build | `electron-vite` | Handles main/preload/renderer bundling + HMR |
| Language | TypeScript, strict mode | Shared types across processes |
| UI | React 18+ | Function components + hooks only |
| Styling | Tailwind CSS | With a custom token layer (see Phase 9) |
| Audio | `HTMLAudioElement` + Web Audio API | `AudioContext`, `MediaElementSource`, `AnalyserNode`, `GainNode` |
| Metadata | `music-metadata` | Tags + embedded cover art, runs in main |
| Library scan | `node:fs/promises` + `fast-glob` | Main process only |
| Fuzzy match | `fuse.js` | Replaces the Rust `fuzzy-matcher` |
| Twitch IRC | `ws` | Main process, `wss://irc-ws.chat.twitch.tv:443` |
| Overlay server | `express` | Bound to `127.0.0.1`, SSE for push updates |
| Config | `electron-store` | JSON, schema-validated |
| Packaging | `electron-builder` | NSIS, dmg, AppImage |
| Tests | `vitest` | Pure logic modules only |

> **Version pinning:** do not hardcode versions from memory. Run `npm install <pkg>` and let npm resolve latest, then commit the resulting `package-lock.json`. If any package fails to install, report it rather than substituting a different library.

---

## Architecture — Process Model

The split matters. Get this wrong and everything downstream fights you.

```
┌─────────────────────── MAIN PROCESS (Node) ────────────────────────┐
│                                                                     │
│  library.ts     scans dirs, reads tags, extracts cover art          │
│  twitch.ts      IRC WebSocket, parses PRIVMSG, emits request events │
│  requests.ts    cooldowns, block list, queue rules (pure)           │
│  overlay.ts     express server on 127.0.0.1:4747 + SSE broadcast    │
│  nowplaying.ts  writes nowplaying.json                              │
│  config.ts      electron-store wrapper                              │
│  protocol.ts    registers bard:// for streaming local audio         │
│                                                                     │
└────────────────────────────┬───────────────────────────────────────┘
                             │ IPC (typed, via preload contextBridge)
┌────────────────────────────┴───────────────────────────────────────┐
│                     RENDERER PROCESS (React)                        │
│                                                                     │
│  audio engine   HTMLAudioElement + Web Audio graph + setSinkId      │
│  UI             library / player+queue / chat+settings              │
│  visualizer     AnalyserNode → the string bar (see Phase 9)         │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

**Rules:**

1. **The renderer never touches `fs`, `net`, or `child_process`.** `contextIsolation: true`, `nodeIntegration: false`, `sandbox: true`. All privileged work goes through the preload bridge.
2. **Audio lives in the renderer**, because `setSinkId()` and the Web Audio API only exist there. The main process never plays anything.
3. **Queue state is owned by the main process.** The renderer renders it and sends intents (`play`, `skip`, `reorder`). This means chat requests and UI actions mutate one source of truth, not two, and the overlay server can read it directly.
4. **The renderer reports playback progress up to main** (throttled to ~4/sec) so the overlay and `nowplaying.json` stay accurate.

---

## Directory Structure (target)

```
bard/
├── package.json
├── electron.vite.config.ts
├── electron-builder.yml
├── tsconfig.json / tsconfig.node.json / tsconfig.web.json
├── tailwind.config.js
├── CLAUDE.md                    ← this file
├── resources/
│   ├── icon.png
│   ├── fonts/                   ← self-hosted woff2, see Phase 9
│   └── overlay/
│       ├── index.html           ← OBS browser source target
│       ├── overlay.css
│       └── overlay.js
├── src/
│   ├── shared/
│   │   ├── types.ts             ← Track, QueueItem, ChatEvent, Config, NowPlaying
│   │   ├── ipc.ts               ← channel names + payload types (single source of truth)
│   │   └── requests.ts          ← pure cooldown/queue rules, imported by main AND tests
│   ├── main/
│   │   ├── index.ts             ← app lifecycle, BrowserWindow, wiring
│   │   ├── config.ts
│   │   ├── protocol.ts
│   │   ├── library.ts
│   │   ├── artwork.ts
│   │   ├── queue.ts             ← stateful wrapper around shared/requests.ts
│   │   ├── twitch.ts
│   │   ├── overlay.ts
│   │   ├── nowplaying.ts
│   │   └── ipc-handlers.ts
│   ├── preload/
│   │   └── index.ts             ← contextBridge.exposeInMainWorld('bard', api)
│   └── renderer/
│       ├── index.html
│       ├── main.tsx
│       ├── App.tsx
│       ├── audio/
│       │   ├── engine.ts        ← the only place audio elements are created
│       │   └── useAudio.ts
│       ├── state/
│       │   └── store.ts         ← zustand or useReducer; keep it small
│       ├── components/
│       │   ├── LibraryPanel.tsx
│       │   ├── NowPlaying.tsx
│       │   ├── StringBar.tsx    ← the signature progress/visualizer element
│       │   ├── QueuePanel.tsx
│       │   ├── ChatLog.tsx
│       │   ├── SettingsPanel.tsx
│       │   └── FirstRun.tsx
│       └── styles/
│           └── index.css        ← @theme tokens + Tailwind
└── tests/
    ├── requests.test.ts
    └── library-match.test.ts
```

---

## Phase 1 — Scaffold

```bash
npm create @quick-start/electron@latest bard -- --template react-ts
cd bard
npm install
npm run dev   # must open a window before continuing
```

Then add runtime deps:

```bash
npm install music-metadata fuse.js ws express electron-store fast-glob zustand
npm install -D tailwindcss @tailwindcss/vite vitest @types/ws @types/express
```

**Verify:** `npm run dev` opens a window, `npm run typecheck` passes.

**Commit before moving on.** Do this at the end of every phase.

---

## Phase 2 — Main Process Shell & Security

### 2.1 BrowserWindow

```ts
const win = new BrowserWindow({
  width: 1280,
  height: 800,
  minWidth: 1000,
  minHeight: 640,
  show: false,
  backgroundColor: '#020617',
  titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'default',
  webPreferences: {
    preload: join(__dirname, '../preload/index.js'),
    contextIsolation: true,
    nodeIntegration: false,
    sandbox: true,
  },
})
win.once('ready-to-show', () => win.show())
```

Show on `ready-to-show`, not immediately — otherwise there's a white flash before the dark UI paints.

### 2.2 Custom protocol for local audio

The renderer cannot load `file://` URLs when the page itself is served from `http://localhost:5173` in dev. Register a scheme instead — this is the single most important piece of plumbing in the app.

In `src/main/protocol.ts`:

```ts
import { protocol, net } from 'electron'
import { pathToFileURL } from 'node:url'

// must be called BEFORE app.whenReady()
export function registerScheme() {
  protocol.registerSchemesAsPrivileged([
    { scheme: 'bard', privileges: { standard: true, secure: true, stream: true, supportFetchAPI: true, bypassCSP: true } },
  ])
}

// called after app.whenReady()
export function handleScheme(isAllowed: (p: string) => boolean) {
  protocol.handle('bard', (req) => {
    const filePath = decodeURIComponent(new URL(req.url).pathname)
    if (!isAllowed(filePath)) return new Response('Forbidden', { status: 403 })
    return net.fetch(pathToFileURL(filePath).toString())
  })
}
```

`stream: true` is required or seeking within a track will not work — the renderer needs range requests.

`isAllowed` must confirm the resolved, normalized path sits inside one of the configured library roots. Do not skip this; without it any page in the renderer could read arbitrary files off disk.

### 2.3 Config

`electron-store`, defaults:

```ts
{
  libraryPaths: [],
  twitch: { channel: '', enabled: false, commandPrefix: '!sr' },
  requests: {
    cooldownSeconds: 120,
    maxQueueLength: 20,
    maxPerUserInQueue: 2,
    blockedUsers: [],
    blockedTracks: [],
  },
  audio: { volume: 0.8, outputDeviceId: 'default' },
  overlay: { port: 4747, writeNowPlayingJson: true },
}
```

**Verify:** app launches, config file appears in `app.getPath('userData')`.

---

## Phase 3 — IPC Contract & Preload Bridge

Define every channel in `src/shared/ipc.ts` first, as a typed map. Both sides import it. No string literals anywhere else.

```ts
export type BardApi = {
  // library
  scanLibrary: () => Promise<Track[]>
  addLibraryPath: () => Promise<string[]>        // opens dialog, returns new paths
  getArtwork: (trackId: string) => Promise<string | null>  // data URL

  // queue — renderer sends intent, main owns state
  enqueue: (trackId: string, requester?: string) => Promise<void>
  removeFromQueue: (queueItemId: string) => Promise<void>
  reorderQueue: (from: number, to: number) => Promise<void>
  skip: () => Promise<void>
  clearQueue: () => Promise<void>

  // playback reporting (renderer → main)
  reportProgress: (p: { position: number; duration: number; playing: boolean }) => void
  reportEnded: () => void

  // twitch
  connectTwitch: (channel: string) => Promise<void>
  disconnectTwitch: () => Promise<void>

  // config
  getConfig: () => Promise<Config>
  setConfig: (patch: DeepPartial<Config>) => Promise<Config>

  // subscriptions (main → renderer), each returns an unsubscribe fn
  onQueueChanged: (cb: (q: QueueItem[]) => void) => () => void
  onPlayTrack: (cb: (t: Track) => void) => () => void
  onChatEvent: (cb: (e: ChatEvent) => void) => () => void
  onTwitchStatus: (cb: (s: 'connected' | 'connecting' | 'disconnected' | 'error') => void) => () => void
}
```

Preload exposes exactly this shape via `contextBridge.exposeInMainWorld('bard', api)`. Declare `window.bard` in a `.d.ts` in the renderer.

**Every `onX` subscription must return an unsubscribe function** and every React effect must call it on cleanup. In dev with HMR, listeners leak fast otherwise and you get each chat message rendered five times.

**Verify:** a round-trip `getConfig()` call from the renderer logs the config object.

---

## Phase 4 — Library Scanning

### 4.1 Track model (`src/shared/types.ts`)

```ts
export type Track = {
  id: string          // stable hash of the absolute path
  path: string
  title: string       // tag title, else filename without extension
  artist: string      // tag artist, else 'Unknown Artist'
  album: string
  durationSec: number
  hasArtwork: boolean
  searchText: string  // lowercased "title artist album" — what Fuse indexes
}
```

### 4.2 Scan

Walk each configured root with `fast-glob` for the supported extensions. For each file, `music-metadata`'s `parseFile` with `{ duration: true, skipCovers: false }`.

Parse concurrently but **cap it** — a pool of 8 is right. Unbounded `Promise.all` over a 5,000-file library will exhaust file handles.

Emit progress to the renderer as you go (`{ scanned, total, currentPath }`) so a big first scan shows a progress bar instead of a frozen window.

### 4.3 Artwork

Do not put cover art in the `Track` objects — a 300 KB JPEG × 2,000 tracks is not something to hold in memory or push over IPC.

Instead: on scan, write each cover to `${userData}/artwork/${trackId}.jpg` (resized to 512px max edge). `getArtwork(trackId)` reads it lazily and returns a data URL. Tracks with no embedded art fall back to the wordmark placeholder in the UI.

### 4.4 Matching

Build a `Fuse` index over `searchText` with `{ threshold: 0.4, includeScore: true, keys: ['searchText'] }`.

Expose `matchRequest(query: string): Track | null` — returns the top hit only if its score is under the threshold. **Ambiguous queries should return null and produce a "not found" chat event, not a random track.** A viewer typing `!sr lofi` should not start a 40-minute mix nobody asked for.

**Verify:** point the app at a folder, see tracks appear with correct titles and durations.

---

## Phase 5 — Audio Engine (Renderer)

`src/renderer/audio/engine.ts` owns a single module-level graph. Never create a second `AudioContext` — Chromium caps them and you'll silently run out.

```
HTMLAudioElement → MediaElementAudioSourceNode → GainNode → AnalyserNode → destination
```

- **Source:** `audio.src = 'bard://local' + encodeURI(track.path)`
- **Volume:** the `GainNode`, not `audio.volume` — gives headroom and a smooth ramp with `setTargetAtTime`
- **Analyser:** `fftSize = 2048`, `getByteTimeDomainData` into a `Uint8Array`, read in a `requestAnimationFrame` loop for the visualizer
- **Progress:** `timeupdate` fires ~4/sec — throttle IPC reports to that, don't use rAF for it
- **Ended:** `ended` → `window.bard.reportEnded()` → main advances the queue → main pushes `onPlayTrack`

### 5.1 Output device routing

This is the feature that justifies the whole rewrite. `audioElement.setSinkId(deviceId)` sends playback to a chosen device.

**Known gotcha:** `navigator.mediaDevices.enumerateDevices()` returns devices with **empty `label` fields** until media permission is granted, so the settings dropdown shows a list of blank entries. Fix it in main:

```ts
session.defaultSession.setPermissionRequestHandler((_wc, permission, cb) => {
  cb(permission === 'media')
})
```

and in the renderer, call `navigator.mediaDevices.getUserMedia({ audio: true })` once on first entry to the audio settings, immediately stopping every track on the returned stream. That single call unlocks the labels. Explain this in the settings UI in one line — "Bard asks for microphone access only to read your output device names. Nothing is recorded." — because an unexplained mic prompt in a streaming tool looks alarming.

Persist `outputDeviceId` and re-apply on startup. If the saved device is gone (unplugged interface), fall back to `'default'` and surface a toast rather than failing silently.

**Verify:** play a track, change output device mid-playback, audio follows.

---

## Phase 6 — Queue, Cooldowns & Block List

Put the rules in `src/shared/requests.ts` as **pure functions with no I/O and no clock access** — pass `now: number` in. This is what Phase 11 tests.

```ts
export type RequestVerdict =
  | { ok: true; track: Track }
  | { ok: false; reason: 'blocked_user' | 'blocked_track' | 'cooldown' | 'queue_full'
                        | 'user_limit' | 'not_found' | 'already_queued'
      detail?: string }

export function evaluateRequest(args: {
  query: string
  requester: string
  now: number
  queue: QueueItem[]
  lastRequestAt: Record<string, number>
  config: RequestConfig
  match: (q: string) => Track | null
}): RequestVerdict
```

Evaluation order matters — check cheapest and most definitive first: blocked user → queue full → user limit → cooldown → match → blocked track → already queued.

Cooldowns are **per requester**, keyed by lowercased username. Broadcaster and mods bypass cooldown and user limits (Twitch tags give you `badges` and `mod`). Every verdict, pass or fail, becomes a `ChatEvent` for the chat log.

The stateful wrapper in `src/main/queue.ts` holds the queue array, the `lastRequestAt` map, and broadcasts `onQueueChanged` after every mutation. Streamer-initiated plays from the UI skip `evaluateRequest` entirely — the streamer is never rate-limited on their own machine.

**Verify:** enqueue from the UI, reorder by drag, skip advances correctly.

---

## Phase 7 — Twitch IRC

Main process, `ws`, `wss://irc-ws.chat.twitch.tv:443`.

```
CAP REQ :twitch.tv/tags twitch.tv/commands
PASS SCHMOOPIIE
NICK justinfan<random 5 digits>
JOIN #<channel>
```

Anonymous read-only — no OAuth, no token storage, nothing for a viewer to steal off a screen share. `justinfan` nicks are Twitch's documented anonymous convention.

**Must handle:**
- `PING :tmi.twitch.tv` → reply `PONG :tmi.twitch.tv` immediately. Miss this and you're disconnected in five minutes with no error.
- Reconnect with exponential backoff (1s, 2s, 4s… capped at 30s) plus jitter. Reset the backoff after 60s of a healthy connection.
- Tag parsing: `display-name`, `user-id`, `mod`, `badges`, `color`. Tags are `;`-separated `key=value`, and values escape `\s` as space, `\:` as semicolon, `\\` as backslash. Unescape them.
- A single message may contain multiple IRC lines. Split on `\r\n` and handle each.

Route any message starting with the configured prefix (default `!sr`) to `evaluateRequest`. Also implement `!queue` (report length) and `!song` (report current track) as no-op log entries for now — they don't need to reply since the connection is read-only.

**Verify:** connect to a live channel, watch messages land in the chat log. Test the request path by typing in your own chat.

---

## Phase 8 — Now Playing & OBS Overlay

### 8.1 Overlay server

`express` bound explicitly to `127.0.0.1` — **never `0.0.0.0`**, or a streamer on a shared network exposes their overlay to the LAN.

- `GET /` → serves `resources/overlay/index.html`
- `GET /events` → SSE stream, pushes the now-playing payload on every change
- `GET /art/:trackId` → current cover art

The overlay page connects to `/events` and updates in place. No polling, no `localStorage` bridge — a real push channel is the main upgrade over the old browser version.

If the port is taken, increment and retry up to 10 times, then surface the actual bound port in the settings panel with a copy button. The streamer needs that URL for OBS.

### 8.2 nowplaying.json

Keep writing it for anyone with an existing file-based setup. Write to a temp file then `rename` — atomic, so OBS never reads a half-written file.

```json
{
  "title": "…", "artist": "…", "album": "…",
  "durationSec": 214, "positionSec": 87,
  "requestedBy": "someviewer", "isPlaying": true,
  "artUrl": "http://127.0.0.1:4747/art/abc123",
  "queueLength": 4, "updatedAt": "2026-08-06T18:00:00Z"
}
```

Write on track change and pause/resume. Do **not** write on every progress tick — that's 4 disk writes a second for an eight-hour stream.

**Verify:** add the URL as an OBS browser source, confirm it updates when the track changes.

---

## Phase 9 — UI

### 9.1 Design tokens

Bard matches hammackj.com — cool slate, cyan accent. The anchor is `#0891B2` (cyan-600), the site's declared `theme-color`. Everything else is the Tailwind `slate` ramp, so the app and the site read as one brand.

The streamer glances at this between chat messages: the current track must be readable from across a room, everything else recedes.

```css
@theme {
  --color-ink:       #020617;  /* slate-950  — app background */
  --color-panel:     #0F172A;  /* slate-900  — panel surfaces */
  --color-edge:      #1E293B;  /* slate-800  — 1px dividers, never a heavy border */
  --color-text:      #F1F5F9;  /* slate-100  — primary text */
  --color-muted:     #94A3B8;  /* slate-400  — secondary text, metadata */
  --color-cyan:      #22D3EE;  /* cyan-400   — thin strokes, text accents, the string */
  --color-cyan-deep: #0891B2;  /* cyan-600   — site theme-color, solid fills and buttons */
  --color-green:     #34D399;  /* emerald-400 — request accepted */
  --color-rose:      #FB7185;  /* rose-400    — request rejected, destructive actions */
}
```

**Two tones of cyan, used for different jobs.** `cyan-600` is the brand value but sits too dark for hairline strokes and small text on `slate-950`; `cyan-400` carries those. Use `cyan-600` for solid fills — primary buttons, the active-row strip, filled badges — where the surface area makes it read correctly.

If the site actually uses `zinc` or `neutral` rather than `slate` for its greys, swap the five neutral values for that ramp at the same numeric stops and change nothing else.

Typography — self-host all three as woff2 in `resources/fonts`, loaded via `@font-face`. The app must work offline; no CDN links.

- **Display** — *Fraunces*, weight 600, optical size high. Track titles only. Nothing else gets it.
- **UI** — *Public Sans*. Everything structural.
- **Utility** — *JetBrains Mono*. Timecodes, durations, queue positions, chat prefixes. Tabular figures so numbers don't jitter as they count.

Type scale: 34 / 20 / 15 / 13 / 11. Five sizes, no improvising.

### 9.2 Layout

Three columns, same information architecture as the TUI version — it was the right shape.

```
┌────────────────┬──────────────────────────────────┬──────────────────┐
│  LIBRARY       │           NOW PLAYING            │   CHAT           │
│  ┌──────────┐  │                                  │  req  dave       │
│  │ search   │  │      ┌────────────┐              │   ok  → Nightfall│
│  └──────────┘  │      │            │  Nightfall   │  req  mira       │
│  Nightfall     │      │  cover art │  Kilobyte    │   no  cooldown   │
│  Slow Tide     │      │            │  Amber EP    │  req  jonas      │
│  Amber Rain    │      └────────────┘              │   ok  → Slow Tide│
│  …             │                                  │                  │
│                │  ╭──────∿∿∿──────────────────╮   │                  │
│                │       1:27          3:34         │                  │
│                │      ⏮   ⏸   ⏭    ─────○──       │                  │
│                ├──────────────────────────────────┼──────────────────┤
│                │  QUEUE                    4      │  SETTINGS        │
│                │  1  Slow Tide      jonas   3:02  │  ● twitch: live  │
│  1,284 tracks  │  2  Amber Rain     mira    4:15  │  overlay :4747   │
└────────────────┴──────────────────────────────────┴──────────────────┘
```

Left column fixed 280px, right fixed 320px, center flexes. Panels are `--color-panel` on `--color-ink`, separated by 1px `--color-edge` — no shadows, no rounded card stacks.

### 9.3 The signature element: the string

The progress bar is a plucked string. `StringBar.tsx` renders an SVG path across the player width. At rest it's a straight cyan hairline. During playback the path's control points are displaced vertically by the live `AnalyserNode` amplitude — the string vibrates in time with the music. The played portion is `--color-cyan` with a faint glow; ahead of the playhead it's `--color-edge`. On pause, the displacement damps to flat over ~400ms rather than snapping.

Clicking or dragging anywhere on the string seeks. The playhead is a small cyan dot, not a chunky handle.

This is the one place to spend effort on motion. Everything else in the UI should be still: no hover lifts, no gradient washes, no pulsing. Respect `prefers-reduced-motion` by holding the string flat and rendering a plain determinate bar.

### 9.4 Component notes

- **Library:** virtualize the list (`@tanstack/react-virtual`) — 5,000 rows of DOM will jank. Type-to-filter from anywhere with the library focused; `Esc` clears. `Enter` enqueues, `Shift+Enter` plays immediately.
- **Chat log:** 3-char prefix per line, monospace, color-coded — `sys` muted, `req` cyan, `ok` green, `no` rose. Auto-scroll to bottom unless the user has scrolled up, then show a "jump to latest" affordance. Cap at 500 lines in memory.
- **Queue:** drag to reorder (`dnd-kit`). Show requester name in cyan, duration in mono. Right-click a row for "remove" and "block this user".
- **Settings:** editable, unlike the TUI version. Library paths (add/remove with a native dialog), Twitch channel + connect toggle, cooldown/limit numbers, output device dropdown, overlay URL with copy button, block lists.

---

## Phase 10 — Shortcuts, Tray & First Run

**Global shortcuts** (registered with `globalShortcut`, so they work while OBS is focused). Make them configurable and **off by default** — grabbing media keys system-wide without asking is hostile.

- `MediaPlayPause`, `MediaNextTrack`, `MediaPreviousTrack`

**In-app:** `Space` play/pause, `→`/`←` seek ±5s, `↑`/`↓` volume, `/` focus search, `Ctrl/Cmd+,` settings, `?` shortcut overlay.

**Tray icon:** current track in the tooltip, menu with play/pause, skip, show window, quit. Closing the window hides to tray rather than quitting (configurable) — a streamer will close it by accident mid-stream otherwise.

**First run:** if `libraryPaths` is empty, the whole center column becomes a single invitation, not an error. One line explaining that Bard only ever plays files you add, and one button: "Choose a music folder." Nothing else on screen.

---

## Phase 11 — Tests

`vitest`, unit only. No Electron in the test environment.

`tests/requests.test.ts` — against `src/shared/requests.ts`:
- blocked user rejected before anything else runs
- cooldown boundary: at exactly `cooldownSeconds` the request passes
- mod and broadcaster bypass cooldown and per-user limit
- queue full rejects
- per-user limit counts only that user's pending items
- duplicate of an already-queued track rejects
- unmatched query returns `not_found` and does not mutate `lastRequestAt`

`tests/library-match.test.ts`:
- exact title matches
- partial and misspelled titles match ("nghtfall" → "Nightfall")
- a query matching nothing returns `null` rather than the least-bad option
- artist-name queries match

`npm test` must pass clean before Phase 12.

---

## Phase 12 — Packaging

`electron-builder.yml`: NSIS (Windows), dmg (macOS, `arm64` + `x64`), AppImage (Linux). `asar: true`, with `resources/overlay` and `resources/fonts` in `extraResources` so the overlay server can serve real files from disk.

Bump `productName` to `Bard`, `appId` to something reverse-DNS you own.

macOS builds need signing and notarization to open without a Gatekeeper warning. If no signing identity is configured, build unsigned and note in the README that macOS users must right-click → Open the first time.

`.gitignore`: `node_modules`, `out`, `dist`, `release`, `nowplaying.json`, `*.log`.

**README** must cover: what Bard does, the library-locked design and why, how to add the OBS browser source, how to pick an output device, and the fact that no audio is ever downloaded or streamed from a third party.

---

## Known Gotchas

Collected here because each one costs an hour if you hit it cold:

1. **`file://` in the renderer will not work.** Use the `bard://` protocol from Phase 2.2. `stream: true` in the privileges or seeking breaks.
2. **`enumerateDevices()` labels are empty** without media permission. See Phase 5.1.
3. **Only ever create one `AudioContext`.** Reuse it across tracks; swap `src` on the element instead of building a new graph.
4. **`MediaElementAudioSourceNode` can only be created once per element.** Create the element and the node once at module load, then only ever assign `audio.src`.
5. **Twitch `PING` must be answered** or you disconnect after ~5 minutes with no error event.
6. **`music-metadata` is ESM-only.** If the main bundle chokes, check `electron.vite.config.ts` externalization rather than downgrading the package.
7. **Don't `Promise.all` the whole library scan.** Pool of 8.
8. **Unsubscribe every IPC listener** in effect cleanup, or HMR duplicates them.
9. **`AudioContext` may start suspended** if nothing has been clicked. Call `resume()` on the first user gesture.
10. **Atomic-write `nowplaying.json`** (temp + rename) so OBS never reads a partial file.

---

## Post-MVP TODOs

- **YouTube live chat** — needs the Data API and an API key; different auth story than Twitch, worth its own design pass
- **Persistent queue** across restarts
- **Crossfade** — trivial with two elements and two `GainNode`s now that Web Audio is available; was a real project in rodio
- **Reply in chat** — requires OAuth and a bot account. Big trust jump from anonymous read-only; make it opt-in and explain what the token can do
- **Overlay themes** — let streamers swap the overlay HTML/CSS, or ship 2–3 presets
- **Library watch** — `chokidar` on the roots for live add/remove instead of manual rescan
- **Loudness normalization** — ReplayGain tags if present, else EBU R128 analysis on scan
- **Request history + stats** — most-requested tracks, top requesters

---

## Migration Notes — What Changed from the Rust/ratatui Spec

| Rust TUI | Electron | Why |
|---|---|---|
| `ratatui` + `crossterm` | React + Tailwind in a BrowserWindow | Album art, drag-to-reorder queue, and click-to-seek all become straightforward instead of impossible or awkward |
| `rodio` on a thread + mpsc | Web Audio in the renderer | Seeking works out of the box (no symphonia backend swap), and `setSinkId` gives real output-device routing |
| `fuzzy-matcher` | `fuse.js` | Same job, richer scoring |
| `tokio-tungstenite` | `ws` | Same anonymous IRC approach, unchanged |
| `nowplaying.json` polled by OBS | Local HTTP server + SSE, JSON kept as fallback | Push updates instead of file polling |
| Read-only settings panel | Fully editable settings | No reason to make the streamer edit TOML by hand once there's a real UI |
| No image support | Embedded cover art | The one thing explicitly given up in the TUI version |

**What deliberately did not change:** the library-locked request model, anonymous read-only Twitch IRC with no OAuth, the three-column information architecture, and per-user cooldowns with a block list. Those decisions were right the first time.

**Honest tradeoffs of this direction:** the shipped app is ~150 MB instead of a few MB, idle memory is ~200 MB instead of ~15 MB, and cold start is roughly a second rather than instant. For a tool that sits open for an entire stream on a machine already running OBS and a game, none of that is the binding constraint — but it's a real cost, and if the appeal of the Rust version was that it was tiny and lived in a terminal alongside everything else, that appeal is gone.
