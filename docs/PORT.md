# Porting Bard to Rust and GPUI

Work happens on the `gpui` branch. The Electron app stays on `master`, working
and shippable, until the Rust build reaches parity; only then is the Electron
code removed. Linux only for now (KDE / Wayland, PipeWire).

**Goal: the Rust app looks exactly like the Electron one.** Every screen is
checked side by side against a screenshot of the Electron build.

## Crates

```
crates/
  bard-core/      types, pure logic, library scan, queue, config and stores
                  (the Electron main process minus I/O to the OS)
  bard-audio/     decode (symphonia, libopus), PipeWire output, gain, fades,
                  ReplayGain, analyser ring buffer for the visualiser
  bard-services/  overlay HTTP/SSE server, nowplaying.json, OBS websocket,
                  MPRIS, tray, keyring
  bard/           the GPUI app: windows, views, and the glue between the above
```

| Electron                         | Rust                                      |
| -------------------------------- | ----------------------------------------- |
| React + Tailwind renderer        | GPUI views (`gpui-pre`)                   |
| Web Audio graph                  | `bard-audio`                              |
| `music-metadata`                 | `lofty`                                   |
| `fuse.js`                        | fuzzy matcher in `bard-core`              |
| `electron-store`                 | serde_json, same file and shape           |
| express + SSE                    | `axum`                                    |
| `ws` (OBS)                       | `tokio-tungstenite`                       |
| Electron safeStorage             | `keyring` (Secret Service)                |
| Chromium's implicit MPRIS        | `mpris-server`                            |
| Electron Tray                    | `ksni`                                    |
| `src/shared/ipc.ts` `BardApi`    | `bard_core::Bard` methods + `CoreEvent`   |

## Compatibility with an existing install

The Rust build reads and writes the same `~/.config/bard`:

- `config.json`, `stats.json`, `albums.json`, `loved.json` — same JSON shape.
  Types in `bard-core/src/types.rs` are verified against a real install.
- `artwork/<trackId>.jpg` — track ids must stay
  `sha1(resolved path).hex[..16]`, or stats and covers detach from tracks.
- `nowplaying.json` and the overlay's SSE payloads — same shape, so the OBS
  browser source page in `resources/overlay/` is served unchanged.
- **Not carried over:** `audio.outputDeviceId`. Electron stored a
  Chromium-hashed id that names no PipeWire sink, so the first Rust run plays
  through the default with a notice; pick the device once in Settings.
- `obs-password.bin` (Electron safeStorage) is migrated once into the keyring
  by `bard-services` and left in place for the Electron build; a
  `obs-password.bin.migrated` marker stops a later clear being undone.

## Fixed from Electron that must stay fixed

- MPRIS / lock screen shows the real title, artist, album and cover — the
  Electron build showed "Bard / Bard" until 0.1.3.
- The overlay binds 127.0.0.1, never 0.0.0.0.
- Global media keys stay off by default.

## Deliberate differences from Electron

- The "Global media keys" checkbox is kept for the config file's sake but does
  nothing: on Linux the media keys reach Bard through MPRIS like any player.
- Tooltips are drawn in the app's panel style, not Chromium's native bubble.
- With a track row focused, Up/Down move the selection only. Electron's
  window-level handler also changed the volume on the same keypress.
- A press on a scrollbar track pages once; Chromium repeats while held.

## Building and running

```
cargo build --release -p bard
./scripts/install-desktop.sh target/release/bard   # point the launcher at it
```

The launcher and icon are the Electron build's: the window's app id is
`bard`, matching `bard.desktop`. Run `install-desktop.sh` with no argument to
point the launcher back at `dist/linux-unpacked/bard`.

GPUI's Wayland and X11 backends are opt-in features of `gpui-pre-platform`.
Without them it runs headless without a word -- audio, MPRIS and the tray all
work, and no window ever appears.

One Bard per data dir: a second launch finds the first by its MPRIS name,
asks it to `Raise`, and exits before opening anything. SIGTERM, SIGINT and
SIGHUP quit as the tray's Quit does: services stopped, stats and loved
tracks flushed, a stopped `nowplaying.json` written.

### Development switches

| Variable | Effect |
| --- | --- |
| `BARD_DATA_DIR` | Use this data dir instead of `~/.config/bard`. Also a dev run: its own MPRIS name (per dir), its own keyring slot, no OBS password migration, no OBS watcher. |
| `BARD_SILENT` | Everything runs and the visualiser moves, but the device gets zeros. |
| `BARD_AUTOPLAY` | Start playback once the library is scanned. |
| `BARD_OBS` | With `BARD_DATA_DIR`, run the OBS watcher anyway. |
| `BARD_POPUP_DEBUG` | Log how the tray preview opened. |
| `BARD_POPUP_PLAIN` | Open the preview as a plain window instead of a layer-shell surface. |

Give a scratch data dir its own `overlay.port` in `config.json`, or a dev run
can take 4747 while the installed Bard is closed and feed OBS.

`bard-snapshot` (`--features snapshot`) renders a view headlessly from a
fixture, with optional keystrokes, clicks and a drag, for side-by-side checks
against the Electron build.
