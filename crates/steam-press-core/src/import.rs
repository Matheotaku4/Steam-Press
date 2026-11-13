//! Import a local Steam installation without downloading it again.

use crate::acf::{self, AcfDepot, AcfJob, SharedDepot};
use crate::archive;
use crate::config::{Paths, Settings};
use crate::depots::DepotNames;
use crate::events::{EventSink, FinishStatus, PackEvent, Stage};
use crate::naming;
use crate::pack::{compress_directory, CompressOutcome, PackStatus, PipelineError};
use crate::platform::Platform;
use crate::steam::{AppInfo, StoreInfo};
use crate::template::{self, TemplateData, TemplateDepot};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub settings: Settings,
    pub paths: Paths,
    pub source: PathBuf,
    pub appid: String,
    pub branch: String,
    pub platform: Platform,
    pub name: Option<String>,
    pub install_dir: Option<String>,
    pub fresh: bool,
    pub depot_names: DepotNames,
}

#[derive(Debug, Clone)]
pub struct ImportOutcome {
    pub status: PackStatus,
    pub stem: String,
    pub output: Option<PathBuf>,
    pub manifest: Option<PathBuf>,
    pub copied_bytes: u64,
    pub duration: Duration,
}

impl ImportOutcome {
    pub fn skipped(stem: String, output: PathBuf, duration: Duration) -> Self {
        Self {
            status: PackStatus::Skipped,
            stem,
            output: Some(output),
            manifest: None,
            copied_bytes: 0,
            duration,
        }
    }
}

/// Imports a local game directory, generating clean manifests and an archive.
pub fn import_local(
    options: &ImportOptions,
    sink: EventSink,
) -> Result<ImportOutcome, PipelineError> {
    let info = crate::steam::fetch_app_info(&options.appid, &options.branch)
        .map_err(PipelineError::Steam)?;
    let store = crate::steam::fetch_store_info(&options.appid).ok();
    import_local_with_info(options, info, store, sink)
}

