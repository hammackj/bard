# Contributing

Thanks for your interest in Bard. Bug reports, fixes and small improvements
are welcome.

## Before you start

- **Bugs:** open an issue with what you did, what you expected and what
  happened, plus your distro and desktop (Bard is Linux-only, on PipeWire).
- **Features:** open an issue first. Bard is deliberately small: it plays
  local files, and things that fetch, stream or share music are out of scope.
  An issue saves you writing something that won't be merged.
- **Security problems:** see [SECURITY.md](SECURITY.md). Please don't open a
  public issue.

## Building

See [Requirements](README.md#requirements) for the system packages, then:

```bash
cargo run -p bard
```

Use `BARD_DATA_DIR=/some/dir` so a development build doesn't touch the
library, stats and settings of a Bard you use day to day.

## Pull requests

Every pull request must pass the same checks CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- **Bug fixes come with a test** that fails without the fix. Most of Bard's
  logic lives in `bard-core` and can be tested without audio or a window.
- **Keep changes focused.** Formatting-only changes go in their own commit.
- **Add a line to `CHANGELOG.md`** under `## [Unreleased]` for anything a user
  would notice.
- **Explain why** in commit messages, not just what changed.

By contributing, you agree that your contributions are licensed under the
project's [MIT licence](LICENSE).
