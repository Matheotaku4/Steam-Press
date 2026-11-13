//! 7-Zip and RAR command lines, archive detection and volume finalization.

use crate::compressor::Compressor;
use crate::config::Settings;
use crate::tooling;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

impl Compressor {
    /// SuperSteamPacker's default command line for this backend.
    pub fn default_args(&self, password: &str, archive: &Path) -> Vec<String> {
        let archive = archive.to_string_lossy().into_owned();
        match self {
            Self::SevenZip => vec![
                "a".to_string(),
                "-mx9".to_string(),
                "-sdel".to_string(),
                format!("-p{password}"),
                "-mhe=on".to_string(),
                "-v5g".to_string(),
                archive,
                "*".to_string(),
            ],
            Self::Rar => vec![
                "a".to_string(),
                "-df".to_string(),
                format!("-hp{password}"),
                "-htc".to_string(),
                "-v5000000k".to_string(),
                "-r".to_string(),
                archive,
                "*".to_string(),
            ],
        }
    }

    /// Custom options replace the flags after `a`, like SuperSteamPacker.
    pub fn args(&self, custom: Option<&str>, password: &str, archive: &Path) -> Vec<String> {
        let custom = custom.map(str::trim).filter(|options| !options.is_empty());
        match custom {
            Some(options) => {
                let mut args = vec!["a".to_string()];
                args.extend(options.split_whitespace().map(str::to_string));
                args.push(archive.to_string_lossy().into_owned());
                args.push("*".to_string());
                args
            }
            None => self.default_args(password, archive),
        }
    }

    pub fn binary_names(&self) -> &'static [&'static str] {
        match self {
            Self::SevenZip => &["7z", "7zz", "7za"],
            Self::Rar => &["rar"],
        }
    }
}

pub fn compressor_binary(compressor: Compressor, settings: &Settings) -> Result<PathBuf, String> {
    let explicit = match compressor {
        Compressor::SevenZip => settings.sevenzip_path.as_deref(),
        Compressor::Rar => settings.rar_path.as_deref(),
    };
    tooling::find_executable(explicit, compressor.binary_names()).ok_or_else(|| {
        format!(
            "{} not found: install {} or set its path in the settings",
            compressor.label(),
            compressor.binary_names().join(" / ")
        )
    })
}

pub fn archive_path(directory: &Path, stem: &str, compressor: Compressor) -> PathBuf {
    directory.join(format!("{stem}.{}", compressor.extension()))
}

pub fn checksum_manifest_path(directory: &Path, stem: &str) -> PathBuf {
    directory.join(format!("{stem}.sha256"))
}

/// Command-line arguments used by `7z t` / `rar t` for an integrity test.
pub fn verify_args(_compressor: Compressor, password: &str, archive: &Path) -> Vec<String> {
    vec![
        "t".to_string(),
        format!("-p{password}"),
        archive.to_string_lossy().into_owned(),
    ]
}

/// Runs the archiver's integrity test against the first archive part.
pub fn verify_archive(
    settings: &Settings,
    directory: &Path,
    stem: &str,
    compressor: Compressor,
) -> Result<(), String> {
    let parts = archive_parts(directory, stem, compressor)
        .map_err(|error| format!("archive scan: {error}"))?;
    let first = parts
        .first()
        .ok_or_else(|| "archive test: no archive parts found".to_string())?;
    let binary = compressor_binary(compressor, settings)?;
    let output = std::process::Command::new(binary)
        .args(verify_args(compressor, settings.archive_password(), first))
        .output()
        .map_err(|error| format!("archive test: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    Err(if detail.is_empty() {
        format!("archive test failed with exit code {}", output.status)
    } else {
        format!("archive test failed: {detail}")
    })
}

/// Lists the archive files for a stem, including split volumes.
pub fn archive_parts(
    directory: &Path,
    stem: &str,
    compressor: Compressor,
) -> io::Result<Vec<PathBuf>> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let prefix = format!("{stem}.");
    let mut parts = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.path().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_archive = name.starts_with(&prefix)
            && match compressor {
                Compressor::SevenZip => {
                    name.ends_with(".7z")
                        || (name.contains(".7z.")
                            && name
                                .rsplit('.')
                                .next()
                                .map(|suffix| suffix.chars().all(|c| c.is_ascii_digit()))
                                .unwrap_or(false))
                }
                Compressor::Rar => {
                    name.ends_with(".rar")
                        && (name == format!("{stem}.rar")
                            || name.starts_with(&format!("{stem}.part")))
                }
            };
        if is_archive {
            parts.push(entry.path());
        }
    }
    parts.sort();
    Ok(parts)
}

/// Writes a `sha256sum`-compatible manifest for every archive part.
pub fn write_checksums(
    directory: &Path,
    stem: &str,
    compressor: Compressor,
) -> io::Result<Option<PathBuf>> {
    let parts = archive_parts(directory, stem, compressor)?;
    if parts.is_empty() {
        return Ok(None);
    }
    let mut content = String::new();
    for part in &parts {
        let name = part
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        content.push_str(&format!("{}  {}\n", sha256_file(part)?, name));
    }
    let manifest = checksum_manifest_path(directory, stem);
    fs::write(&manifest, content)?;
    Ok(Some(manifest))
}

pub(crate) fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Whether any archive part for this stem already exists.
pub fn archive_exists(directory: &Path, stem: &str, compressor: Compressor) -> bool {
    let candidates: &[&str] = match compressor {
        Compressor::SevenZip => &["7z", "7z.001", "7z.002"],
        Compressor::Rar => &["rar", "part1.rar", "part2.rar"],
    };
    candidates
        .iter()
        .any(|suffix| directory.join(format!("{stem}.{suffix}")).is_file())
}

