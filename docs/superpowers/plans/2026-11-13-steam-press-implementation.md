# Steam Press Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recreate SuperSteamPacker in Rust as a tested core library, a headless CLI, and an egui desktop app, with SteamCMD and DepotDownloader engines.

**Architecture:** One Cargo workspace with three crates. `steam-press-core` owns every business rule and has no GUI dependency; `steam-press-cli` and `steam-press-gui` are thin frontends that exchange commands and progress events with the core.

**Tech Stack:** Rust 2021, serde/serde_json, reqwest (blocking, rustls), chrono, aes-gcm, rand, zip, clap, eframe/egui.

**Spec:** `docs/superpowers/specs/2026-11-13-steam-press-design.md`

## Global Constraints

- Linux is the v1 target; no OS-specific code paths outside `config`/`tooling`. Windows/macOS must remain plausible.
- No Tauri, no WebKit, no GUI dependency inside `steam-press-core`.
- Outputs must stay compatible with SuperSteamPacker: `.SSPQ` (`os|appid|branch|password`), `.SSWQ` (`appid|workshopid`), `<Game>.Build.<build>.<OS>.<branch>` stems, 7z password `cs.rin.ru`, 5 GB volumes.
- Steam privacy fields `LastOwner` and `LauncherPath` are always written as `"0"`.
- Credentials never appear in logs, events, error messages, or committed files.
- Every stage is cancellable through a process kill plus an atomic flag; partial archives are kept.
- Tests never touch the network or a real Steam account.
- Commit after every task with the repository's existing message style.

---

### Task 1: Workspace and pure foundations

**Files:**
- Create: `Cargo.toml`
- Create: `crates/steam-press-core/Cargo.toml`
- Create: `crates/steam-press-core/src/lib.rs`
- Create: `crates/steam-press-core/src/ini.rs`
- Create: `crates/steam-press-core/src/platform.rs`
- Create: `crates/steam-press-core/src/naming.rs`

**Interfaces:**
- Produces: `Ini::parse(&str)`, `Ini::load(path) -> io::Result<Ini>`, `Ini::save(path)`, `Ini::get(section, key) -> Option<&str>`, `Ini::set(section, key, value)`, `Ini::remove(section, key)`, `Ini::to_string()`.
- Produces: `Platform` enum (`Win64`, `Win32`, `Linux64`, `Linux32`, `Mac`) with `key()`, `label()`, `os_tag()`, `from_key()`, `from_label()`, `steamcmd_directives()`.
- Produces: `sanitize_game_name(&str) -> String`, `sanitize_install_dir(&str) -> String`, `output_stem(game, build, platform, branch) -> String`, `archive_stem`.
- Produces: `test_support::temp_dir(name) -> PathBuf` (unique directory under the system temp dir, created on demand, used by every test).

- [ ] **Step 1: Create the workspace and core manifest**

```toml
# Cargo.toml
[workspace]
resolver = "2"
members = ["crates/steam-press-core"]
```

```toml
# crates/steam-press-core/Cargo.toml
[package]
name = "steam-press-core"
version = "0.1.0"
edition = "2021"
license = "MIT"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
chrono = { version = "0.4", features = ["serde"] }
aes-gcm = "0.10"
rand = "0.8"
zip = { version = "2", default-features = false, features = ["deflate"] }
```

- [ ] **Step 2: Write failing tests for INI, platform and naming**

Put these in the respective modules:

```rust
// ini.rs tests
#[test]
fn parses_bom_comments_crlf_and_duplicate_keys() {
    let doc = Ini::parse("\u{feff}; comment\r\n[SSP]\r\ncompressor=7z\r\ncompressor=RAR\r\n\r\n[depots]\r\n10 = Counter-Strike\r\n");
    assert_eq!(doc.get("SSP", "compressor"), Some("RAR"));
    assert_eq!(doc.get("depots", "10"), Some("Counter-Strike"));
    assert!(doc.get("SSP", "missing").is_none());
}

#[test]
fn round_trips_sections_and_values() {
    let mut doc = Ini::parse("[SSP]\nlanguage=English\n");
    doc.set("SSP", "compressor", "RAR");
    let reparsed = Ini::parse(&doc.to_string());
    assert_eq!(reparsed.get("SSP", "language"), Some("English"));
    assert_eq!(reparsed.get("SSP", "compressor"), Some("RAR"));
}
```

```rust
// platform.rs tests
#[test]
fn maps_platform_keys_labels_and_tags() {
    assert_eq!(Platform::from_key("win64"), Some(Platform::Win64));
    assert_eq!(Platform::from_label("Linux x64"), Some(Platform::Linux64));
    assert_eq!(Platform::Linux64.os_tag(), "Linux64");
    assert_eq!(Platform::Mac.key(), "macos");
}

#[test]
fn emits_steamcmd_directives() {
    assert_eq!(
        Platform::Win64.steamcmd_directives(),
        vec!["@sSteamCmdForcePlatformType windows", "@sSteamCmdForcePlatformBitness 64"]
    );
    assert_eq!(Platform::Mac.steamcmd_directives(), vec!["@sSteamCmdForcePlatformType macos"]);
}
```

