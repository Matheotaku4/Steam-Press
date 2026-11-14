//! End-to-end pipeline: download, normalize, manifest, archive, release notes.

use crate::acf::{self, AcfDepot, AcfJob, SharedDepot};
use crate::archive;
use crate::config::{Paths, Settings};
use crate::depots::DepotNames;
use crate::downloader::{
    self, process, Credentials, DepotSelector, DownloadError, DownloadReport, DownloadRequest,
    DownloaderKind,
};
use crate::events::{CancellationToken, EventSink, FinishStatus, PackEvent, Stage};
use crate::naming;
use crate::queue::GameJob;
use crate::steam::{self, DepotEntry, StoreInfo};
use crate::template::{self, TemplateData, TemplateDepot};
use chrono::{DateTime, Utc};
use fs2::available_space;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preflight {
    pub appid: String,
    pub name: String,
    pub install_dir: Option<String>,
    pub build_id: String,
    pub build_time: Option<DateTime<Utc>>,
    pub depots: Vec<DepotEntry>,
    pub store: Option<StoreInfo>,
}

pub type PreflightFn = Arc<dyn Fn(&GameJob) -> Result<Preflight, PipelineError> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    Steam(String),
    Download(String),
    Io(String),
    Archive(String),
    Template(String),
    Invalid(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Steam(message) => write!(formatter, "Steam: {message}"),
            Self::Download(message) => write!(formatter, "download: {message}"),
            Self::Io(message) => write!(formatter, "files: {message}"),
            Self::Archive(message) => write!(formatter, "archive: {message}"),
            Self::Template(message) => write!(formatter, "release info: {message}"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for PipelineError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackStatus {
    Completed,
    Skipped,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct PackOutcome {
    pub status: PackStatus,
    pub stem: String,
    pub output: Option<PathBuf>,
    pub info_files: Vec<PathBuf>,
    pub build_id: String,
    pub build_time: Option<DateTime<Utc>>,
    pub depots: Vec<DepotEntry>,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct PackOptions {
    pub settings: Settings,
    pub paths: Paths,
    pub downloader_kind: DownloaderKind,
    pub dry_run: bool,
    pub fresh: bool,
    pub credentials: Credentials,
    pub depot_names: DepotNames,
}

impl PackOptions {
    pub fn for_tests(directory: impl Into<PathBuf>) -> Self {
        let directory = directory.into();
        Self {
            settings: Settings {
                output_dir: Some(directory.clone()),
                ..Default::default()
            },
            paths: Paths {
                config_dir: directory.join("config"),
                data_dir: directory.join("data"),
                work_dir: directory.join("work"),
                output_dir: directory.clone(),
            },
            downloader_kind: DownloaderKind::SteamCmd,
            dry_run: false,
            fresh: false,
            credentials: Credentials::anonymous(),
            depot_names: DepotNames::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CheckpointStage {
    Downloaded,
    Normalized,
}

fn checkpoint_format_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
struct Checkpoint {
    #[serde(default = "checkpoint_format_version")]
    version: u32,
    stage: CheckpointStage,
    content_dir: PathBuf,
}

fn checkpoint_path(work_dir: &Path) -> PathBuf {
    work_dir.join(".steam-press-state.json")
}

fn read_checkpoint(work_dir: &Path) -> Option<(CheckpointStage, PathBuf)> {
    let text = fs::read_to_string(checkpoint_path(work_dir)).ok()?;
    serde_json::from_str::<Checkpoint>(&text)
        .ok()
        .map(|checkpoint| (checkpoint.stage, checkpoint.content_dir))
}

fn write_checkpoint(work_dir: &Path, stage: CheckpointStage, content_dir: &Path) -> io::Result<()> {
    let text = serde_json::to_string(&Checkpoint {
        version: checkpoint_format_version(),
        stage,
        content_dir: content_dir.to_path_buf(),
    })
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(checkpoint_path(work_dir), text)
}

fn estimated_download_bytes(preflight: &Preflight) -> Option<u64> {
    let sizes: Vec<u64> = preflight
        .depots
        .iter()
        .filter_map(|depot| depot.size_bytes)
        .collect();
    if sizes.is_empty() {
        None
    } else {
        Some(sizes.into_iter().sum())
    }
}

fn check_disk_space(
    settings: &Settings,
    preflight: &Preflight,
    work_dir: &Path,
    output_dir: &Path,
) -> Result<Option<u64>, String> {
    let Some(download) = estimated_download_bytes(preflight) else {
        return Ok(None);
    };
    let margin = settings.disk_margin_mb.saturating_mul(1024 * 1024);
    let required = download
        .saturating_add(download / 10)
        .saturating_add(256 * 1024 * 1024)
        .saturating_add(margin);
    for directory in [work_dir, output_dir] {
        let free = available_space(directory)
            .map_err(|error| format!("disk space check for {}: {error}", directory.display()))?;
        if free < required {
            return Err(format!(
                "not enough disk space in {}: {:.1} GiB free, {:.1} GiB required",
                directory.display(),
                free as f64 / 1024.0 / 1024.0 / 1024.0,
                required as f64 / 1024.0 / 1024.0 / 1024.0
            ));
        }
    }
    Ok(Some(required))
}

pub struct Packer {
    options: PackOptions,
    preflight: PreflightFn,
    cancel: CancellationToken,
    active_cancel: Mutex<Option<CancellationToken>>,
}

impl Packer {
    pub fn new(options: PackOptions) -> Self {
        Self::with_preflight(options, Arc::new(fetch_preflight))
    }

    pub fn with_preflight(options: PackOptions, preflight: PreflightFn) -> Self {
        Self {
            options,
            preflight,
            cancel: CancellationToken::new(),
            active_cancel: Mutex::new(None),
        }
    }

    pub fn preflight(&self, job: &GameJob) -> Result<Preflight, PipelineError> {
        (self.preflight)(job)
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
        if let Some(token) = self.active_cancel.lock().ok().and_then(|slot| slot.clone()) {
            token.cancel();
        }
    }

    pub fn run_dry(&self, job: &GameJob) -> PackOutcome {
        let started = Instant::now();
        let preflight = match (self.preflight)(job) {
            Ok(preflight) => preflight,
            Err(error) => {
                return failed_outcome(
                    String::new(),
                    started,
                    PackStatus::Failed(error.to_string()),
                )
            }
        };
        let stem = naming::output_stem(
            &preflight.name,
            &preflight.build_id,
            job.platform,
            &job.branch,
        );
        let output_dir = self.options.settings.output_dir(&self.options.paths);
        let output = archive::archive_path(&output_dir, &stem, self.options.settings.compressor);
        let status =
            if archive::archive_exists(&output_dir, &stem, self.options.settings.compressor) {
                PackStatus::Skipped
            } else {
                PackStatus::Completed
            };
        PackOutcome {
            status,
            stem,
            output: Some(output),
            info_files: Vec::new(),
            build_id: preflight.build_id,
            build_time: preflight.build_time,
            depots: preflight.depots,
            duration: started.elapsed(),
        }
    }

    pub fn run(&self, job: &GameJob, sink: EventSink) -> PackOutcome {
        let started = Instant::now();
        let label = job.encode();

        sink.emit(PackEvent::Stage {
            job: label.clone(),
            stage: Stage::Preflight,
            message: "Fetching Steam metadata".to_string(),
        });

        let preflight = match (self.preflight)(job) {
            Ok(preflight) => preflight,
            Err(error) => return self.failed(job, "", started, &sink, error.to_string()),
        };

        let stem = naming::output_stem(
            &preflight.name,
            &preflight.build_id,
            job.platform,
            &job.branch,
        );
        let output_dir = self.options.settings.output_dir(&self.options.paths);

        if let Err(error) = fs::create_dir_all(&output_dir) {
            return self.failed(
                job,
                &stem,
                started,
                &sink,
                format!("output directory: {error}"),
            );
        }

        if archive::archive_exists(&output_dir, &stem, self.options.settings.compressor) {
            let output =
                archive::archive_path(&output_dir, &stem, self.options.settings.compressor);
            sink.emit(PackEvent::Finished {
                job: label,
                status: FinishStatus::Skipped,
                outcome: "Archive already exists".to_string(),
                output: Some(output.clone()),
            });
            return PackOutcome {
                status: PackStatus::Skipped,
                stem,
                output: Some(output),
                info_files: Vec::new(),
                build_id: preflight.build_id,
                build_time: preflight.build_time,
                depots: preflight.depots,
                duration: started.elapsed(),
            };
        }

        if self.options.dry_run {
            let output =
                archive::archive_path(&output_dir, &stem, self.options.settings.compressor);
            return PackOutcome {
                status: PackStatus::Completed,
                stem,
                output: Some(output),
                info_files: Vec::new(),
                build_id: preflight.build_id,
                build_time: preflight.build_time,
                depots: preflight.depots,
                duration: started.elapsed(),
            };
        }

        if self.cancel.is_cancelled() {
            return self.cancelled(job, stem, started, preflight, &sink);
        }

        sink.emit(PackEvent::Stage {
            job: label.clone(),
            stage: Stage::Download,
            message: format!("Downloading {} ({})", preflight.name, job.platform.label()),
        });

        let work_dir = self.options.paths.work_dir.join(&stem);
        if self.options.fresh && work_dir.exists() {
            if let Err(error) = fs::remove_dir_all(&work_dir) {
                return self.failed(
                    job,
                    &stem,
                    started,
                    &sink,
                    format!("work directory: {error}"),
                );
            }
        }
        if work_dir.exists() {
            sink.emit(PackEvent::Log(
                "Resuming the existing work directory".to_string(),
            ));
        }
        if let Err(error) = fs::create_dir_all(&work_dir) {
            return self.failed(
                job,
                &stem,
                started,
                &sink,
                format!("work directory: {error}"),
            );
        }

        if let Err(message) =
            check_disk_space(&self.options.settings, &preflight, &work_dir, &output_dir)
        {
            return self.failed(job, &stem, started, &sink, message);
        }

        let checkpoint = read_checkpoint(&work_dir);
        let report = if let Some((CheckpointStage::Downloaded, content_dir)) = &checkpoint {
            sink.emit(PackEvent::Log(
                "Download already complete; skipping the download stage".to_string(),
            ));
            DownloadReport {
                content_dir: content_dir.clone(),
                app_manifest: None,
                depots: Vec::new(),
                build_id: String::new(),
            }
        } else if let Some((CheckpointStage::Normalized, content_dir)) = &checkpoint {
            sink.emit(PackEvent::Log(
                "Download and cleanup already complete; resuming at manifests".to_string(),
            ));
            DownloadReport {
                content_dir: content_dir.clone(),
                app_manifest: None,
                depots: Vec::new(),
                build_id: String::new(),
            }
        } else {
            // `cancel()` may arrive after the pre-download check and before
            // the downloader is created. Re-check here so the cancellation
            // is not lost in that race.
            if self.cancel.is_cancelled() {
                return self.cancelled(job, stem, started, preflight, &sink);
            }
            let mut downloader =
                downloader::create(self.options.downloader_kind, &self.options.settings);
            if let Ok(mut slot) = self.active_cancel.lock() {
                *slot = Some(downloader.cancel_token());
                if self.cancel.is_cancelled() {
                    downloader.cancel();
                }
            }

            let request = DownloadRequest {
                appid: job.appid.clone(),
                platform: job.platform,
                branch: job.branch.clone(),
                branch_password: if job.branch_password.trim().is_empty() {
                    None
                } else {
                    Some(job.branch_password.clone())
                },
                credentials: self.options.credentials.clone(),
                work_dir: work_dir.clone(),
                install_dir: Some(install_dir_name(&preflight)),
                depots: preflight
                    .depots
                    .iter()
                    .map(|depot| DepotSelector {
                        depot_id: depot.depot_id.clone(),
                        manifest_id: Some(depot.manifest_id.clone()),
                    })
                    .collect(),
                validate: true,
            };

            let report = match downloader.download(&request, &sink) {
                Ok(report) => report,
                Err(DownloadError::Cancelled) => {
                    return self.cancelled(job, stem, started, preflight, &sink);
                }
                Err(error) => return self.failed(job, &stem, started, &sink, error.to_string()),
            };
            if let Err(error) =
                write_checkpoint(&work_dir, CheckpointStage::Downloaded, &report.content_dir)
            {
                sink.emit(PackEvent::Log(format!("Checkpoint warning: {error}")));
            }
            report
        };

        sink.emit(PackEvent::Stage {
            job: label.clone(),
            stage: Stage::Normalize,
            message: "Cleaning the downloaded output".to_string(),
        });

        let temp = work_dir.join("temp");
        if !matches!(checkpoint, Some((CheckpointStage::Normalized, _))) {
            if let Err(error) = normalize_content(&report.content_dir, &work_dir, &temp) {
                return self.failed(job, &stem, started, &sink, format!("cleanup: {error}"));
            }
            if let Err(error) = write_checkpoint(&work_dir, CheckpointStage::Normalized, &temp) {
                sink.emit(PackEvent::Log(format!("Checkpoint warning: {error}")));
            }
        }

        sink.emit(PackEvent::Stage {
            job: label.clone(),
            stage: Stage::Manifest,
            message: "Preparing manifests".to_string(),
        });

        let steamapps = temp.join("steamapps");
        let acf_path = steamapps.join(format!("appmanifest_{}.acf", job.appid));
        let install_dir = install_dir_name(&preflight);
        let install_path = steamapps.join("common").join(&install_dir);
        let shared_depots: Vec<SharedDepot> = preflight
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
            appid: job.appid.clone(),
            name: preflight.name.clone(),
            install_dir: install_dir.clone(),
            build_id: preflight.build_id.clone(),
            size_on_disk: directory_size(&install_path),
            depots: preflight
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

        let manifest = if acf_path.is_file() {
            if let Err(error) = acf::rewrite_privacy_fields(&acf_path) {
                return self.failed(job, &stem, started, &sink, format!("manifest: {error}"));
            }
            match acf::validate_manifest(&acf_path) {
                Ok(manifest) => Some(manifest),
                Err(error) => {
                    return self.failed(job, &stem, started, &sink, format!("manifest: {error}"))
                }
            }
        } else {
            if let Err(error) = acf::write_acf(&steamapps, &acf_job) {
                return self.failed(job, &stem, started, &sink, format!("manifest: {error}"));
            }
            acf::read_manifest(&acf_path).ok()
        };

        if !shared_depots.is_empty() {
            if let Err(error) = acf::write_shared_acf(&steamapps, &acf_job) {
                return self.failed(
                    job,
                    &stem,
                    started,
                    &sink,
                    format!("shared manifest: {error}"),
                );
            }
        }

        let game_name = manifest
            .as_ref()
            .map(|manifest| manifest.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| preflight.name.clone());
        let build_id = manifest
            .as_ref()
            .map(|manifest| manifest.build_id.clone())
            .filter(|build| !build.is_empty())
            .unwrap_or_else(|| preflight.build_id.clone());

        let mut installed = manifest
            .as_ref()
            .map(|manifest| manifest.installed_depots.clone())
            .unwrap_or_default();
        installed.sort_by_key(|depot| depot.depot_id.parse::<u64>().unwrap_or(u64::MAX));

        let api_names: HashMap<String, String> = preflight
            .depots
            .iter()
            .map(|depot| (depot.depot_id.clone(), depot.name.clone()))
            .collect();

        let template_depots: Vec<TemplateDepot> = if installed.is_empty() {
            preflight
                .depots
                .iter()
                .map(|depot| TemplateDepot {
                    depot_id: depot.depot_id.clone(),
                    depot_name: self
                        .options
                        .depot_names
                        .get(&depot.depot_id)
                        .map(str::to_string)
                        .or_else(|| Some(depot.name.clone()))
                        .unwrap_or_default(),
                    manifest_id: depot.manifest_id.clone(),
                })
                .collect()
        } else {
            installed
                .iter()
                .map(|depot| TemplateDepot {
                    depot_id: depot.depot_id.clone(),
                    depot_name: self
                        .options
                        .depot_names
                        .get(&depot.depot_id)
                        .map(str::to_string)
                        .or_else(|| api_names.get(&depot.depot_id).cloned())
                        .unwrap_or_default(),
                    manifest_id: depot.manifest.clone(),
                })
                .collect()
        };

        sink.emit(PackEvent::Stage {
            job: label.clone(),
            stage: Stage::Archive,
            message: "Creating the archive".to_string(),
        });

        let output = match compress_directory(
            &self.options.settings,
            &output_dir,
            &stem,
            &temp,
            &self.cancel,
            &sink,
        ) {
            Ok(CompressOutcome::Done(path)) => path,
            Ok(CompressOutcome::Cancelled) => {
                return self.cancelled(job, stem, started, preflight, &sink);
            }
            Err(error) => return self.failed(job, &stem, started, &sink, error.to_string()),
        };

        sink.emit(PackEvent::Stage {
            job: label.clone(),
            stage: Stage::Template,
            message: "Rendering release information".to_string(),
        });

        let custom_template = self
            .options
            .settings
            .template
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok());
        let (description, website) = preflight
            .store
            .as_ref()
            .map(|store| (store.short_description.clone(), store.website.clone()))
            .unwrap_or_default();

        let data = TemplateData {
            game_name: game_name.clone(),
            os: job.platform.os_tag().to_string(),
            branch: job.branch.clone(),
            build_id: build_id.clone(),
            build_datetime: preflight
                .build_time
                .map(template::format_utc_datetime)
                .unwrap_or_default(),
            upload_datetime: template::format_utc_datetime(Utc::now()),
            app_id: job.appid.clone(),
            description,
            website,
            username: self.options.settings.uploader.clone(),
            filehost: self.options.settings.filehost.clone(),
            upload_date: Utc::now().format("%Y-%m-%d").to_string(),
            depots: template_depots.clone(),
        };

        let info_files = match template::write_info_files(
            &output_dir,
            &stem,
            &data,
            self.options.settings.upload_crew_mode,
            custom_template.as_deref(),
        ) {
            Ok(paths) => paths,
            Err(error) => return self.failed(job, &stem, started, &sink, error),
        };

        let _ = fs::remove_dir_all(&work_dir);

        sink.emit(PackEvent::Finished {
            job: label,
            status: FinishStatus::Completed,
            outcome: "Completed".to_string(),
            output: Some(output.clone()),
        });

        PackOutcome {
            status: PackStatus::Completed,
            stem,
            output: Some(output),
            info_files,
            build_id,
            build_time: preflight.build_time,
            depots: preflight.depots,
            duration: started.elapsed(),
        }
    }

    fn cancelled(
        &self,
        job: &GameJob,
        stem: String,
        started: Instant,
        preflight: Preflight,
        sink: &EventSink,
    ) -> PackOutcome {
        sink.emit(PackEvent::Finished {
            job: job.encode(),
            status: FinishStatus::Cancelled,
            outcome: "Cancelled".to_string(),
            output: None,
        });
        cancelled_outcome(stem, started, preflight)
    }

    fn failed(
        &self,
        job: &GameJob,
        stem: &str,
        started: Instant,
        sink: &EventSink,
        message: String,
    ) -> PackOutcome {
        sink.emit(PackEvent::Finished {
            job: job.encode(),
            status: FinishStatus::Failed,
            outcome: message.clone(),
            output: None,
        });
        failed_outcome(stem.to_string(), started, PackStatus::Failed(message))
    }
}

fn fetch_preflight(job: &GameJob) -> Result<Preflight, PipelineError> {
    let info = steam::fetch_app_info(&job.appid, &job.branch).map_err(PipelineError::Steam)?;
    let store = steam::fetch_store_info(&job.appid).ok();
    Ok(Preflight {
        appid: info.appid,
        name: info.name,
        install_dir: info.install_dir,
        build_id: info.build_id,
        build_time: info.build_time,
        depots: info.depots,
        store,
    })
}

fn failed_outcome(stem: String, started: Instant, status: PackStatus) -> PackOutcome {
    PackOutcome {
        status,
        stem,
        output: None,
        info_files: Vec::new(),
        build_id: String::new(),
        build_time: None,
        depots: Vec::new(),
        duration: started.elapsed(),
    }
}

fn cancelled_outcome(stem: String, started: Instant, preflight: Preflight) -> PackOutcome {
    PackOutcome {
        status: PackStatus::Cancelled,
        stem,
        output: None,
        info_files: Vec::new(),
        build_id: preflight.build_id,
        build_time: preflight.build_time,
        depots: preflight.depots,
        duration: started.elapsed(),
    }
}

pub(crate) enum CompressOutcome {
    Done(PathBuf),
    Cancelled,
}

/// Compresses (or moves) a prepared directory into the output folder.
pub(crate) fn compress_directory(
    settings: &Settings,
    output_dir: &Path,
    stem: &str,
    source_dir: &Path,
    cancel: &CancellationToken,
    sink: &EventSink,
) -> Result<CompressOutcome, PipelineError> {
    if settings.skip_compression {
        let target = output_dir.join(stem);
        if target.exists() {
            fs::remove_dir_all(&target).map_err(|error| PipelineError::Io(error.to_string()))?;
        }
        fs::rename(source_dir, &target).map_err(|error| PipelineError::Io(error.to_string()))?;
        return Ok(CompressOutcome::Done(target));
    }

    let compressor = settings.compressor;
    let binary =
        archive::compressor_binary(compressor, settings).map_err(PipelineError::Archive)?;
    let target = archive::archive_path(output_dir, stem, compressor);
    let args = compressor.args(
        settings.custom_compress_options.as_deref(),
        settings.archive_password(),
        &target,
    );
    let mut command = Command::new(binary);
    command.args(&args).current_dir(source_dir);

    match process::run_streaming(command, cancel, sink) {
        Ok(output) if output.code == 0 => {}
        Ok(output) => {
            let _ = archive::remove_archive_parts(output_dir, stem);
            return Err(PipelineError::Archive(format!(
                "{} failed (exit code {})",
                compressor.label(),
                output.code
            )));
        }
        Err(DownloadError::Cancelled) => return Ok(CompressOutcome::Cancelled),
        Err(error) => return Err(PipelineError::Download(error.to_string())),
    }

    archive::finalize_volumes(output_dir, stem, compressor)
        .map_err(|error| PipelineError::Archive(format!("volumes: {error}")))?;

    if settings.verify_archives {
        archive::verify_archive(settings, output_dir, stem, compressor)
            .map_err(PipelineError::Archive)?;
    }

    match archive::write_checksums(output_dir, stem, compressor) {
        Ok(Some(manifest)) => sink.emit(PackEvent::Log(format!(
            "Checksums written: {}",
            manifest.display()
        ))),
        Ok(None) => {}
        Err(error) => sink.emit(PackEvent::Log(format!("Checksum warning: {error}"))),
    }

    Ok(CompressOutcome::Done(target))
}

fn install_dir_name(preflight: &Preflight) -> String {
    preflight
        .install_dir
        .as_deref()
        .map(naming::sanitize_install_dir)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| naming::sanitize_install_dir(&preflight.name))
}

fn directory_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    if path.is_file() {
        return fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    }
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            total += directory_size(&entry.path());
        }
    }
    total
}

pub(crate) fn move_children(source: &Path, target: &Path, skip: &[&str]) -> io::Result<()> {
    fs::create_dir_all(target)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if skip.contains(&name.as_str()) || name.starts_with(".steam-press-") {
            continue;
        }
        paths.push(entry.path());
    }
    for path in paths {
        let Some(name) = path.file_name() else {
            continue;
        };
        let destination = target.join(name);
        if destination.exists() {
            if destination.is_dir() {
                fs::remove_dir_all(&destination)?;
            } else {
                fs::remove_file(&destination)?;
            }
        }
        fs::rename(&path, &destination)?;
    }
    Ok(())
}

fn normalize_content(content_dir: &Path, work_dir: &Path, temp: &Path) -> io::Result<()> {
    fs::create_dir_all(temp.join("steamapps"))?;
    move_children(content_dir, &temp.join("steamapps"), &["temp"])?;

    let depotcache = work_dir.join("depotcache");
    if depotcache.is_dir() {
        move_children(&depotcache, &temp.join("depotcache"), &[])?;
    }

    let steamapps = temp.join("steamapps");
    let library_folders = steamapps.join("libraryfolders.vdf");
    if library_folders.is_file() {
        fs::remove_file(&library_folders)?;
    }
    for directory in ["downloading", "temp", "workshop/downloads", "workshop/temp"] {
        let path = steamapps.join(directory);
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Platform;
    use crate::test_support::temp_dir;

    fn fixture_preflight(_: &GameJob) -> Result<Preflight, PipelineError> {
        Ok(Preflight {
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
                size_bytes: Some(1024),
                shared_owner: None,
            }],
            store: None,
        })
    }

    #[test]
    fn skips_when_archive_already_exists() {
        let dir = temp_dir("pack-skip");
        fs::write(dir.join("Balatro.Build.18674832.Linux64.public.7z"), b"x").unwrap();
        let packer =
            Packer::with_preflight(PackOptions::for_tests(dir), Arc::new(fixture_preflight));
        let outcome = packer.run(
            &GameJob::new(Platform::Linux64, "2379780"),
            EventSink::noop(),
        );
        assert!(matches!(outcome.status, PackStatus::Skipped));
        assert_eq!(outcome.stem, "Balatro.Build.18674832.Linux64.public");
    }

    #[test]
    fn dry_run_reports_naming_without_downloading() {
        let dir = temp_dir("pack-dry");
        let packer =
            Packer::with_preflight(PackOptions::for_tests(dir), Arc::new(fixture_preflight));
        let outcome = packer.run_dry(&GameJob::new(Platform::Linux64, "2379780"));
        assert!(matches!(outcome.status, PackStatus::Completed));
        assert_eq!(outcome.stem, "Balatro.Build.18674832.Linux64.public");
        assert!(outcome
            .output
            .unwrap()
            .ends_with("Balatro.Build.18674832.Linux64.public.7z"));
    }

    #[test]
    fn estimates_download_size_and_blocks_an_impossible_disk_request() {
        let dir = temp_dir("pack-disk");
        let preflight = fixture_preflight(&GameJob::new(Platform::Linux64, "2379780")).unwrap();
        assert_eq!(estimated_download_bytes(&preflight), Some(1024));
        let settings = Settings {
            disk_margin_mb: u64::MAX / 2,
            ..Default::default()
        };
        let result = check_disk_space(&settings, &preflight, &dir, &dir);
        assert!(result.is_err());
    }

    #[test]
    fn checkpoints_round_trip_in_the_work_directory() {
        let dir = temp_dir("pack-checkpoint");
        write_checkpoint(&dir, CheckpointStage::Downloaded, &dir.join("steamapps")).unwrap();
        let (stage, content) = read_checkpoint(&dir).unwrap();
        assert_eq!(stage, CheckpointStage::Downloaded);
        assert_eq!(content, dir.join("steamapps"));

        std::fs::write(
            checkpoint_path(&dir),
            r#"{"stage":"downloaded","content_dir":"legacy"}"#,
        )
        .unwrap();
        assert_eq!(
            read_checkpoint(&dir).unwrap().0,
            CheckpointStage::Downloaded
        );
    }

    #[cfg(unix)]
    fn fake_steamcmd(dir: &Path, exit_code: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/appmanifest_2379780.acf");
        let script = dir.join("fake-steamcmd.sh");
        let body = format!(
            "#!/bin/sh\nset -eu\ndir=\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    +force_install_dir) dir=\"$2\"; shift 2 ;;\n    +runscript) shift 2 ;;\n    *) shift ;;\n  esac\ndone\nmkdir -p \"$dir/steamapps/common/Balatro\"\nprintf 'game' > \"$dir/steamapps/common/Balatro/game.bin\"\ncp '{}' \"$dir/steamapps/appmanifest_2379780.acf\"\necho \"Update state (0x61) downloading, progress: 50.00 (100 / 200)\"\necho \"Success! App '2379780' fully installed.\"\nexit {}\n",
            fixture.display(),
            exit_code
        );
        fs::write(&script, body).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    #[test]
    fn completes_with_a_fake_steamcmd_and_no_real_account() {
        let dir = temp_dir("pack-fake-steamcmd");
        let mut options = PackOptions::for_tests(&dir);
        options.settings.steamcmd_path = Some(fake_steamcmd(&dir, 0));
        options.settings.skip_compression = true;
        options.settings.verify_archives = false;
        let packer = Packer::with_preflight(options, Arc::new(fixture_preflight));
        let (sink, receiver) = EventSink::channel();
        let outcome = packer.run(&GameJob::new(Platform::Linux64, "2379780"), sink);
        assert!(
            matches!(outcome.status, PackStatus::Completed),
            "{:?}",
            outcome.status
        );
        let mut saw_progress = false;
        while let Ok(event) = receiver.try_recv() {
            if let PackEvent::Log(line) = event {
                saw_progress |= line.contains("progress: 50.00");
            }
        }
        assert!(saw_progress, "fake SteamCMD progress was not forwarded");
        let output = outcome.output.unwrap();
        assert!(output.join("steamapps/common/Balatro/game.bin").is_file());
        assert!(output.join("steamapps/appmanifest_2379780.acf").is_file());
        assert!(!dir.join("work").join(&outcome.stem).exists());
    }

    #[cfg(unix)]
    fn fake_depotdownloader(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let script = dir.join("fake-DepotDownloader.sh");
        fs::write(
            &script,
            "#!/bin/sh\nset -eu\ndir=\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    -dir) dir=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nmkdir -p \"$dir/.DepotDownloader\"\nprintf 'game' > \"$dir/game.bin\"\nprintf 'metadata' > \"$dir/.DepotDownloader/state\"\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    #[test]
    fn depotdownloader_content_lands_in_common_and_metadata_is_removed() {
        let dir = temp_dir("pack-fake-depotdownloader");
        let mut options = PackOptions::for_tests(&dir);
        options.downloader_kind = DownloaderKind::DepotDownloader;
        options.settings.depotdownloader_path = Some(fake_depotdownloader(&dir));
        options.settings.skip_compression = true;
        options.settings.verify_archives = false;
        let packer = Packer::with_preflight(options, Arc::new(fixture_preflight));
        let outcome = packer.run(
            &GameJob::new(Platform::Linux64, "2379780"),
            EventSink::noop(),
        );
        assert!(
            matches!(outcome.status, PackStatus::Completed),
            "{:?}",
            outcome.status
        );
        let output = outcome.output.unwrap();
        assert!(output.join("steamapps/common/Balatro/game.bin").is_file());
        assert!(!output.join("steamapps/game.bin").exists());
        assert!(!output.join("steamapps/.DepotDownloader").exists());
        let acf = fs::read_to_string(output.join("steamapps/appmanifest_2379780.acf")).unwrap();
        assert!(acf.contains("\"SizeOnDisk\"\t\t\"4\""), "{acf}");
    }

    #[cfg(unix)]
    fn fake_steamcmd_error(dir: &Path, message: &str, exit_code: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let script = dir.join("fake-steamcmd-error.sh");
        fs::write(
            &script,
            format!("#!/bin/sh\necho '{message}'\nexit {exit_code}\n"),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    #[test]
    fn maps_fake_steamcmd_error_outputs() {
        for (message, expected) in [
            ("No subscription", "does not own"),
            ("Invalid Password", "invalid Steam credentials"),
            ("Rate Limit Exceeded", "rate limiting"),
            ("Steam Guard required", "Steam Guard"),
            ("Failed to connect to Steam", "network error"),
        ] {
            let dir = temp_dir(&format!("pack-fake-error-{}", expected.len()));
            let mut options = PackOptions::for_tests(&dir);
            options.settings.steamcmd_path = Some(fake_steamcmd_error(&dir, message, 1));
            options.settings.skip_compression = true;
            let packer = Packer::with_preflight(options, Arc::new(fixture_preflight));
            let outcome = packer.run(
                &GameJob::new(Platform::Linux64, "2379780"),
                EventSink::noop(),
            );
            match outcome.status {
                PackStatus::Failed(text) => assert!(text.contains(expected), "{text}"),
                other => panic!("expected failure for {message}, got {other:?}"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn reports_a_fake_steamcmd_failure() {
        let dir = temp_dir("pack-fake-steamcmd-failure");
        let mut options = PackOptions::for_tests(&dir);
        options.settings.steamcmd_path = Some(fake_steamcmd(&dir, 3));
        options.settings.skip_compression = true;
        let packer = Packer::with_preflight(options, Arc::new(fixture_preflight));
        let outcome = packer.run(
            &GameJob::new(Platform::Linux64, "2379780"),
            EventSink::noop(),
        );
        match outcome.status {
            PackStatus::Failed(message) => assert!(message.contains("code 3"), "{message}"),
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancels_a_running_fake_download() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("pack-fake-cancel");
        let script = dir.join("slow-steamcmd.sh");
        fs::write(&script, "#!/bin/sh\necho 'Downloading...'\nsleep 30\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut options = PackOptions::for_tests(&dir);
        options.settings.steamcmd_path = Some(script);
        options.settings.skip_compression = true;
        let packer = Arc::new(Packer::with_preflight(options, Arc::new(fixture_preflight)));
        let (sink, receiver) = EventSink::channel();
        let worker = {
            let packer = Arc::clone(&packer);
            std::thread::spawn(move || {
                packer.run(&GameJob::new(Platform::Linux64, "2379780"), sink)
            })
        };

        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if matches!(
                receiver.recv_timeout(Duration::from_millis(100)),
                Ok(PackEvent::Stage {
                    stage: Stage::Download,
                    ..
                })
            ) {
                break;
            }
        }
        packer.cancel();
        let outcome = worker.join().unwrap();
        assert!(
            matches!(outcome.status, PackStatus::Cancelled),
            "{:?}",
            outcome.status
        );
    }

    #[test]
    fn cancellation_is_observable_before_downloading() {
        let dir = temp_dir("pack-cancel");
        let packer =
            Packer::with_preflight(PackOptions::for_tests(dir), Arc::new(fixture_preflight));
        packer.cancel();
        let (sink, receiver) = EventSink::channel();
        let outcome = packer.run(&GameJob::new(Platform::Linux64, "2379780"), sink);
        assert!(matches!(outcome.status, PackStatus::Cancelled));
        assert!(receiver.try_iter().any(|event| matches!(
            event,
            PackEvent::Finished {
                status: FinishStatus::Cancelled,
                ..
            }
        )));
    }
}