/// Renames a lone `.7z.001` volume to `.7z`, matching SuperSteamPacker.
pub fn finalize_volumes(
    directory: &Path,
    stem: &str,
    compressor: Compressor,
) -> io::Result<Option<PathBuf>> {
    if compressor != Compressor::SevenZip {
        return Ok(None);
    }
    let first = directory.join(format!("{stem}.7z.001"));
    let second = directory.join(format!("{stem}.7z.002"));
    let target = directory.join(format!("{stem}.7z"));
    if first.is_file() && !second.is_file() && !target.exists() {
        fs::rename(&first, &target)?;
        return Ok(Some(target));
    }
    Ok(None)
}

/// Deletes partial archive files after a compression failure.
pub fn remove_archive_parts(directory: &Path, stem: &str) -> io::Result<usize> {
    let mut removed = 0;
    if !directory.is_dir() {
        return Ok(0);
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_archive = name.starts_with(&format!("{stem}."))
            && (name.ends_with(".7z")
                || name.contains(".7z.")
                || name.ends_with(".rar")
                || name.contains(".part"));
        if is_archive && entry.path().is_file() {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    #[test]
    fn builds_default_and_custom_archive_args() {
        let args = Compressor::SevenZip.default_args("cs.rin.ru", Path::new("/out/Game.7z"));
        assert_eq!(args[0], "a");
        assert!(args.contains(&"-mx9".to_string()));
        assert!(args.contains(&"-sdel".to_string()));
        assert!(args.contains(&"-pcs.rin.ru".to_string()));
        assert!(args.contains(&"-mhe=on".to_string()));
        assert!(args.contains(&"-v5g".to_string()));

        let args = Compressor::Rar.default_args("cs.rin.ru", Path::new("/out/Game.rar"));
        assert!(args.contains(&"-df".to_string()));
        assert!(args.contains(&"-hpcs.rin.ru".to_string()));
        assert!(args.contains(&"-htc".to_string()));
        assert!(args.contains(&"-v5000000k".to_string()));

        let args = Compressor::SevenZip.args(Some("-mx5 -mmt=8"), "pw", Path::new("/out/G.7z"));
        assert_eq!(args, vec!["a", "-mx5", "-mmt=8", "/out/G.7z", "*"]);
    }

    #[test]
    fn detects_archives_and_finalizes_single_volumes() {
        let dir = temp_dir("archive");
        let stem = "Balatro.Build.1.Linux64.public";
        assert!(!archive_exists(&dir, stem, Compressor::SevenZip));

        fs::write(dir.join(format!("{stem}.7z.001")), b"x").unwrap();
        assert!(archive_exists(&dir, stem, Compressor::SevenZip));
        let renamed = finalize_volumes(&dir, stem, Compressor::SevenZip)
            .unwrap()
            .unwrap();
        assert!(renamed.ends_with(format!("{stem}.7z")));

        fs::write(dir.join(format!("{stem}.rar")), b"x").unwrap();
        assert!(archive_exists(&dir, stem, Compressor::Rar));
    }

    #[test]
    fn writes_checksums_for_split_archives() {
        let dir = temp_dir("archive-checksums");
        let stem = "Game.Build.1.Linux64.public";
        fs::write(dir.join(format!("{stem}.7z.001")), b"first").unwrap();
        fs::write(dir.join(format!("{stem}.7z.002")), b"second").unwrap();
        fs::write(dir.join("other.7z.001"), b"ignored").unwrap();

        let parts = archive_parts(&dir, stem, Compressor::SevenZip).unwrap();
        assert_eq!(parts.len(), 2);
        let manifest = write_checksums(&dir, stem, Compressor::SevenZip)
            .unwrap()
            .unwrap();
        let content = fs::read_to_string(&manifest).unwrap();
        assert_eq!(content.lines().count(), 2);
        assert!(content.contains(&format!("{stem}.7z.001")));
        assert!(content.contains(&format!("{stem}.7z.002")));
        assert!(content.lines().all(|line| line.len() > 70));
    }

    #[test]
    fn builds_verification_arguments() {
        let args = verify_args(Compressor::SevenZip, "pw", Path::new("/out/Game.7z"));
        assert_eq!(args, vec!["t", "-ppw", "/out/Game.7z"]);
    }

    #[cfg(unix)]
    #[test]
    fn verification_fails_when_the_archiver_rejects_the_archive() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("archive-verify");
        let stem = "Game.Build.1.Linux64.public";
        fs::write(dir.join(format!("{stem}.7z")), b"archive").unwrap();
        let script = dir.join("fake-7z.sh");
        fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let settings = Settings {
            sevenzip_path: Some(script.clone()),
            ..Default::default()
        };
        verify_archive(&settings, &dir, stem, Compressor::SevenZip).unwrap();

        fs::write(&script, "#!/bin/sh\nexit 2\n").unwrap();
        assert!(verify_archive(&settings, &dir, stem, Compressor::SevenZip).is_err());
    }

    #[test]
    fn removes_only_archive_parts() {
        let dir = temp_dir("archive-cleanup");
        fs::write(dir.join("Game.7z.001"), b"x").unwrap();
        fs::write(dir.join("Game.7z.002"), b"x").unwrap();
        fs::write(dir.join("Game.txt"), b"x").unwrap();
        assert_eq!(remove_archive_parts(&dir, "Game").unwrap(), 2);
        assert!(dir.join("Game.txt").is_file());
    }
}