```rust
// naming.rs tests
#[test]
fn sanitizes_like_supersteampacker() {
    assert_eq!(sanitize_game_name("Half-Life 2"), "Half-Life.2");
    assert_eq!(sanitize_game_name("Assassin's Creed"), "Assassins.Creed");
    assert_eq!(sanitize_game_name("Who's Your Daddy?!"), "Whos.Your.Daddy!");
    assert_eq!(sanitize_game_name("S.T.A.L.K.E.R."), "S.T.A.L.K.E.R");
}

#[test]
fn builds_output_stem() {
    assert_eq!(
        output_stem("Portal 2", "5781", Platform::Linux64, "public"),
        "Portal.2.Build.5781.Linux64.public"
    );
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p steam-press-core`
Expected: FAIL with unresolved `Ini`, `Platform`, `sanitize_game_name`.

- [ ] **Step 4: Implement the three modules**

`Ini` stores `Vec<(String, Vec<(String, String)>)>`. Parsing trims a leading BOM, skips blank lines and lines starting with `;` or `#`, reads `[section]` and `key=value` via `split_once('=')`, and replaces an existing key case-insensitively. Writing emits sections separated by one blank line, `key=value`, and a trailing newline. `load` uses `fs::read_to_string`, `save` creates parent directories.

`Platform` derives `Serialize, Deserialize` with lowercase renames, and maps: `win64`/`Windows x64`/`Win64`/`windows` + 64-bit; `win32`/`Windows x86`/`Win32`/`windows` + 32-bit; `lin64`/`Linux x64`/`Linux64`; `lin32`/`Linux x86`/`Linux32`; `macos`/`Mac`/`Mac`.

Naming rules: spaces become dots; apostrophes, non-ASCII, control characters and `< > : " / \ | ? *` are removed; trailing dots and spaces are trimmed. `sanitize_install_dir` keeps spaces. `output_stem` uses `Unknown` when the build is empty and the raw branch.

Add `pub mod test_support` behind `#[cfg(any(test, feature = "test-support"))]`, exposing `temp_dir(name)` built from `std::env::temp_dir()`, `std::process::id()` and an atomic counter so parallel tests never collide.

- [ ] **Step 5: Run tests, then commit**

Run: `cargo test -p steam-press-core`
Expected: PASS (all tests).
Run: `git add Cargo.toml crates/steam-press-core && git commit -m "feat: add workspace and pure core foundations"`

---

### Task 2: Configuration, settings and credential vault

**Files:**
- Create: `crates/steam-press-core/src/config.rs`
- Create: `crates/steam-press-core/src/credentials.rs`
- Modify: `crates/steam-press-core/src/lib.rs`

**Interfaces:**
- Consumes: `Ini` from Task 1.
- Produces: `Paths { config_dir, data_dir, work_dir, output_dir }`, `Paths::discover()`, `Paths::ensure()`.
- Produces: `Settings` with the fields in the spec, `Settings::load(&Paths) -> Settings`, `Settings::save(&Paths) -> io::Result<()>`, `Settings::archive_password()`.
- Produces: `credentials::generate_key() -> String`, `encrypt(plain, key) -> Result<String, String>`, `decrypt(encoded, key) -> Result<String, String>`, `CredentialStore::save/load/clear`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn settings_round_trip_and_defaults() {
    let tmp = tempfile_dir("settings");
    let paths = Paths::under(&tmp);
    let mut settings = Settings::default();
    settings.compressor = Compressor::Rar;
    settings.skip_compression = true;
    settings.uploader = "alice".into();
    settings.ensure_key();
    settings.save(&paths).unwrap();
    let loaded = Settings::load(&paths);
    assert_eq!(loaded.compressor, Compressor::Rar);
    assert!(loaded.skip_compression);
    assert_eq!(loaded.uploader, "alice");
    assert_eq!(loaded.compress_password, "cs.rin.ru");
    assert!(!loaded.key.is_empty());
}

