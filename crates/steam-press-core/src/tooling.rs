//! External tool discovery and small platform helpers.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Finds an executable from an explicit path or `PATH`.
pub fn find_executable(explicit: Option<&Path>, names: &[&str]) -> Option<PathBuf> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            for name in names {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    #[cfg(unix)]
    for directory in ["/usr/bin", "/usr/local/bin", "/bin", "/snap/bin"] {
        for name in names {
            let candidate = Path::new(directory).join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Writes a file readable only by its owner on Unix.
pub fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let mut file = fs::File::create(path)?;
        file.write_all(contents.as_bytes())
    }
}

/// Returns the first version-looking token in a tool's output, for example
/// `7-Zip 25.01` or `DepotDownloader v3.4.0`.
pub fn probe_version(path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(path).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    first_version(&text)
}

fn first_version(text: &str) -> Option<String> {
    for token in text.split(|character: char| character.is_whitespace()) {
        let candidate = token
            .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '.');
        let candidate = candidate
            .trim_start_matches('v')
            .trim_start_matches('V')
            .split('+')
            .next()
            .unwrap_or("");
        let mut parts = candidate.split('.');
        let first = parts.next()?;
        if first.is_empty() || !first.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }
        if parts.clone().count() > 2
            || parts.any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        return Some(candidate.to_string());
    }
    None
}

/// Compares dotted numeric versions, padding missing components with zero.
pub fn version_at_least(found: &str, minimum: &str) -> bool {
    let parse = |value: &str| -> Option<Vec<u64>> {
        value
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    let (Some(mut found), Some(mut minimum)) = (parse(found), parse(minimum)) else {
        return false;
    };
    while found.len() < minimum.len() {
        found.push(0);
    }
    while minimum.len() < found.len() {
        minimum.push(0);
    }
    found >= minimum
}

/// Opens a directory in the platform file manager.
pub fn open_folder(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        command.arg(path);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };

    command.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::test_support::temp_dir("private-file");
        let path = dir.join("secret.txt");
        write_private(&path, "secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn reads_and_compares_tool_versions() {
        assert_eq!(
            first_version("7-Zip (z) 25.01 (x64)").as_deref(),
            Some("25.01")
        );
        assert_eq!(
            first_version("DepotDownloader v3.4.0+c553ef4").as_deref(),
            Some("3.4.0")
        );
        assert_eq!(first_version("no version here"), None);
        assert!(version_at_least("25.01", "21.0"));
        assert!(version_at_least("3.4.0", "3.0"));
        assert!(!version_at_least("2.9", "3.0"));
        assert!(!version_at_least("unknown", "1.0"));
    }

    #[test]
    fn missing_explicit_path_falls_back_to_path() {
        assert!(find_executable(
            Some(Path::new("/definitely/not/here")),
            &["definitely-not-a-tool"]
        )
        .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_system_shell() {
        let found = find_executable(None, &["sh"]);
        assert!(found.is_some());
        assert!(found.unwrap().is_file());
    }
}
