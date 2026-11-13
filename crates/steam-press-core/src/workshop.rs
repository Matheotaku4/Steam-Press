//! Workshop item pipeline: download, normalize, archive.

use crate::archive;
use crate::downloader::{self, Credentials, DownloadError, WorkshopRequest};
use crate::events::{CancellationToken, EventSink, FinishStatus, PackEvent, Stage};
use crate::pack::{compress_directory, move_children, CompressOutcome, PackOptions};
use crate::queue::WorkshopJob;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::pack::PackStatus;

#[derive(Debug, Clone)]
pub struct WorkshopOutcome {
    pub status: PackStatus,
    pub stem: String,
    pub output: Option<PathBuf>,
    pub duration: Duration,
}

impl fmt::Display for WorkshopOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = match &self.status {
            PackStatus::Completed => "Completed".to_string(),
            PackStatus::Skipped => "Skipped".to_string(),
            PackStatus::Failed(message) => format!("Failed: {message}"),
            PackStatus::Cancelled => "Cancelled".to_string(),
        };
        write!(formatter, "{} - {status}", self.stem)
    }
}

pub struct WorkshopPacker {
    options: PackOptions,
    cancel: CancellationToken,
    active_cancel: Mutex<Option<CancellationToken>>,
}

impl WorkshopPacker {
    pub fn new(options: PackOptions) -> Self {
        Self {
            options,
            cancel: CancellationToken::new(),
            active_cancel: Mutex::new(None),
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
        if let Some(token) = self.active_cancel.lock().ok().and_then(|slot| slot.clone()) {
            token.cancel();
        }
    }

    pub fn run(&self, job: &WorkshopJob, sink: EventSink) -> WorkshopOutcome {
        let started = Instant::now();
        let stem = format!("Workshop_{}_{}", job.appid.trim(), job.workshop_id.trim());
        let output_dir = self.options.settings.output_dir(&self.options.paths);

        if let Err(error) = fs::create_dir_all(&output_dir) {
            return self.failed(&stem, started, &sink, format!("output directory: {error}"));
        }

        if archive::archive_exists(&output_dir, &stem, self.options.settings.compressor) {
            let output =
                archive::archive_path(&output_dir, &stem, self.options.settings.compressor);
            sink.emit(PackEvent::Finished {
                job: stem.clone(),
                status: FinishStatus::Skipped,
                outcome: "Archive already exists".to_string(),
                output: Some(output.clone()),
            });
            return WorkshopOutcome {
                status: PackStatus::Skipped,
                stem,
                output: Some(output),
                duration: started.elapsed(),
            };
        }

        if self.cancel.is_cancelled() {
            return self.cancelled(stem, started, &sink);
        }

        sink.emit(PackEvent::Stage {
            job: stem.clone(),
            stage: Stage::Download,
            message: format!(
                "Downloading workshop item {} / {}",
                job.appid, job.workshop_id
            ),
        });

        let work_dir = self.options.paths.work_dir.join(&stem);
        if work_dir.exists() {
            if let Err(error) = fs::remove_dir_all(&work_dir) {
                return self.failed(&stem, started, &sink, format!("work directory: {error}"));
            }
        }
        if let Err(error) = fs::create_dir_all(&work_dir) {
            return self.failed(&stem, started, &sink, format!("work directory: {error}"));
        }

        let mut downloader =
            downloader::create(self.options.downloader_kind, &self.options.settings);
        if let Ok(mut slot) = self.active_cancel.lock() {
            *slot = Some(downloader.cancel_token());
        }

        let request = WorkshopRequest::new(
            job.clone(),
            Credentials {
                username: self.options.credentials.username.clone(),
                password: self.options.credentials.password.clone(),
            },
            work_dir.clone(),
        );

        let report = match downloader.download_workshop(&request, &sink) {
            Ok(report) => report,
            Err(DownloadError::Cancelled) => return self.cancelled(stem, started, &sink),
            Err(error) => return self.failed(&stem, started, &sink, error.to_string()),
        };

        sink.emit(PackEvent::Stage {
            job: stem.clone(),
            stage: Stage::Normalize,
            message: "Preparing workshop content".to_string(),
        });

        let temp = work_dir.join("temp");
        if let Err(error) = fs::create_dir_all(&temp) {
            return self.failed(&stem, started, &sink, format!("content: {error}"));
        }
        if let Err(error) = move_children(&report.content_dir, &temp, &["temp"]) {
            return self.failed(&stem, started, &sink, format!("content: {error}"));
        }

        sink.emit(PackEvent::Stage {
            job: stem.clone(),
            stage: Stage::Archive,
            message: "Creating the workshop archive".to_string(),
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
            Ok(CompressOutcome::Cancelled) => return self.cancelled(stem, started, &sink),
            Err(error) => return self.failed(&stem, started, &sink, error.to_string()),
        };

        let _ = fs::remove_dir_all(&work_dir);

        sink.emit(PackEvent::Finished {
            job: stem.clone(),
            status: FinishStatus::Completed,
            outcome: "Completed".to_string(),
            output: Some(output.clone()),
        });

        WorkshopOutcome {
            status: PackStatus::Completed,
            stem,
            output: Some(output),
            duration: started.elapsed(),
        }
    }

    fn cancelled(&self, stem: String, started: Instant, sink: &EventSink) -> WorkshopOutcome {
        sink.emit(PackEvent::Finished {
            job: stem.clone(),
            status: FinishStatus::Cancelled,
            outcome: "Cancelled".to_string(),
            output: None,
        });
        WorkshopOutcome {
            status: PackStatus::Cancelled,
            stem,
            output: None,
            duration: started.elapsed(),
        }
    }

    fn failed(
        &self,
        stem: &str,
        started: Instant,
        sink: &EventSink,
        message: String,
    ) -> WorkshopOutcome {
        sink.emit(PackEvent::Finished {
            job: stem.to_string(),
            status: FinishStatus::Failed,
            outcome: message.clone(),
            output: None,
        });
        WorkshopOutcome {
            status: PackStatus::Failed(message),
            stem: stem.to_string(),
            output: None,
            duration: started.elapsed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    #[test]
    fn workshop_skips_existing_archive() {
        let dir = temp_dir("workshop-skip");
        fs::write(dir.join("Workshop_730_999.7z"), b"x").unwrap();
        let packer = WorkshopPacker::new(PackOptions::for_tests(dir));
        let outcome = packer.run(&WorkshopJob::new("730", "999"), EventSink::noop());
        assert!(matches!(outcome.status, PackStatus::Skipped));
        assert_eq!(outcome.stem, "Workshop_730_999");
    }
}