#[test]
fn credentials_round_trip_and_wrong_key_fails() {
    let key = generate_key();
    let encoded = encrypt("s3cret", &key).unwrap();
    assert!(!encoded.contains("s3cret"));
    assert_eq!(decrypt(&encoded, &key).unwrap(), "s3cret");
    assert!(decrypt(&encoded, &generate_key()).is_err());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p steam-press-core settings_round_trip`
Expected: FAIL, `Settings` not found.

- [ ] **Step 3: Implement `config.rs` and `credentials.rs`**

`Paths::discover()` reads `STEAM_PRESS_HOME` first; otherwise uses `XDG_CONFIG_HOME` or `~/.config/steam-press`, `XDG_DATA_HOME` or `~/.local/share/steam-press`, with `work_dir = data_dir/work` and `output_dir = ~/SteamPress/Completed` unless the settings override it. `Paths::under(root)` is a test helper that puts every directory under `root`.

`Settings` maps to `[SSP]` keys: `compressor`, `customcompressoption`, `skipcompression`, `darkmode`, `uploadcrewmode`, `rinruusername`, `filehost`, `outputdir`, `steamcmdpath`, `depotdownloaderpath`, `sevenzippath`, `rarpath`, `compresspassword`, `template`, `key`. Missing values fall back to `7z`, `cs.rin.ru`, empty paths, dark mode on. `ensure_key()` generates a base64 32-byte key when absent.

Credentials use `Aes256Gcm` with a random 12-byte nonce prefixed to the ciphertext and base64-standard encoding. `CredentialStore` persists `username\npassword` in `credentials.bin` with mode `0600` on Unix.

- [ ] **Step 4: Run tests, commit**

Run: `cargo test -p steam-press-core`
Expected: PASS.
Run: `git add crates/steam-press-core && git commit -m "feat: add settings and encrypted credential vault"`

---

### Task 3: Queue formats and platform directives

**Files:**
- Create: `crates/steam-press-core/src/queue.rs`
- Create: `crates/steam-press-core/src/tooling.rs`
- Modify: `crates/steam-press-core/src/lib.rs`

**Interfaces:**
- Produces: `GameJob { platform, appid, branch, branch_password }` with `encode() -> String`, `decode(&str) -> Result<GameJob, String>`, `validate() -> Result<(), String>`, `is_public_branch()`.
- Produces: `WorkshopJob { appid, workshop_id }` with `encode`, `decode` accepting both `appid|id` and `workshop_download_item app id validate`.
- Produces: `load_game_queue(path) -> Result<Vec<GameJob>, String>`, `save_game_queue(path, &[GameJob])`, same for workshop.
- Produces: `tooling::find_executable(explicit: Option<&Path>, names: &[&str]) -> Option<PathBuf>`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn round_trips_sspq_lines() {
    let job = GameJob::new(Platform::Linux64, "2379780").with_branch("public", "");
    assert_eq!(job.encode(), "lin64|2379780|public|");
    assert_eq!(GameJob::decode(&job.encode()).unwrap(), job);
}

#[test]
fn tolerates_short_and_legacy_lines() {
    let job = GameJob::decode("win64|123").unwrap();
    assert_eq!(job.branch, "Public");
    let workshop = WorkshopJob::decode("workshop_download_item 730 123456 validate").unwrap();
    assert_eq!(workshop, WorkshopJob { appid: "730".into(), workshop_id: "123456".into() });
}

#[test]
fn rejects_non_numeric_appids() {
    assert!(GameJob::decode("lin64|abc|public|").unwrap_err().contains("AppID"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-core round_trips_sspq`
Expected: FAIL, `GameJob` not found.

- [ ] **Step 3: Implement the modules**

`GameJob::decode` splits on `|`, requires at least two fields, accepts two to four, defaults the branch to `Public` and the password to empty, and validates that the appid parses as `u64` and is non-zero. Queue loading ignores empty lines, `;`/`#` comments and malformed lines (returned as warnings). Saving writes one encoded job per line with a trailing newline.

`WorkshopJob::decode` accepts a pipe line or a `workshop_download_item` script line, ignoring tokens such as `validate`.

`tooling::find_executable` checks the explicit path first, then each name in `PATH`, on Unix also trying `/usr/bin`, `/usr/local/bin` and `/snap/bin`, and returns an existing file path.

- [ ] **Step 4: Test, commit**

Run: `cargo test -p steam-press-core`
Expected: PASS.
Run: `git add crates/steam-press-core && git commit -m "feat: add compatible game and workshop queues"`

---

### Task 4: Depot names and Steam metadata

**Files:**
- Create: `crates/steam-press-core/src/depots.rs`
- Create: `crates/steam-press-core/src/steam.rs`
- Create: `crates/steam-press-core/tests/fixtures/appinfo.json`
- Create: `crates/steam-press-core/tests/fixtures/store.json`
- Modify: `crates/steam-press-core/src/lib.rs`

**Interfaces:**
- Produces: `DepotNames` with `parse(&str)`, `load`, `get(&str) -> Option<&str>`, `name_or_unknown`, `merge(&DepotNames) -> usize`, `save`, `sync_from_url(url) -> Result<usize, String>`.
- Produces: `AppInfo { appid, name, install_dir, build_id, build_time, depots }`, `DepotEntry { depot_id, name, manifest_id, dlc_appid }`, `StoreInfo { name, short_description, website }`.
- Produces: `parse_app_info(appid, branch, json) -> Result<AppInfo, String>`, `parse_store_info(appid, json)`, `fetch_app_info(appid, branch)`, `fetch_store_info(appid)`.

- [ ] **Step 1: Add fixtures and failing tests**

`appinfo.json` mirrors `api.steamcmd.net/v1/info/2379780`: `status=success`, `data.2379780.common.name=Balatro`, `config.installdir=Balatro`, `depots.branches.public.buildid=18674832`, `timeupdated=1740441718`, depots `228989` (Steamworks) and `2379781` (`Balatro`, manifest `4851806656204679952`, dlcappid absent).

```rust
#[test]
fn parses_balatro_appinfo() {
    let json = include_str!("../tests/fixtures/appinfo.json");
    let info = parse_app_info("2379780", "public", json).unwrap();
    assert_eq!(info.name, "Balatro");
    assert_eq!(info.install_dir.as_deref(), Some("Balatro"));
    assert_eq!(info.build_id, "18674832");
    assert_eq!(info.depots.len(), 2);
    assert_eq!(info.depots[1].manifest_id, "4851806656204679952");
}

#[test]
fn unknown_branch_keeps_unknown_build() {
    let json = include_str!("../tests/fixtures/appinfo.json");
    let info = parse_app_info("2379780", "beta", json).unwrap();
    assert_eq!(info.build_id, "Unknown");
    assert!(info.build_time.is_none());
}
```

```rust
#[test]
fn parses_depot_names_and_merges_without_unknown() {
    let mut names = DepotNames::parse("[depots]\n10 = Counter-Strike\n11 = Unknown\n");
    assert_eq!(names.get("10"), Some("Counter-Strike"));
    assert!(names.get("11").is_none());
    let other = DepotNames::parse("[depots]\n10 = Other\n12 = New\n");
    assert_eq!(names.merge(&other), 1);
    assert_eq!(names.get("10"), Some("Counter-Strike"));
    assert_eq!(names.get("12"), Some("New"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-core parses_balatro`
Expected: FAIL, `parse_app_info` missing.

- [ ] **Step 3: Implement parsing and network calls**

`DepotNames` uses a `BTreeMap<u32, String>`, reads only the `[depots]` section, ignores `Unknown`, and writes sorted `id = name` lines. `sync_from_url` fetches with a 30-second timeout and a `SteamPress/0.1` user agent.

`parse_app_info` walks `data.<appid>`; name and install dir default to empty/`None`; branch build/time are read from `depots.branches.<branch>`; every numeric depot key with a `manifests.<branch>.gid` becomes a `DepotEntry` using `name`, `gid`, and `dlcappid`. A branch named `public` falls back to `public` when the requested branch is missing. Network functions build a blocking client with 15-second timeout and return contextual errors.

`fetch_store_info` reads `success`, `data.name`, `data.short_description`, `data.website`, normalizing `http://` to `https://` only when the scheme is exactly `http://`.

- [ ] **Step 4: Test, commit**

Run: `cargo test -p steam-press-core`
Expected: PASS.
Run: `git add crates/steam-press-core && git commit -m "feat: add depot names and Steam metadata parsing"`

---

### Task 5: VDF and ACF handling

**Files:**
- Create: `crates/steam-press-core/src/vdf.rs`
- Create: `crates/steam-press-core/src/acf.rs`
- Create: `crates/steam-press-core/tests/fixtures/appmanifest_2379780.acf`
- Modify: `crates/steam-press-core/src/lib.rs`

**Interfaces:**
- Produces: `vdf::Value::{Str(String), Object(Vec<(String, Value)>)}` with `get(&str)`, `get_str`, `as_str`; `parse(&str) -> Result<Value, String>`; `to_string()`.
- Produces: `InstalledDepot { depot_id, manifest, size }`, `AppManifest { appid, name, build_id, installdir, installed_depots }`.
- Produces: `read_manifest(path) -> Result<AppManifest, String>`, `rewrite_privacy_fields(path) -> Result<bool, String>`, `write_acf(dir, &AcfJob) -> Result<PathBuf, String>`, `write_shared_acf(dir, &AcfJob) -> Result<Option<PathBuf>, String>`.

- [ ] **Step 1: Add fixture and failing tests**

Fixture contains `AppState` with `appid`, `name`, `buildid`, `LastOwner "123"`, `LauncherPath "C:\\..."`, `InstalledDepots` with depot `2379781` (`manifest`, `size`), `UserConfig`, `MountedConfig`.

```rust
#[test]
fn reads_manifest_fields_and_depots() {
    let manifest = read_manifest(Path::new("tests/fixtures/appmanifest_2379780.acf")).unwrap();
    assert_eq!(manifest.name, "Balatro");
    assert_eq!(manifest.build_id, "18674832");
    assert_eq!(manifest.installed_depots[0].manifest, "4851806656204679952");
}

#[test]
fn rewrites_privacy_fields_idempotently() {
    let dir = tempfile_dir("acf");
    let path = dir.join("appmanifest_2379780.acf");
    fs::write(&path, include_str!("../tests/fixtures/appmanifest_2379780.acf")).unwrap();
    assert!(rewrite_privacy_fields(&path).unwrap());
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"LastOwner\"\t\t\"0\""));
    assert!(text.contains("\"LauncherPath\"\t\t\"0\""));
    assert!(!rewrite_privacy_fields(&path).unwrap());
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-core reads_manifest_fields`
Expected: FAIL, `read_manifest` missing.

- [ ] **Step 3: Implement parser, serializer and ACF helpers**

Tokenizer handles quoted strings with `\\`/`\"` escapes, `{`, `}`, unquoted tokens, `//` comments and whitespace. `to_string` indents with tabs and always quotes keys and values.

`read_manifest` requires `AppState`; reads `appid`, `name`, `buildid`, `installdir`, and each `InstalledDepots.<id>` manifest/size.

`rewrite_privacy_fields` parses, sets both keys to `"0"` under `AppState` when present (adding them if missing), serializes, compares with the original, writes only on change, and returns whether a write happened.

`AcfJob` carries appid, game name, install dir, build id, depots (id/name/manifest/size/dlcappid), total size, shared depots and install scripts. `write_acf` emits `AppState` with `StateFlags 4`, `LastUpdated "0"`, `LastPlayed "0"`, `LastOwner "0"`, `InstalledDepots`, optional `SharedDepots`, empty `UserConfig` and `MountedConfig`. `write_shared_acf` writes `appmanifest_228980.acf` when shared depots exist, with `InstallScripts` using Windows-style escaped paths.

- [ ] **Step 4: Test, commit**

Run: `cargo test -p steam-press-core`
Expected: PASS.
Run: `git add crates/steam-press-core && git commit -m "feat: add VDF parsing and ACF generation"`

---

### Task 6: Downloader abstraction and SteamCMD engine

**Files:**
- Create: `crates/steam-press-core/src/downloader/mod.rs`
- Create: `crates/steam-press-core/src/downloader/steamcmd.rs`
- Modify: `crates/steam-press-core/src/lib.rs`

**Interfaces:**
- Produces: `DownloaderKind`, `Credentials { username, password: Option<String> }`, `DownloadRequest` (with `depots: Vec<DepotSelector>`), `DepotSelector { depot_id, manifest_id: Option<String> }`, `WorkshopRequest`.
- Produces: `DownloadRequest::for_job(GameJob, Credentials, &Path) -> DownloadRequest` with empty `depots`, for tests and callers that let the engine pick depots.
- Produces: `DownloadReport { content_dir, app_manifest: Option<PathBuf>, depots: Vec<DepotEntry>, build_id: String }`, `WorkshopReport { content_dir }`.
- Produces: `DownloadError` enum with `MissingTool`, `InvalidCredentials`, `RateLimited`, `NoSubscription`, `Failed(String)`, `Cancelled`.
- Produces: `trait Downloader { kind, check, download, download_workshop, cancel }` and `EventSink = Arc<dyn Fn(PackEvent) + Send + Sync>`.
- Produces: `steamcmd::build_script(&DownloadRequest) -> String`, `classify_run(exit_code, content_log, connection_log, has_sitelicense, has_compat_log) -> Result<(), DownloadError>`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn builds_public_script_with_platform_and_validate() {
    let req = DownloadRequest::for_job(
        GameJob::new(Platform::Linux64, "2379780"),
        Credentials { username: "alice".into(), password: Some("pw".into()) },
        Path::new("/tmp/dl"),
    );
    assert_eq!(
        build_script(&req),
        "login alice pw\n@sSteamCmdForcePlatformType linux\n@sSteamCmdForcePlatformBitness 64\napp_update 2379780 validate\nquit"
    );
}

#[test]
fn builds_protected_beta_script() {
    let mut req = DownloadRequest::for_job(
        GameJob::new(Platform::Win64, "123").with_branch("beta", "pw2"),
        Credentials { username: "alice".into(), password: None },
        Path::new("/tmp/dl"),
    );
    assert!(build_script(&req).contains("app_update 123 -beta beta -betapassword pw2 validate"));
}

#[test]
fn classifies_log_failures() {
    assert_eq!(
        classify_run(1, "", "", true, true),
        Err(DownloadError::Failed("steamcmd exited with code 1".into()))
    );
    assert_eq!(classify_run(0, "", "Rate Limit Exceeded", true, true), Err(DownloadError::RateLimited));
    assert_eq!(classify_run(0, "No subscription", "", true, true), Err(DownloadError::NoSubscription));
    assert_eq!(classify_run(0, "", "Invalid Password", true, true), Err(DownloadError::InvalidCredentials));
    assert_eq!(classify_run(0, "", "", true, true), Ok(()));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-core builds_public_script`
Expected: FAIL, downloader module missing.

- [ ] **Step 3: Implement the abstraction and SteamCMD**

`DownloadRequest` holds job, credentials, work dir, optional pinned manifests and `DownloaderKind`. `build_script` emits the login line with the password only when present and the user is not `anonymous`, then platform directives, then `app_update <id>` with `-beta`, `-betapassword`, `validate`, then `quit`.

`classify_run` checks rate limit, no subscription, invalid password, missing site license/compat log (treated as Steam Guard failure), and non-zero exit in that priority order.

The engine locates `steamcmd`/`steamcmd.sh` via `tooling::find_executable`, writes the script to `work/steamcmd-<appid>.job` with `0600`, runs `steamcmd +runscript <path>`, streams stdout/stderr as `PackEvent::Log`, removes the script, then reads `content_log.txt` and `connection_log.txt` from the game dir. Cancellation terminates the child and returns `Cancelled`. `check` reports whether the tool exists; on Linux it never downloads SteamCMD automatically.

- [ ] **Step 4: Test, commit**

Run: `cargo test -p steam-press-core`
Expected: PASS.
Run: `git add crates/steam-press-core && git commit -m "feat: add downloader abstraction and SteamCMD engine"`

---

### Task 7: DepotDownloader engine

**Files:**
- Create: `crates/steam-press-core/src/downloader/depotdownloader.rs`
- Modify: `crates/steam-press-core/src/downloader/mod.rs`

**Interfaces:**
- Consumes: the trait and types from Task 6.
- Produces: `depotdownloader::build_args(&DownloadRequest) -> Vec<String>` and a `DepotDownloader` implementation.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn builds_depotdownloader_args_for_selected_depots() {
    let mut req = DownloadRequest::for_job(
        GameJob::new(Platform::Linux64, "2379780").with_branch("beta", "pw2"),
        Credentials { username: "alice".into(), password: Some("secret".into()) },
        Path::new("/tmp/dl"),
    );
    req.depots = vec![
        DepotSelector { depot_id: "2379781".into(), manifest_id: Some("111".into()) },
        DepotSelector { depot_id: "228989".into(), manifest_id: None },
    ];
    let args = build_args(&req);
    assert!(args.contains(&"-app".to_string()));
    assert!(args.windows(2).any(|w| w == ["-depot", "2379781"]));
    assert!(args.windows(2).any(|w| w == ["-manifest", "111"]));
    assert!(args.windows(2).any(|w| w == ["-branch", "beta"]));
    assert!(args.windows(2).any(|w| w == ["-branchpassword", "pw2"]));
    assert!(args.windows(2).any(|w| w == ["-os", "linux"]));
    assert!(args.windows(2).any(|w| w == ["-dir", "/tmp/dl"]));
    assert!(!args.join(" ").contains("secret"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-core builds_depotdownloader_args`
Expected: FAIL, `build_args` missing.

- [ ] **Step 3: Implement**

Args: `-app <appid>`, repeated `-depot`, `-manifest` when pinned, `-branch` unless public, `-branchpassword` when present, `-os <windows|linux|macos>`, `-dir <work>`, `-validate`, `-username`, `-password` only when explicitly allowed (never logged). The binary is detected through `depotdownloaderpath` then `DepotDownloader`/`depotdownloader` in `PATH`; missing tool returns `MissingTool` with the expected names. Output is streamed, cancellation kills the child, and the report points at the staging directory with no ACF (the core generates one).

- [ ] **Step 4: Test, commit**

Run: `cargo test -p steam-press-core`
Expected: PASS.
Run: `git add crates/steam-press-core && git commit -m "feat: add DepotDownloader engine"`

---

### Task 8: Archives and templates

**Files:**
- Create: `crates/steam-press-core/src/archive.rs`
- Create: `crates/steam-press-core/src/template.rs`
- Create: `crates/steam-press-core/tests/fixtures/template.txt`
- Modify: `crates/steam-press-core/src/lib.rs`

**Interfaces:**
- Produces: `Compressor::{SevenZip, Rar}`, `Compressor::from_setting`, `setting`, `extension`, `default_args(password, archive)`, `args(custom, password, archive)`.
- Produces: `archive_exists(dir, stem, compressor) -> bool`, `finalize_volumes(dir, stem, compressor) -> io::Result<Option<PathBuf>>`.
- Produces: `TemplateData`, `TemplateDepot`, `render_builtin_rin`, `render_builtin_crew`, `render_custom(template, data)`, `write_info_files(...)`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn builds_default_and_custom_archive_args() {
    let args = Compressor::SevenZip.default_args("cs.rin.ru", Path::new("/out/Game.7z"));
    assert_eq!(args[0], "a");
    assert!(args.contains(&"-mx9".to_string()));
    assert!(args.contains(&"-pcs.rin.ru".to_string()));
    assert!(args.contains(&"-v5g".to_string()));
    let args = Compressor::Rar.default_args("pw", Path::new("/out/Game.rar"));
    assert!(args.contains(&"-hpw".to_string()));
    assert!(args.contains(&"-v5000000k".to_string()));
    let args = Compressor::SevenZip.args(Some("-mx5 -mmt=8"), "pw", Path::new("/out/G.7z"));
    assert_eq!(args, vec!["a", "-mx5", "-mmt=8", "/out/G.7z", "*"]);
}

#[test]
fn renders_supersteampacker_template() {
    let data = sample_template_data();
    let text = render_builtin_rin(&data);
    assert!(text.contains("[b]Balatro [Linux64] [Branch: public] (Clean Steam Files)[/b]"));
    assert!(text.contains("[Build 18674832]"));
    assert!(text.contains("2379781 - Balatro [Manifest 4851806656204679952]"));
}

#[test]
fn renders_custom_tokens_and_depot_loop() {
    let data = sample_template_data();
    let template = "{{game_name}}/{{build_id}}\n{{#depots}}{{depot_id}}={{manifest_id}}\n{{/depots}}";
    assert_eq!(render_custom(template, &data), "Balatro/18674832\n2379781=4851806656204679952\n");
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-core builds_default_and_custom`
Expected: FAIL, `Compressor` missing.

- [ ] **Step 3: Implement**

Archive args follow the spec exactly; `args` uses custom options when non-empty. `archive_exists` checks `.7z`, `.7z.001`, `.rar`, `.part1.rar` (and `.7z.002` for finalization). `finalize_volumes` renames a lone `.7z.001` to `.7z`.

`TemplateData` carries game, os, branch, build id and datetime, app id, description, website, username, filehost, upload date, upload datetime and depots. Built-in renders reproduce the SuperSteamPacker BBCode exactly, with the depot lines joined by `\n` and no trailing newline inside the code block. `render_custom` supports `{{token}}` replacement and a `{{#depots}}...{{/depots}}` loop; unknown tokens stay literal. The test module defines `sample_template_data()` locally with Balatro, one depot `2379781` and manifest `4851806656204679952`.

- [ ] **Step 4: Test, commit**

Run: `cargo test -p steam-press-core`
Expected: PASS.
Run: `git add crates/steam-press-core && git commit -m "feat: add archive arguments and release templates"`

---

### Task 9: Pack pipeline

**Files:**
- Create: `crates/steam-press-core/src/pack.rs`
- Create: `crates/steam-press-core/src/tooling/process.rs`
- Modify: `crates/steam-press-core/src/lib.rs`

**Interfaces:**
- Consumes: every previous module.
- Produces: `PackOptions { settings, paths, downloader_kind, dry_run }` with `PackOptions::for_tests(dir)`, `PackStatus::{Completed, Skipped, Failed(String), Cancelled}`, `PackOutcome { status, stem, output, info_files, build_id, build_time, depots, duration }`, `Preflight`.
- Produces: `Packer::new(PackOptions) -> Packer`, `Packer::with_preflight(PackOptions, PreflightFn) -> Packer` where `PreflightFn = Arc<dyn Fn(&GameJob) -> Result<Preflight, PipelineError> + Send + Sync>`, `Packer::preflight(&GameJob) -> Result<Preflight, PipelineError>`, `Packer::run(&GameJob, EventSink) -> PackOutcome`, `Packer::run_dry(&GameJob) -> PackOutcome`, `Packer::cancel()`.
- Produces: `EventSink::noop()` and `EventSink::channel()` for tests and frontends.

- [ ] **Step 1: Write failing tests**

```rust
fn fixture_preflight(_: &GameJob) -> Result<Preflight, PipelineError> {
    Ok(Preflight {
        appid: "2379780".into(),
        name: "Balatro".into(),
        install_dir: Some("Balatro".into()),
        build_id: "18674832".into(),
        build_time: None,
        depots: vec![],
        store: None,
    })
}

#[test]
fn skips_when_archive_already_exists() {
    let dir = test_support::temp_dir("pack-skip");
    fs::write(dir.join("Balatro.Build.18674832.Linux64.public.7z"), b"x").unwrap();
    let packer = Packer::with_preflight(PackOptions::for_tests(dir), Arc::new(fixture_preflight));
    let outcome = packer.run(&GameJob::new(Platform::Linux64, "2379780"), EventSink::noop());
    assert!(matches!(outcome.status, PackStatus::Skipped));
}

#[test]
fn dry_run_reports_naming_without_downloading() {
    let dir = test_support::temp_dir("pack-dry");
    let packer = Packer::with_preflight(PackOptions::for_tests(dir), Arc::new(fixture_preflight));
    let outcome = packer.run_dry(&GameJob::new(Platform::Linux64, "2379780"));
    assert_eq!(outcome.stem, "Balatro.Build.18674832.Linux64.public");
    assert!(outcome.output.unwrap().ends_with("Balatro.Build.18674832.Linux64.public.7z"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-core skips_when_archive`
Expected: FAIL, `Packer` missing.

- [ ] **Step 3: Implement**

`run` executes the nine spec stages with a `CancellationToken` (`Arc<AtomicBool>`). It fetches appinfo, computes the stem, checks existing archives, invokes the selected downloader, normalizes content into `work/temp`, sanitizes or generates ACFs, collects depot lines (name from `depots.ini`), runs the compressor with streamed output, finalizes volumes, writes info files, and returns `PackOutcome`. Failures map to `Failed` with the stage and message; cancellation returns `Cancelled` and keeps partial output. `dry_run` performs only preflight and naming. The process helper runs a command, forwards lines to the sink, honors cancellation and returns the exit code.

- [ ] **Step 4: Test, commit**

Run: `cargo test -p steam-press-core`
Expected: PASS.
Run: `git add crates/steam-press-core && git commit -m "feat: add pack pipeline with cancellation"`

---

### Task 10: CLI

**Files:**
- Create: `crates/steam-press-cli/Cargo.toml`
- Create: `crates/steam-press-cli/src/main.rs`
- Create: `crates/steam-press-cli/src/commands/{mod,pack,queue,workshop,depots,template,doctor,login}.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: the whole core public API.
- Produces: binary `steam-press` with subcommands from the spec.

- [ ] **Step 1: Add the crate, clap and a failing smoke test**

`crates/steam-press-cli/Cargo.toml` depends on `steam-press-core` (path), `clap` with `derive`, `serde_json`, `chrono`.

```rust
#[test]
fn cli_definition_is_valid() {
    let cmd = build_cli();
    cmd.clone().debug_assert();
    let matches = cmd.try_get_matches_from(["steam-press", "depots", "lookup", "10"]).unwrap();
    assert_eq!(matches.subcommand_name(), Some("depots"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-cli`
Expected: FAIL, crate or `build_cli` missing.

- [ ] **Step 3: Implement commands**

Clap derive with `doctor`, `login`, `info`, `queue` (`list/add/remove/move/clear/import/export`), `pack`, `workshop`, `depots`, `template`, and `--config-dir`/`--output` global options. `pack` prints a live stage/progress line on stderr and the final stem on stdout; exit codes follow the spec. `doctor` reports tool paths, depot count, output writability and network reachability. `template render` reads a JSON job description and a template file. No prompt ever echoes a password.

- [ ] **Step 4: Test, commit**

Run: `cargo test --workspace`
Expected: PASS.
Run: `git add Cargo.toml crates/steam-press-cli && git commit -m "feat: add steam-press CLI"`

---

### Task 11: egui desktop app

**Files:**
- Create: `crates/steam-press-gui/Cargo.toml`
- Create: `crates/steam-press-gui/src/{main,app,state,views}.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `Settings`, `Paths`, `GameJob`, `WorkshopJob`, `Packer`, `PackEvent`.
- Produces: binary `steam-press-gui`.

- [ ] **Step 1: Add the crate and a headless test of the view model**

`eframe = { version = "0.31", default-features = false, features = ["default_fonts", "glow", "x11"] }`.

```rust
#[test]
fn queue_state_adds_deduplicates_and_reorders() {
    let mut state = QueueState::default();
    assert!(state.push(GameJob::new(Platform::Linux64, "10")));
    assert!(!state.push(GameJob::new(Platform::Linux64, "10")));
    state.push(GameJob::new(Platform::Win64, "20"));
    state.move_up(1);
    assert_eq!(state.jobs()[0].appid, "20");
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p steam-press-gui`
Expected: FAIL, crate or `QueueState` missing.

- [ ] **Step 3: Implement the UI**

`App` owns settings, paths, queue state, workshop state, log buffer, progress, active job and a receiver. Tabs **Jeu**, **Atelier**, **Dépôts**, **Réglages**; workers run in `std::thread` and send `PackEvent` over `mpsc`; `ctx.request_repaint_after(100ms)` while a job is active. Starting requires a username and confirmation for `anonymous`. Closing while active asks for confirmation. Paths are text fields with an "Ouvrir" button using `xdg-open`/`explorer`/`open`. All labels are French; no business logic beyond view state.

- [ ] **Step 4: Test, build and commit**

Run: `cargo test -p steam-press-gui`, then `cargo check -p steam-press-gui`
Expected: PASS / success. Report that the window cannot be opened here (no display).
Run: `git add Cargo.toml crates/steam-press-gui && git commit -m "feat: add egui desktop interface"`

---

### Task 12: Documentation and final verification

**Files:**
- Modify: `README.md`
- Create: `docs/usage.md`

- [ ] **Step 1: Write the README and usage guide**

Cover prerequisites (`steamcmd`, `7z`, optional `rar`/`DepotDownloader`), install/build commands, CLI examples, GUI workflow, config locations, security note about the local encrypted vault, template tokens, and the Linux-first support statement.

- [ ] **Step 2: Run the full verification suite**

Run: `cargo fmt --all -- --check` (apply `cargo fmt --all` first)
Run: `cargo clippy --workspace --all-targets -- -D warnings` (fix warnings)
Run: `cargo test --workspace`
Run: `cargo build -p steam-press-cli`
Run: `cargo check -p steam-press-gui`
Run: `./target/debug/steam-press doctor` and record the output as a limitation (no SteamCMD installed here).

- [ ] **Step 3: Commit**

Run: `git add README.md docs/usage.md && git commit -m "docs: document Steam Press usage and limitations"`

---

## Self-Review

- **Spec coverage:** config, ini, credentials, depots, steam, vdf/acf, platform, queue, downloader abstraction, SteamCMD, DepotDownloader, archive, template, naming, pack, events, tooling, CLI, GUI, tests and docs each map to Tasks 1–12.
- **Placeholders:** no TBD/TODO; every task names files, tests, commands and expected results.
- **Type consistency:** `GameJob`, `Platform`, `DepotEntry`, `AppInfo`, `Settings`, `Paths`, `PackEvent`, `PackOutcome` keep the same names and fields across tasks.
- **Known limitation:** GUI compilation and real Steam downloads cannot be verified in this environment; the plan reports both explicitly.
