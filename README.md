# Steam Press

Packing tool for Steam games, written in Rust.

Steam Press downloads Steam depots, produces clean app manifests, compresses the result and
generates cs.rin.ru release notes. It is a Rust reimplementation of the workflow popularised
by [SuperSteamPacker](https://github.com/Masquerade64/SuperSteamPacker), with the reliability
ideas of [OmniPacker](https://github.com/elgreams/OmniPacker).

Linux is the primary target. CI also checks and packages Windows and macOS builds; the GUI
updater installs automatically on Linux and macOS and points Windows users at the archive.

## Features

- Game queue with platform, branch and protected-branch support.
- Search the Steam store by name to fill an App ID, from the CLI (text or JSON) or the GUI
  (with Lucide icons throughout).
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
- Archive verification after compression (`verify_archives`), plus `sha256sum`-compatible
  checksum manifests for every packed archive.
- Disk-space preflight before downloading, resumable work directories with checkpoints, and a
  `--fresh` option to discard them.
- Shared Steamworks redistributables (`shareddepots`) are downloaded and get their own
  `appmanifest_228980.acf`.
- Import an existing local installation (`import`) or compare two builds (`diff`) and build an
  update archive with added and modified files plus a `deleted_files.txt` manifest.
- Global `--json` output for doctor, search, info, login, queue, workshop, depots, pack,
  template, import and diff.
- Encrypted local credential vault (AES-256-GCM).
- GUI queues and the finished-job history are saved automatically and restored at the next
  launch.
- Automatic GUI updates from the main `v*` releases, with SHA-256 verification and restart.

## Architecture

```
crates/steam-press-core   business rules, no GUI dependency
crates/steam-press-cli    `steam-press` command-line interface
crates/steam-press-gui    `steam-press-gui` egui desktop application
```

## Prerequisites

- Rust 1.88 or newer (2021 edition).
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

GitHub Actions publishes one release when the version in
`crates/steam-press-core/Cargo.toml` has no matching release yet. A build matrix produces the
same three archives for Linux, macOS (Intel and Apple Silicon) and Windows (x86_64).
Linux archives are built on Ubuntu 22.04 (glibc 2.35) for broad compatibility:

- `steam-press-<version>-<platform>.tar.gz` with both binaries, the README and the guide;
- `steam-press-cli-<version>-<platform>.tar.gz` with the CLI only;
- `steam-press-gui-<version>-<platform>.tar.gz` with the GUI only.

`<platform>` is `linux-x86_64`, `macos-x86_64`, `macos-aarch64` or `windows-x86_64`. Every archive has a
`.sha256` file, and `SHA256SUMS` covers every archive. To publish from a tag instead:

```bash
git tag -a v1.0.5 -m "Steam Press 1.0.5"
git push origin v1.0.5
```

The GUI checks the main `v*` releases on startup and from **Settings > Updates**. It downloads
the GUI archive for its platform, verifies its SHA-256 checksum, replaces the running
executable and restarts. Automatic installation is available on Linux and macOS; on Windows
the app reports the download URL to use manually. The `CI` workflow runs formatting, Clippy and
the test suite on Linux, and workspace checks on macOS and Windows.

## Quick start (CLI)

```bash
# Save Steam credentials (encrypted locally), or use --anonymous later.
steam-press login

# Find a game by name (add --json for scripts).
steam-press search "half-life"
steam-press search "half-life" --json

# Inspect an app.
steam-press info 2379780
steam-press info 2379780 --json

# Synchronize depot names.
steam-press depots sync

# Queue jobs and pack them.
steam-press queue add 2379780 --os lin64
steam-press pack --queue ~/.config/steam-press/queue.sspq --compressor 7z

# One-off job, dry run first, then resume or restart from scratch.
steam-press pack --appid 2379780 --os lin64 --dry-run
steam-press pack --appid 2379780 --os lin64 --downloader steamcmd
steam-press pack --appid 2379780 --os lin64 --fresh

# Import an existing installation and produce the usual archive.
steam-press import /games/MyGame --appid 2379780 --os lin64 --compressor 7z

# Compare two builds or write an update archive in the output directory.
steam-press diff ./build-a ./build-b
steam-press diff ./build-a ./build-b --archive --stem MyGame.patch

# Any command can emit machine-readable JSON.
steam-press doctor --json

# Workshop item.
steam-press workshop add 730 123456
steam-press workshop run --queue ~/.config/steam-press/queue.sswq
```

`steam-press doctor` checks the tools, directories, depot names, output writability, free
space, credential readability and Steam API reachability. Its JSON output includes
`diskFreeBytes`, `credentials` and `network`; see [docs/json-schema.md](docs/json-schema.md).

## Desktop application

```bash
cargo run -p steam-press-gui
```

The window has five sections: **Library** (game queue), **Workshop** (workshop queue),
**History** (finished, skipped, failed and cancelled jobs with their logs), **Depots** (depot
search and synchronization) and **Settings** (engine, compressor, paths, credentials, template,
theme). Games can be found by name in the Steam store, and every action uses a Lucide icon.
Downloads run in a worker thread; the activity panel streams the live log and the active queue
can be cancelled. Both queues and the history are saved automatically and restored on launch;
history entries can open their output folder or be queued again. The GUI also checks for
updates automatically and can install them from the Settings page.

## Configuration

- Config: `$XDG_CONFIG_HOME/steam-press/` (`settings.ini`, `depots.ini`, queues, templates).
- Data: `$XDG_DATA_HOME/steam-press/`.
- Work directory: `~/SteamPress/`, output in `~/SteamPress/Completed`.
- `STEAM_PRESS_HOME` overrides the whole layout.

`settings.ini` keeps SuperSteamPacker's `[SSP]` keys (`compressor`, `customcompressoption`,
`skipcompression`, `darkmode`, `uploadcrewmode`, `rinruusername`, `filehost`) and adds
`verifyarchives` (test every archive after compression), `diskmarginmb` (free-space margin
kept during the preflight) and `formatversion` (persisted format, currently `1`).

## Documentation

- [Guide d'utilisation](docs/usage.md)
- [Migration 0.x → 1.0](docs/migration-1.0.md)
- [JSON schema](docs/json-schema.md)
- [Exit codes](docs/exit-codes.md)
- [Security review](docs/security-review.md)
- [Changelog](CHANGELOG.md)

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

- The 1.0.1 validation downloaded Spacewar (app 480) anonymously with DepotDownloader and
  produced a verified 7z archive. SteamCMD's real download path could not run in the build
  environment because its 32-bit loader was unavailable; it remains covered by scripts,
  fake-engine tests and log classifiers.
- The GUI was exercised on Linux under X11 via Xvfb with the 1.0.2 build: all five sections
  were opened, both themes were switched, a job was added to the queue and restored after a
  restart. It has not been run under Wayland, Windows or macOS, and a complete real download
  has not yet been started from the GUI.
- Windows and macOS are compiled by CI but were not run locally; automatic updates install on
  Linux and macOS, while Windows users get the archive URL to install manually.
- Automatic updates require write access to the directory containing the GUI executable.
- Update archives contain a `deleted_files.txt` manifest, but the CLI does not delete the
  old files itself; the consumer must apply the manifest after extraction.
- Release assets are checksummed but not cryptographically signed yet.

## Credits

Inspired by [SuperSteamPacker](https://github.com/Masquerade64/SuperSteamPacker) (workflow,
outputs, templates) and [OmniPacker](https://github.com/elgreams/OmniPacker) (architecture,
templates, cancellation). Steam Press is an independent implementation and is not affiliated
with Valve or Steam.
