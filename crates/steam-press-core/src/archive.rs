//! 7-Zip and RAR command lines, archive detection and volume finalization.

use crate::compressor::Compressor;
use crate::config::Settings;
use crate::tooling;
use std::fs;
use std::io;
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
    fn removes_only_archive_parts() {
        let dir = temp_dir("archive-cleanup");
        fs::write(dir.join("Game.7z.001"), b"x").unwrap();
        fs::write(dir.join("Game.7z.002"), b"x").unwrap();
        fs::write(dir.join("Game.txt"), b"x").unwrap();
        assert_eq!(remove_archive_parts(&dir, "Game").unwrap(), 2);
        assert!(dir.join("Game.txt").is_file());
    }
}
