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

Linux with PipeWire, on Wayland or X11. Bard is developed on KDE Plasma
(Wayland); the tray needs a StatusNotifierItem host, which Plasma provides.

Building needs a stable Rust toolchain and the PipeWire, libopus and Wayland
development headers.

## Development

```bash
cargo run -p bard            # debug build
cargo run --release -p bard  # what you will actually ship
cargo test --workspace
```

Before committing, formatting and lints must pass clean (`rustfmt.toml` holds
the formatter settings):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The workspace is four crates:

| Crate           | What it holds                                                    |
| --------------- | ---------------------------------------------------------------- |
| `bard-core`     | types, library scan, queue, config and stores                    |
| `bard-audio`    | decode, PipeWire output, gain, fades, the visualiser's analyser  |
| `bard-services` | overlay server, `nowplaying.json`, OBS, MPRIS, tray, keyring     |
| `bard`          | the GPUI app: windows, views, and the glue between the above     |

Development environment variables:

- `BARD_DATA_DIR=/some/dir` — runs against that data directory instead of
  `~/.config/bard`, under its own MPRIS name and keyring entry, so a dev build
  can run beside the installed one without touching its library or stats
- `cargo run -p bard --features snapshot --bin snapshot` — renders the real
  views headless to PNG from a fixture, for checking a screen without a
  desktop

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
directory (`~/.config/bard`). It is written atomically, so OBS never reads a
half-written file, and only when the track, play state or queue length
changes — not on every progress tick.

### Choosing an output device

Settings → Audio → Output device lists PipeWire's sinks. To send music to a
virtual cable while your own monitoring stays separate, pick that device here.

If the saved device is missing at startup (a USB interface not yet plugged
in), Bard plays to the system default, keeps your setting, and switches over
by itself when the device appears.

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

## Installing

```bash
./scripts/install-desktop.sh
```

That builds the release binary, installs it to `~/.local/bin/bard`, and
registers a `.desktop` entry and the icon set in your per-user XDG
directories. Run it again after pulling to update the installed app; `cargo
build` alone does not touch the copy your launcher runs. Every build stamps in
its commit — the About section in Settings and the avatar menu show it next to
the version, with `-dirty` when the build carried uncommitted changes — so if
the app is not behaving like the source, check that first.

`--desktop` also drops a launcher on the desktop.

### Cutting a release

The workspace version in `Cargo.toml` is the single source of truth. Write the
release's entry in `CHANGELOG.md` first, then bump the version, and commit
both together so the tag points at the release:

```bash
git add -A && git commit -m "Bard 0.2.1"
git tag -a v0.2.1 -m "Bard 0.2.1"
git push && git push --tags
```

Bard is on a `0.x` line: minor bumps carry breaking changes, patch bumps do
not.

### The tray preview

The now-playing preview is a layer-shell surface on Wayland, so the compositor
puts it in the corner chosen in Settings; no window rule is needed.

## Where things live

```
crates/       the Rust workspace (see Development)
resources/
  overlay/    the OBS browser source page, embedded in the binary
  icons/      tray and launcher icons
docs/
  PORT.md     notes from the port off Electron, and where it deliberately differs
```

Some deliberate structure worth knowing before changing things:

- **The core owns the queue.** The window sends intents and renders what comes
  back, so there is one source of truth for what plays next.
- **The window never touches the core or the audio engine directly.** Views
  read a model and call a `Backend` trait, which is what lets the snapshot tool
  render the real views from fixture data.
- **Closing the window hides to the tray.** GPUI cannot hide a window on
  Linux, so it really closes; its state is parked and a new window is built
  from it when the tray asks.

## Licence

The application code is MIT — see [`LICENSE`](LICENSE). The bundled fonts
(Inter, Fira Code) are under the SIL Open Font License and two icons are
adapted from Feather (MIT); see [`THIRD-PARTY.md`](THIRD-PARTY.md).

Music you play through it is governed by whatever licence that music carries —
Bard neither supplies nor relicenses any audio.