/// Testable import path with pre-fetched Steam metadata.
pub fn import_local_with_info(
    options: &ImportOptions,
    info: AppInfo,
    store: Option<StoreInfo>,
    sink: EventSink,
) -> Result<ImportOutcome, PipelineError> {
    let started = Instant::now();
    if !options.source.is_dir() {
        return Err(PipelineError::Invalid(format!(
            "{} is not a directory",
            options.source.display()
        )));
    }

    let name = options
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(info.name.clone());
    let install_dir = options
        .install_dir
        .clone()
        .filter(|dir| !dir.trim().is_empty())
        .or(info.install_dir.clone())
        .map(|dir| naming::sanitize_install_dir(&dir))
        .filter(|dir| !dir.is_empty())
        .unwrap_or_else(|| naming::sanitize_install_dir(&name));
    let stem = naming::output_stem(&name, &info.build_id, options.platform, &options.branch);
    let output_dir = options.settings.output_dir(&options.paths);
    fs::create_dir_all(&output_dir).map_err(|error| PipelineError::Io(error.to_string()))?;

    if let Ok(parts) = archive::archive_parts(&output_dir, &stem, options.settings.compressor) {
        if !parts.is_empty() {
            return Ok(ImportOutcome::skipped(
                stem.clone(),
                archive::archive_path(&output_dir, &stem, options.settings.compressor),
                started.elapsed(),
            ));
        }
    }

    sink.emit(PackEvent::Stage {
        job: stem.clone(),
        stage: Stage::Preflight,
        message: format!("Importing {}", options.source.display()),
    });

    let work_dir = options.paths.work_dir.join(format!("{stem}.import"));
    if options.fresh && work_dir.exists() {
        fs::remove_dir_all(&work_dir).map_err(|error| PipelineError::Io(error.to_string()))?;
    }
    let temp = work_dir.join("temp");
    let install_path = temp.join("steamapps/common").join(&install_dir);
    if temp.exists() {
        fs::remove_dir_all(&temp).map_err(|error| PipelineError::Io(error.to_string()))?;
    }
    fs::create_dir_all(&install_path).map_err(|error| PipelineError::Io(error.to_string()))?;

    sink.emit(PackEvent::Stage {
        job: stem.clone(),
        stage: Stage::Normalize,
        message: "Copying the local installation".to_string(),
    });
    let copied_bytes = copy_tree(&options.source, &install_path)
        .map_err(|error| PipelineError::Io(error.to_string()))?;

    sink.emit(PackEvent::Stage {
        job: stem.clone(),
        stage: Stage::Manifest,
        message: "Generating clean manifests".to_string(),
    });

    let shared_depots: Vec<SharedDepot> = info
        .depots
        .iter()
        .filter_map(|depot| {
            depot.shared_owner.as_ref().map(|owner| SharedDepot {
                depot_id: depot.depot_id.clone(),
                owner_appid: owner.clone(),
                manifest_id: depot.manifest_id.clone(),
                size: depot.size_bytes.unwrap_or(0),
            })
        })
        .collect();
    let acf_job = AcfJob {
        appid: options.appid.clone(),
        name: name.clone(),
        install_dir: install_dir.clone(),
        build_id: info.build_id.clone(),
        size_on_disk: copied_bytes,
        depots: info
            .depots
            .iter()
            .filter(|depot| depot.shared_owner.is_none())
            .map(|depot| AcfDepot {
                depot_id: depot.depot_id.clone(),
                manifest_id: depot.manifest_id.clone(),
                size: depot.size_bytes.unwrap_or(0),
                dlc_appid: depot.dlc_appid.clone(),
            })
            .collect(),
        shared_depots: shared_depots.clone(),
        install_scripts: Vec::new(),
    };
    let steamapps = temp.join("steamapps");
    acf::write_acf(&steamapps, &acf_job)
        .map_err(|error| PipelineError::Archive(format!("manifest: {error}")))?;
    if !shared_depots.is_empty() {
        acf::write_shared_acf(&steamapps, &acf_job)
            .map_err(|error| PipelineError::Archive(format!("shared manifest: {error}")))?;
    }

    sink.emit(PackEvent::Stage {
        job: stem.clone(),
        stage: Stage::Archive,
        message: "Creating the archive".to_string(),
    });
    let output = match compress_directory(
        &options.settings,
        &output_dir,
        &stem,
        &temp,
        &crate::events::CancellationToken::new(),
        &sink,
    )
    .map_err(|error| PipelineError::Invalid(error.to_string()))?
    {
        CompressOutcome::Done(path) => path,
        CompressOutcome::Cancelled => {
            return Err(PipelineError::Invalid("import cancelled".to_string()))
        }
    };

    sink.emit(PackEvent::Stage {
        job: stem.clone(),
        stage: Stage::Template,
        message: "Rendering release information".to_string(),
    });
    let data = TemplateData {
        game_name: name.clone(),
        os: options.platform.os_tag().to_string(),
        branch: options.branch.clone(),
        build_id: info.build_id.clone(),
        build_datetime: info
            .build_time
            .map(template::format_utc_datetime)
            .unwrap_or_default(),
        upload_datetime: template::format_utc_datetime(Utc::now()),
        app_id: options.appid.clone(),
        description: store
            .as_ref()
            .map(|store| store.short_description.clone())
            .unwrap_or_default(),
        website: store.as_ref().and_then(|store| store.website.clone()),
        username: options.settings.uploader.clone(),
        filehost: options.settings.filehost.clone(),
        upload_date: Utc::now().format("%Y-%m-%d").to_string(),
        depots: info
            .depots
            .iter()
            .map(|depot| TemplateDepot {
                depot_id: depot.depot_id.clone(),
                depot_name: options
                    .depot_names
                    .get(&depot.depot_id)
                    .map(str::to_string)
                    .or_else(|| Some(depot.name.clone()))
                    .unwrap_or_default(),
                manifest_id: depot.manifest_id.clone(),
            })
            .collect(),
    };
    let custom_template = options
        .settings
        .template
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok());
    template::write_info_files(
        &output_dir,
        &stem,
        &data,
        options.settings.upload_crew_mode,
        custom_template.as_deref(),
    )
    .map_err(PipelineError::Template)?;

    let manifest = if output.is_dir() {
        Some(output.join(format!("steamapps/appmanifest_{}.acf", options.appid)))
    } else {
        None
    };
    let _ = fs::remove_dir_all(&work_dir);
    sink.emit(PackEvent::Finished {
        job: stem.clone(),
        status: FinishStatus::Completed,
        outcome: "Imported from a local installation".to_string(),
        output: Some(output.clone()),
    });

    Ok(ImportOutcome {
        status: PackStatus::Completed,
        stem,
        output: Some(output),
        manifest,
        copied_bytes,
        duration: started.elapsed(),
    })
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<u64> {
    let mut copied = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::metadata(entry.path())?;
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir_all(&target)?;
            copied += copy_tree(&entry.path(), &target)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), &target)?;
            copied += metadata.len();
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam::DepotEntry;
    use crate::test_support::temp_dir;

    fn app_info() -> AppInfo {
        AppInfo {
            appid: "2379780".into(),
            name: "Balatro".into(),
            install_dir: Some("Balatro".into()),
            build_id: "18674832".into(),
            build_time: None,
            depots: vec![DepotEntry {
                depot_id: "2379781".into(),
                name: "Balatro".into(),
                manifest_id: "4851806656204679952".into(),
                dlc_appid: None,
                size_bytes: Some(4),
                shared_owner: None,
            }],
        }
    }

    #[test]
    fn imports_a_local_directory_without_downloading() {
        let root = temp_dir("import-local");
        let source = root.join("installed");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("Balatro.exe"), b"game").unwrap();
        let options = ImportOptions {
            settings: Settings {
                output_dir: Some(root.join("outputs")),
                skip_compression: true,
                ..Default::default()
            },
            paths: Paths::under(&root),
            source,
            appid: "2379780".into(),
            branch: "public".into(),
            platform: Platform::Linux64,
            name: None,
            install_dir: None,
            fresh: true,
            depot_names: DepotNames::default(),
        };
        let outcome =
            import_local_with_info(&options, app_info(), None, EventSink::noop()).unwrap();
        assert!(matches!(outcome.status, PackStatus::Completed));
        assert_eq!(outcome.copied_bytes, 4);
        let output = outcome.output.unwrap();
        assert!(output.is_dir());
        assert!(output
            .join("steamapps/common/Balatro/Balatro.exe")
            .is_file());
        let manifest = output.join("steamapps/appmanifest_2379780.acf");
        assert!(manifest.is_file());
        acf::validate_manifest(&manifest).unwrap();
    }
}
