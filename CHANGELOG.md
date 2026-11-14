# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

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
