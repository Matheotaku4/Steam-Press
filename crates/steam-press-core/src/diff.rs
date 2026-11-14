//! Compare two builds and optionally package the changed files as an update.

use crate::archive;
use crate::config::{Paths, Settings};
use crate::events::{CancellationToken, EventSink};
use crate::pack::{compress_directory, CompressOutcome, PipelineError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffReport {
    pub added: Vec<FileChange>,
    pub modified: Vec<FileChange>,
    pub removed: Vec<FileChange>,
    pub unchanged: usize,
}

impl DiffReport {
    pub fn changes(&self) -> Vec<FileChange> {
        self.added
            .iter()
            .chain(self.modified.iter())
            .chain(self.removed.iter())
            .cloned()
            .collect()
    }

    pub fn update_files(&self) -> Vec<FileChange> {
        self.added
            .iter()
            .chain(self.modified.iter())
            .cloned()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.removed.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct UpdateArchive {
    pub stem: String,
    pub report: DiffReport,
    pub report_path: PathBuf,
    pub output: Option<PathBuf>,
}

/// Compares files by relative path. Same-size files are compared by SHA-256.
pub fn diff_directories(old: &Path, new: &Path) -> io::Result<DiffReport> {
    if !old.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", old.display()),
        ));
    }
    if !new.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", new.display()),
        ));
    }
    let old_files = collect_files(old)?;
    let new_files = collect_files(new)?;
    let mut report = DiffReport::default();

    for (path, new_meta) in &new_files {
        match old_files.get(path) {
            None => report.added.push(FileChange {
                path: path.clone(),
                kind: ChangeKind::Added,
                old_size: None,
                new_size: Some(new_meta.size),
            }),
            Some(old_meta) => {
                let modified = old_meta.size != new_meta.size || old_meta.hash != new_meta.hash;
                if modified {
                    report.modified.push(FileChange {
                        path: path.clone(),
                        kind: ChangeKind::Modified,
                        old_size: Some(old_meta.size),
                        new_size: Some(new_meta.size),
                    });
                } else {
                    report.unchanged += 1;
                }
            }
        }
    }
    for (path, old_meta) in &old_files {
        if !new_files.contains_key(path) {
            report.removed.push(FileChange {
                path: path.clone(),
                kind: ChangeKind::Removed,
                old_size: Some(old_meta.size),
                new_size: None,
            });
        }
    }
    report
        .added
        .sort_by(|left, right| left.path.cmp(&right.path));
    report
        .modified
        .sort_by(|left, right| left.path.cmp(&right.path));
    report
        .removed
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(report)
}

#[derive(Debug)]
struct FileMeta {
    size: u64,
    hash: String,
}

fn collect_files(root: &Path) -> io::Result<BTreeMap<String, FileMeta>> {
    let mut files = BTreeMap::new();
    walk(root, root, &mut files)?;
    Ok(files)
}

fn walk(root: &Path, directory: &Path, files: &mut BTreeMap<String, FileMeta>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            walk(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(
                relative,
                FileMeta {
                    size: metadata.len(),
                    hash: archive::sha256_file(&path)?,
                },
            );
        }
    }
    Ok(())
}

/// Name of the manifest listing files removed from the new build.
pub const DELETED_MANIFEST: &str = "deleted_files.txt";

/// Writes the list of removed files so consumers can apply deletions after
/// extracting the update archive.
pub fn write_deleted_manifest(report: &DiffReport, destination: &Path) -> io::Result<bool> {
    if report.removed.is_empty() {
        return Ok(false);
    }
    let mut content =
        String::from("# Files present in the old build and absent from the new build.\n");
    for change in &report.removed {
        content.push_str(&change.path);
        content.push('\n');
    }
    fs::write(destination.join(DELETED_MANIFEST), content)?;
    Ok(true)
}

/// Copies added and modified files into a staging directory.
pub fn stage_changes(new: &Path, report: &DiffReport, destination: &Path) -> io::Result<usize> {
    let mut copied = 0;
    for change in report.update_files() {
        let source = new.join(change.path.split('/').collect::<PathBuf>());
        let target = destination.join(change.path.split('/').collect::<PathBuf>());
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &target)?;
        copied += 1;
    }
    Ok(copied)
}

