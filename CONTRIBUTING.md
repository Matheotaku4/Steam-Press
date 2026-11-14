# Contributing to Steam Press

## Prerequisites

- Rust stable, at least the MSRV in `Cargo.toml` (currently 1.88).
- `cargo fmt` and `cargo clippy`.
- Optional for manual runs: SteamCMD, 7-Zip, RAR and DepotDownloader.

## Build and checks

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The test suite is offline. Do not require a Steam account for a test.

## Pull requests

- Keep changes focused and explain the user-visible effect.
- Add or update tests for behaviour changes.
- Update `CHANGELOG.md` under `Unreleased` for user-visible changes.
- Preserve the stable file formats and the `--json` schema; add a migration
  when a format changes.
- Do not commit credentials, `.env` files, generated archives, or test fixtures
  containing a real account.

## Style

Rust 2021, `cargo fmt`, no Clippy warnings. UI and source identifiers are in
English; user documentation is in French (the README is bilingual in practice,
with an English overview and French guide).

## Releases

A release is produced by the GitHub Actions `Release` workflow when the
workspace version has no matching `v*` release. Tags and assets are generated
from the version in `crates/steam-press-core/Cargo.toml`.
