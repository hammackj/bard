# Security

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub:
**[Report a vulnerability](https://github.com/hammackj/bard/security/advisories/new)**
(the repository's Security tab → _Report a vulnerability_). Don't open a public
issue.

Include what you found, how to reproduce it, and which version or commit you
tested. Fixes go into the next release, and the advisory is published once
that release is out.

## Supported versions

Only the latest release is supported. Bard is on a `0.x` line and fixes are not
backported.

## What is in scope

Bard is a desktop app with a few parts worth a close look:

- **The overlay server** binds to `127.0.0.1` only and serves the now-playing
  card, its cover art and an event stream to OBS. Anything that exposes it
  beyond the local machine, or lets a page in a browser read the library
  through it, is in scope.
- **The OBS connection** (obs-websocket v5). The password is stored in the
  system keyring and is never logged or written to disk in the clear.
- **Files Bard writes**: config, stats and `nowplaying.json` under
  `~/.config/bard`, and the cover art cache.
- **Media files**: a crafted audio file or embedded image that crashes Bard,
  hangs it or worse while it scans or plays.

The music itself, and whether it is licensed for your stream, is out of scope:
Bard plays what you give it.
