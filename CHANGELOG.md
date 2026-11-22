# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [1.0.4] - 2026-11-22

### Fixed

- SteamCMD no longer reports a Steam Guard failure when a successful login
  prints "Steam Guard must be approved before continuing" after the user
  confirms the login on the mobile app. A Steam Guard line is only treated as
  a failure when SteamCMD exits nonzero or the run does not log its success
  marker; a stale content directory from a previous attempt is not enough.
- The Steam Guard error now mentions mobile approval as well as a fresh code.

## [1.0.3] - 2026-11-21

### Added

- Steam Guard support for real accounts. `pack` and `workshop run` accept
  `--steam-guard-code`; the CLI prompts once for a fresh code in interactive
  mode, and the GUI exposes the code in Settings plus a Retry action on the
  failure banner. DepotDownloader receives the password and code on stdin,
  never on the command line.
- Exact offline snapshots for the `doctor`, `search`, `info`, `pack` and
  `import` JSON documents. Their builders are now pure functions, so every
  stable field is checked without network access or a Steam account.
- Screenshots of the five GUI sections and the light theme in the user guide;
  release archives now include `docs/screenshots/`.

### Changed

- `docs/json-schema.md` lists the five additional contract tests and the
  offline guarantee they provide.
- `docs/usage.md` documents the Steam Guard flow and the `--steam-guard-code`
  option.
- The DepotDownloader minimum version is now 3.3.0, the first release with
  `-no-mobile` for entering a Steam Guard code instead of approving on the
  mobile app.

### Fixed

- DepotDownloader prompts for Steam Guard are now reported as Steam Guard
  failures instead of a generic exit code, so the CLI and GUI can ask for a
  fresh code and retry.
- A freshly written sidecar that is briefly busy (`ETXTBSY`) is retried
  instead of failing the job immediately.

## [1.0.2] - 2026-11-14

### Added

- `doctor` now probes installed tool versions and enforces the documented
  minimums for 7-Zip 21.0, RAR 6.0 and DepotDownloader 3.0.0. SteamCMD has no
  stable semantic version and is checked for presence. Missing tools and
  outdated versions include a corrective action naming the right setting.
- Contract snapshots for the stable `--json` output of `queue list`,
  `workshop list`, `depots list`, `login`, `template render`, `diff` and CLI
  errors, so a field change fails CI.
- Tests for Steam Guard and interrupted fake downloads.

### Fixed

- `workshop list --json` no longer prints a human-readable line before the
  JSON document when the queue is empty.
- A cancellation arriving between the preflight and the start of the download
  is no longer lost.
- Steam Guard messages from SteamCMD are now reported as Steam Guard instead
  of a generic exit-code failure.

## [1.0.1] - 2026-11-14

### Fixed

- DepotDownloader content is now placed in
  `steamapps/common/<installdir>/`, matching SteamCMD, instead of the root of
  `steamapps/`. This also fixes the generated `SizeOnDisk` field.
- DepotDownloader's `.DepotDownloader` metadata and staging files are no longer
  included in packed output.

## [1.0.0] - 2026-11-14

### Added

- Archive integrity verification after compression, disk-space preflight,
  resumable checkpoints and `--fresh`.
- Local installation import and build diff with an update archive, including
  a `deleted_files.txt` manifest for removed files.
- Shared depot manifests (`appmanifest_228980.acf`).
- Stable `--json` output with `schemaVersion`, documented in
  [docs/json-schema.md](docs/json-schema.md).
- Persisted GUI history with logs, retry and output-folder access.
- Windows, macOS Intel, macOS Apple Silicon and Linux release archives.
- Format versioning and migrations for settings, queues, depot names,
  credentials, checkpoints and GUI history.
- `LICENSE`, `CONTRIBUTING.md`, `SECURITY.md`, exit-code documentation,
  migration guide and security review.

### Changed

- Linux release binaries are built on Ubuntu 22.04 (glibc 2.35).
- `diff --archive` includes `deleted_files.txt` when files were removed.

### Fixed

- Per-asset `.sha256` files are relative to their download directory.
- The GUI updater selects the archive for the current platform.

## [0.6.1] - 2026-11-14

### Fixed

- Build Linux release archives on Ubuntu 22.04 to avoid a glibc 2.43
  requirement.
- Write per-asset checksums relative to the archive directory.

## [0.6.0] - 2026-11-14

### Added

- The ten planned 0.6 features: archive verification, disk preflight,
  resumable downloads, ACF validation, shared depots, local import, build
  diff, global JSON output, GUI history and platform-aware builds.

## [0.5.0] - 2026-11-13

### Added

- Unified CLI and GUI release, automatic GUI updates, SHA-256 checksums and
  encrypted credentials.

## [0.1.0] - 2026-11-13

- First public CLI/GUI packaging workflow.
