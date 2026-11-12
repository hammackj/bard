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
| React + Tailwind renderer        | GPUI views (`gpui-pre` + `gpui-component`) |
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
- **Not carried over:** `audio.outputDeviceId` (a Chromium-hashed id; falls
  back to the default sink once) and `obs-password.bin` (Electron
  safeStorage; re-enter it, or migrate by decrypting Chromium's v11 format).

## Fixed from Electron that must stay fixed

- MPRIS / lock screen shows the real title, artist, album and cover — the
  Electron build showed "Bard / Bard" until 0.1.3.
- The overlay binds 127.0.0.1, never 0.0.0.0.
- Global media keys stay off by default.
