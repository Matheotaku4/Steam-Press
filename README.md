# Steam Press

Packing tool for Steam games, written in Rust.

Steam Press downloads Steam depots, produces clean app manifests, compresses the result and
generates cs.rin.ru release notes. It is a Rust reimplementation of the workflow popularised
by [SuperSteamPacker](https://github.com/Masquerade64/SuperSteamPacker), with the reliability
ideas of [OmniPacker](https://github.com/elgreams/OmniPacker).

Linux is the primary target of this first version. The code stays portable so Windows and
macOS support can be added later.

## Features

- Game queue with platform, branch and protected-branch support.
- Workshop queue (`workshop_download_item`).
- Two download engines behind one abstraction: **SteamCMD** (default) and **DepotDownloader**.
- `.acf` reading, privacy sanitizing (`LastOwner`/`LauncherPath` set to `"0"`) and full
  manifest generation, including `appmanifest_228980.acf` for redistributables.
- 7-Zip and RAR archives with SuperSteamPacker's defaults (5 GB volumes, `cs.rin.ru`
  password, encrypted headers) or custom options.
- SuperSteamPacker-compatible queues: `.SSPQ` (`os|appid|branch|password`) and
  `.SSWQ` (`appid|workshopid`).
- Built-in `[CS.RIN.RU Info]` / `[Upload Crew Info]` templates plus editable token templates.
- Depot names synchronized from
  [SteamDepotNames](https://github.com/Masquerade64/SteamDepotNames).
- Cancellation during download or compression; partial archives are kept.
- Encrypted local credential vault (AES-256-GCM).

## Architecture

```
crates/steam-press-core   business rules, no GUI dependency
crates/steam-press-cli    `steam-press` command-line interface
crates/steam-press-gui    `steam-press-gui` egui desktop application
```

## Prerequisites

- Rust stable (2021 edition).
- `steamcmd` (`steamcmd.sh`) in `PATH`, or configured in the settings.
- `7z` (`7zz`/`7za` also work) for compression.
- Optional: a `rar` binary for RAR archives (RAR is proprietary; bring your own binary).
- Optional: `DepotDownloader` in `PATH` to use the second engine.

## Build

```bash
cargo build --release -p steam-press-cli -p steam-press-gui
```

Binaries land in `target/release/steam-press` and `target/release/steam-press-gui`.

## Releases

GitHub Actions publishes the Linux binaries when the version in
`crates/steam-press-core/Cargo.toml` does not have a release yet, and when a matching `v*`
tag is pushed. To force a release after a manual version bump:

```bash
git tag -a v0.1.0 -m "Steam Press 0.1.0"
git push origin v0.1.0
```

Each release contains `steam-press`, `steam-press-gui`, the README, the usage guide and a
`.sha256` checksum. The `CI` workflow runs formatting, Clippy and the test suite on every
push to `main` and on pull requests.

## Quick start (CLI)

```bash
# Save Steam credentials (encrypted locally), or use --anonymous later.
steam-press login

# Inspect an app.
steam-press info 2379780

# Synchronize depot names.
steam-press depots sync

# Queue jobs and pack them.
steam-press queue add 2379780 --os lin64
steam-press pack --queue ~/.config/steam-press/queue.sspq --compressor 7z

# One-off job, dry run first.
steam-press pack --appid 2379780 --os lin64 --dry-run
steam-press pack --appid 2379780 --os lin64 --downloader steamcmd

# Workshop item.
steam-press workshop add 730 123456
steam-press workshop run --queue ~/.config/steam-press/queue.sswq
```

`steam-press doctor` checks the tools, directories, depot names and output writability.

## Desktop application

```bash
cargo run -p steam-press-gui
```

The window has four sections: **Library** (game queue), **Workshop** (workshop queue),
**Depots** (depot search and synchronization) and **Settings** (engine, compressor, paths,
credentials, template, theme). Downloads run in a worker thread; the activity panel streams
the live log and the active queue can be cancelled.

## Configuration

- Config: `$XDG_CONFIG_HOME/steam-press/` (`settings.ini`, `depots.ini`, queues, templates).
- Data: `$XDG_DATA_HOME/steam-press/`.
- Work directory: `~/SteamPress/`, output in `~/SteamPress/Completed`.
- `STEAM_PRESS_HOME` overrides the whole layout.

`settings.ini` keeps SuperSteamPacker's `[SSP]` keys (`compressor`, `customcompressoption`,
`skipcompression`, `darkmode`, `uploadcrewmode`, `rinruusername`, `filehost`).

Credentials are encrypted with AES-256-GCM using a locally generated key and stored in
`credentials.bin` with mode `0600`. They are never logged and never passed on a command line
to DepotDownloader.

## Templates

A custom template is a text file selected in the settings. Tokens:

`{{game_name}}`, `{{os}}`, `{{branch}}`, `{{build_id}}`, `{{build_datetime_utc}}`,
`{{upload_datetime_utc}}`, `{{app_id}}`, `{{description}}`, `{{website}}`, `{{username}}`,
`{{filehost}}`, `{{upload_date}}`, `{{depot_list}}`, `{{primary_depot_id}}`,
`{{primary_manifest_id}}`.

Per-depot loop:

```
{{#depots}}- {{depot_id}} — {{depot_name}} [Manifest {{manifest_id}}]
{{/depots}}
```

Without a custom template, the built-in SuperSteamPacker renders are used.

## Tests

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
```

The test suite is offline: Steam responses come from fixtures and no account is required.

## Known limitations

- No real Steam download was executed while developing this version (no account in the build
  environment). Download engines are covered through their generated scripts/arguments and
  log classifiers.
- The GUI compiles but was not opened in the development environment (no display server).
- Shared-depot handling is read-only: existing `SharedDepots` are preserved, but Steamworks
  redistributable rows are not synthesized yet.
- Windows and macOS are not validated.

## Credits

Inspired by [SuperSteamPacker](https://github.com/Masquerade64/SuperSteamPacker) (workflow,
outputs, templates) and [OmniPacker](https://github.com/elgreams/OmniPacker) (architecture,
templates, cancellation). Steam Press is an independent implementation and is not affiliated
with Valve or Steam.
