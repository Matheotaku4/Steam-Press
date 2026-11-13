//! Self-updater for the GUI, backed by GitHub releases containing a GUI archive.

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const OWNER: &str = "Matheotaku4";
const REPO: &str = "Steam-Press";
const BIN_NAME: &str = "steam-press-gui";
const USER_AGENT: &str = "SteamPress-GUI-Updater";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    pub tag: String,
    pub archive_url: String,
    pub checksum_url: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Returns the newest non-draft GUI release newer than `current`.
pub fn check_for_update(current: &str) -> Result<Option<AvailableUpdate>, String> {
    let tags = fetch_release_tags()?;
    let Some(update) = select_update(&tags, current)? else {
        return Ok(None);
    };
    if !asset_exists(&update.archive_url)? {
        return Ok(None);
    }
    Ok(Some(update))
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| format!("HTTP client failed: {error}"))
}

fn fetch_release_tags() -> Result<Vec<String>, String> {
    let url = format!("https://github.com/{OWNER}/{REPO}/releases.atom");
    let atom = fetch_text(&url)?;
    Ok(parse_release_tags(&atom))
}

fn parse_release_tags(atom: &str) -> Vec<String> {
    const MARKER: &str = "/releases/tag/";
    let mut tags = Vec::new();
    for entry in atom.split("<entry>").skip(1) {
        let Some(start) = entry.find(MARKER) else {
            continue;
        };
        let rest = &entry[start + MARKER.len()..];
        let tag = rest.split(['"', '<']).next().unwrap_or("").trim();
        if !tag.is_empty() && !tags.iter().any(|known| known == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

fn asset_exists(url: &str) -> Result<bool, String> {
    let response = client()?
        .head(url)
        .send()
        .map_err(|error| format!("Update check failed: {error}"))?;
    let status = response.status();
    if status.is_success() {
        Ok(true)
    } else if status == reqwest::StatusCode::NOT_FOUND {
        Ok(false)
    } else {
        Err(format!(
            "GitHub returned HTTP {status} for the update archive"
        ))
    }
}

fn select_update(tags: &[String], current: &str) -> Result<Option<AvailableUpdate>, String> {
    let current_version =
        parse_version(current).ok_or_else(|| format!("unsupported current version: {current}"))?;
    let mut candidates = Vec::new();

    for tag in tags {
        let Some(version) = release_version(tag) else {
            continue;
        };
        let Some(parsed) = parse_version(version) else {
            continue;
        };
        if parsed <= current_version {
            continue;
        }
        let archive_name = format!("{BIN_NAME}-{version}-linux-x86_64.tar.gz");
        let base =
            format!("https://github.com/{OWNER}/{REPO}/releases/download/{tag}/{archive_name}");
        candidates.push(AvailableUpdate {
            version: version.to_string(),
            tag: tag.clone(),
            archive_url: base.clone(),
            checksum_url: format!("{base}.sha256"),
        });
    }

    candidates.sort_by_key(|update| parse_version(&update.version).unwrap_or((0, 0, 0)));
    Ok(candidates.pop())
}

fn release_version(tag: &str) -> Option<&str> {
    tag.strip_prefix("gui-v").or_else(|| tag.strip_prefix('v'))
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Downloads, verifies and replaces the running GUI binary.
pub fn install(update: &AvailableUpdate) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("Automatic updates are only available on Linux.".to_string());
    }
    let temp_dir =
        std::env::temp_dir().join(format!("steam-press-gui-update-{}", std::process::id()));
    let result = install_in(update, &temp_dir);
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

fn install_in(update: &AvailableUpdate, temp_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(temp_dir).map_err(|error| format!("update directory: {error}"))?;
    let archive = temp_dir.join("gui.tar.gz");
    download(&update.archive_url, &archive)?;

    let checksum = fetch_text(&update.checksum_url)?;
    let expected =
        parse_checksum(&checksum).ok_or_else(|| "invalid update checksum file".to_string())?;
    let actual = sha256_file(&archive)?;
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err("update checksum does not match".to_string());
    }

    let extracted = temp_dir.join(BIN_NAME);
    extract_binary(&archive, &extracted)?;

    let current =
        std::env::current_exe().map_err(|error| format!("current executable: {error}"))?;
    let staged = current.with_extension("update-new");
    fs::copy(&extracted, &staged).map_err(|error| format!("staging update: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("update permissions: {error}"))?;
    }
    if let Err(error) = fs::rename(&staged, &current) {
        let _ = fs::remove_file(&staged);
        return Err(format!("replacing the executable: {error}"));
    }
    Ok(())
}

fn download(url: &str, destination: &Path) -> Result<(), String> {
    let response = client()?
        .get(url)
        .send()
        .map_err(|error| format!("download failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("download returned HTTP {}", response.status()));
    }
    let mut file = File::create(destination).map_err(|error| format!("update file: {error}"))?;
    let mut reader = response;
    std::io::copy(&mut reader, &mut file).map_err(|error| format!("saving update: {error}"))?;
    file.flush()
        .map_err(|error| format!("saving update: {error}"))
}

fn fetch_text(url: &str) -> Result<String, String> {
    let response = client()?
        .get(url)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("GitHub returned HTTP {}", response.status()));
    }
    response
        .text()
        .map_err(|error| format!("unreadable response: {error}"))
}

fn parse_checksum(value: &str) -> Option<String> {
    let hash = value.split_whitespace().next()?;
    if hash.len() == 64 && hash.chars().all(|character| character.is_ascii_hexdigit()) {
        Some(hash.to_ascii_lowercase())
    } else {
        None
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("checksum file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("checksum read: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn extract_binary(archive_path: &Path, destination: &Path) -> Result<(), String> {
    let file = File::open(archive_path).map_err(|error| format!("update archive: {error}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive
        .entries()
        .map_err(|error| format!("update archive: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("update archive entry: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("update archive path: {error}"))?;
        if path.file_name().and_then(|name| name.to_str()) != Some(BIN_NAME) {
            continue;
        }
        let mut output =
            File::create(destination).map_err(|error| format!("extracting update: {error}"))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("extracting update: {error}"))?;
        return Ok(());
    }
    Err(format!("update archive does not contain {BIN_NAME}"))
}

pub fn restart() -> Result<(), String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("current executable: {error}"))?;
    Command::new(executable)
        .spawn()
        .map_err(|error| format!("restarting: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATOM: &str = r#"<feed>
        <entry><link href="https://github.com/Matheotaku4/Steam-Press/releases/tag/gui-v0.4.0"/></entry>
        <entry><link href="https://github.com/Matheotaku4/Steam-Press/releases/tag/v0.5.0"/></entry>
        <entry><link href="https://github.com/Matheotaku4/Steam-Press/releases/tag/cli-v0.9.0"/></entry>
    </feed>"#;

    #[test]
    fn selects_the_newest_gui_release() {
        let tags = parse_release_tags(ATOM);
        let update = select_update(&tags, "0.4.0").unwrap().unwrap();
        assert_eq!(update.version, "0.5.0");
        assert_eq!(update.tag, "v0.5.0");
        assert!(update
            .archive_url
            .ends_with("releases/download/v0.5.0/steam-press-gui-0.5.0-linux-x86_64.tar.gz"));
        assert!(update.checksum_url.ends_with(".tar.gz.sha256"));
    }

    #[test]
    fn ignores_old_and_non_gui_entries() {
        let tags = parse_release_tags(ATOM);
        assert!(select_update(&tags, "0.5.0").unwrap().is_none());
        assert!(select_update(&tags, "0.5.1").unwrap().is_none());
    }

    #[test]
    #[ignore = "requires network access to GitHub"]
    fn github_releases_are_reachable() {
        let tags = fetch_release_tags().unwrap();
        assert!(!tags.is_empty());
    }

    #[test]
    #[ignore = "requires a published GUI release"]
    fn finds_the_published_gui_release() {
        let update = check_for_update("0.3.1").unwrap().unwrap();
        assert_eq!(update.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn parses_strict_versions_and_checksums() {
        assert_eq!(parse_version("0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(
            parse_checksum(
                "63cdfa38bb391a85ff4da2e209c09b45d0dec379a81d22461ec044671ab4d8e6  file"
            )
            .as_deref(),
            Some("63cdfa38bb391a85ff4da2e209c09b45d0dec379a81d22461ec044671ab4d8e6")
        );
        assert!(parse_checksum("not-a-checksum").is_none());
    }
}
