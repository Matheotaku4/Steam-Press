//! External tool discovery and small platform helpers.

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

    #[test]
    fn missing_explicit_path_falls_back_to_path() {
        assert!(find_executable(Some(Path::new("/definitely/not/here")), &["definitely-not-a-tool"]).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_system_shell() {
        let found = find_executable(None, &["sh"]);
        assert!(found.is_some());
        assert!(found.unwrap().is_file());
    }
}