/// Builds a report and, when there are changes, an archive of added/modified files.
pub fn create_update_archive(
    settings: &Settings,
    paths: &Paths,
    old: &Path,
    new: &Path,
    stem: &str,
    sink: &EventSink,
) -> Result<UpdateArchive, PipelineError> {
    let report =
        diff_directories(old, new).map_err(|error| PipelineError::Io(format!("diff: {error}")))?;
    let output_dir = settings.output_dir(paths);
    fs::create_dir_all(&output_dir).map_err(|error| PipelineError::Io(error.to_string()))?;
    let report_path = output_dir.join(format!("{stem}.diff.json"));
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| PipelineError::Invalid(format!("diff JSON: {error}")))?;
    fs::write(&report_path, json).map_err(|error| PipelineError::Io(error.to_string()))?;

    if report.is_empty() {
        return Ok(UpdateArchive {
            stem: stem.to_string(),
            report,
            report_path,
            output: None,
        });
    }

    let staging = paths.work_dir.join(format!("{stem}.update"));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| PipelineError::Io(error.to_string()))?;
    }
    fs::create_dir_all(&staging).map_err(|error| PipelineError::Io(error.to_string()))?;
    stage_changes(new, &report, &staging)
        .map_err(|error| PipelineError::Io(format!("stage update: {error}")))?;
    write_deleted_manifest(&report, &staging)
        .map_err(|error| PipelineError::Io(format!("deleted manifest: {error}")))?;

    let output = match compress_directory(
        settings,
        &output_dir,
        stem,
        &staging,
        &CancellationToken::new(),
        sink,
    )? {
        CompressOutcome::Done(path) => path,
        CompressOutcome::Cancelled => {
            return Err(PipelineError::Invalid(
                "update archive cancelled".to_string(),
            ))
        }
    };
    let _ = fs::remove_dir_all(&staging);
    Ok(UpdateArchive {
        stem: stem.to_string(),
        report,
        report_path,
        output: Some(output),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    #[test]
    fn detects_added_modified_and_removed_files() {
        let root = temp_dir("diff");
        let old = root.join("old");
        let new = root.join("new");
        fs::create_dir_all(old.join("bin")).unwrap();
        fs::create_dir_all(new.join("bin")).unwrap();
        fs::write(old.join("bin/game"), b"old").unwrap();
        fs::write(new.join("bin/game"), b"new").unwrap();
        fs::write(old.join("old-only.txt"), b"x").unwrap();
        fs::write(new.join("new-only.txt"), b"y").unwrap();

        let report = diff_directories(&old, &new).unwrap();
        assert_eq!(report.added.len(), 1);
        assert_eq!(report.modified.len(), 1);
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.modified[0].path, "bin/game");
    }

    #[test]
    fn writes_removed_files_for_the_update_archive() {
        let root = temp_dir("diff-deleted");
        let old = root.join("old");
        let new = root.join("new");
        let stage = root.join("stage");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(old.join("removed.txt"), b"old").unwrap();
        let report = diff_directories(&old, &new).unwrap();
        assert!(write_deleted_manifest(&report, &stage).unwrap());
        let manifest = fs::read_to_string(stage.join(DELETED_MANIFEST)).unwrap();
        assert!(manifest.contains("removed.txt"));
    }

    #[test]
    fn creates_an_update_directory_with_the_deleted_manifest() {
        let root = temp_dir("diff-archive-deleted");
        let old = root.join("old");
        let new = root.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("removed.txt"), b"old").unwrap();
        fs::write(new.join("added.txt"), b"new").unwrap();
        let paths = Paths::under(&root);
        let settings = Settings {
            output_dir: Some(root.join("out")),
            skip_compression: true,
            verify_archives: false,
            ..Default::default()
        };
        let archive = create_update_archive(
            &settings,
            &paths,
            &old,
            &new,
            "Game.patch",
            &EventSink::noop(),
        )
        .unwrap();
        let output = archive.output.unwrap();
        assert!(output.join("added.txt").is_file());
        let manifest = fs::read_to_string(output.join(DELETED_MANIFEST)).unwrap();
        assert!(manifest.contains("removed.txt"));
    }

    #[test]
    fn stages_only_update_files() {
        let root = temp_dir("diff-stage");
        let old = root.join("old");
        let new = root.join("new");
        let stage = root.join("stage");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("same"), b"same").unwrap();
        fs::write(new.join("same"), b"same").unwrap();
        fs::write(new.join("added"), b"new").unwrap();
        let report = diff_directories(&old, &new).unwrap();
        assert_eq!(stage_changes(&new, &report, &stage).unwrap(), 1);
        assert!(stage.join("added").is_file());
        assert!(!stage.join("same").exists());
    }
}
