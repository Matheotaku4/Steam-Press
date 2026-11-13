//! DepotDownloader engine: deterministic depot selection through the sidecar.

use super::process::run_streaming;
use super::{
    DownloadError, DownloadReport, DownloadRequest, Downloader, DownloaderKind, ToolStatus,
    WorkshopReport, WorkshopRequest,
};
use crate::config::Settings;
use crate::events::{CancellationToken, EventSink, PackEvent};
use crate::tooling;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub struct DepotDownloader {
    binary: Option<PathBuf>,
    cancel: CancellationToken,
}

impl DepotDownloader {
    pub fn new(binary: Option<PathBuf>) -> Self {
        Self {
            binary,
            cancel: CancellationToken::new(),
        }
    }

    pub fn from_settings(settings: &Settings) -> Self {
        Self::new(settings.depotdownloader_path.clone())
    }

    fn locate(&self) -> Result<PathBuf, DownloadError> {
        tooling::find_executable(
            self.binary.as_deref(),
            &["DepotDownloader", "depotdownloader"],
        )
        .ok_or_else(|| DownloadError::MissingTool {
            tool: "DepotDownloader".to_string(),
            expected: "DepotDownloader binary in PATH or `depotdownloaderpath` in the settings"
                .to_string(),
        })
    }
}

impl Downloader for DepotDownloader {
    fn kind(&self) -> DownloaderKind {
        DownloaderKind::DepotDownloader
    }

    fn check(&self) -> ToolStatus {
        match tooling::find_executable(
            self.binary.as_deref(),
            &["DepotDownloader", "depotdownloader"],
        ) {
            Some(path) => ToolStatus {
                available: true,
                path: Some(path),
                detail: "DepotDownloader found".to_string(),
            },
            None => ToolStatus {
                available: false,
                path: None,
                detail: "DepotDownloader not found".to_string(),
            },
        }
    }

    fn download(
        &mut self,
        request: &DownloadRequest,
        sink: &EventSink,
    ) -> Result<DownloadReport, DownloadError> {
        let binary = self.locate()?;
        fs::create_dir_all(&request.work_dir)
            .map_err(|error| DownloadError::Failed(format!("work directory: {error}")))?;

        let args = build_args(request);
        sink.emit(PackEvent::Log(format!(
            "DepotDownloader : app {} ({})",
            request.appid,
            request.platform.label()
        )));

        let mut command = Command::new(binary);
        command.args(&args).current_dir(&request.work_dir);
        let output = run_streaming(command, &self.cancel, sink)?;

        if output.code != 0 {
            return Err(DownloadError::Failed(format!(
                "DepotDownloader exited with code {}",
                output.code
            )));
        }

        let manifest = request
            .work_dir
            .join(format!("appmanifest_{}.acf", request.appid));
        Ok(DownloadReport {
            content_dir: request.work_dir.clone(),
            app_manifest: if manifest.is_file() {
                Some(manifest)
            } else {
                None
            },
            depots: Vec::new(),
            build_id: String::new(),
        })
    }

    fn download_workshop(
        &mut self,
        request: &WorkshopRequest,
        sink: &EventSink,
    ) -> Result<WorkshopReport, DownloadError> {
        let binary = self.locate()?;
        fs::create_dir_all(&request.work_dir)
            .map_err(|error| DownloadError::Failed(format!("work directory: {error}")))?;

        let mut command = Command::new(binary);
        command
            .arg("-app")
            .arg(&request.appid)
            .arg("-pubfile")
            .arg(&request.workshop_id)
            .arg("-dir")
            .arg(&request.work_dir);
        if !request.credentials.is_anonymous() {
            command
                .arg("-username")
                .arg(request.credentials.username.trim());
        }

        let output = run_streaming(command, &self.cancel, sink)?;
        if output.code != 0 {
            return Err(DownloadError::Failed(format!(
                "DepotDownloader exited with code {}",
                output.code
            )));
        }

        Ok(WorkshopReport {
            content_dir: request.work_dir.clone(),
        })
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }

    fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

/// Builds sidecar arguments. The password never appears here: it would be
/// visible in the process list.
pub fn build_args(request: &DownloadRequest) -> Vec<String> {
    let mut args = vec!["-app".to_string(), request.appid.trim().to_string()];

    for depot in &request.depots {
        args.push("-depot".to_string());
        args.push(depot.depot_id.clone());
        if let Some(manifest) = &depot.manifest_id {
            args.push("-manifest".to_string());
            args.push(manifest.clone());
        }
    }

    if !request.branch.trim().eq_ignore_ascii_case("public") {
        args.push("-branch".to_string());
        args.push(request.branch.trim().to_string());
        if let Some(password) = request
            .branch_password
            .as_deref()
            .map(str::trim)
            .filter(|password| !password.is_empty())
        {
            args.push("-branchpassword".to_string());
            args.push(password.to_string());
        }
    }

    args.push("-os".to_string());
    args.push(request.platform.depotdownloader_os().to_string());
    args.push("-dir".to_string());
    args.push(request.work_dir.to_string_lossy().into_owned());

    if request.validate {
        args.push("-validate".to_string());
    }

    if !request.credentials.is_anonymous() {
        args.push("-username".to_string());
        args.push(request.credentials.username.trim().to_string());
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::{Credentials, DepotSelector};
    use crate::platform::Platform;
    use crate::queue::GameJob;
    use std::path::Path;

    #[test]
    fn builds_args_for_selected_depots() {
        let mut request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780").with_branch("beta", "pw2"),
            Credentials {
                username: "alice".into(),
                password: Some("secret".into()),
            },
            Path::new("/tmp/dl"),
        );
        request.depots = vec![
            DepotSelector {
                depot_id: "2379781".into(),
                manifest_id: Some("111".into()),
            },
            DepotSelector {
                depot_id: "228989".into(),
                manifest_id: None,
            },
        ];

        let args = build_args(&request);
        let has_pair = |left: &str, right: &str| {
            args.windows(2)
                .any(|window| window[0] == left && window[1] == right)
        };

        assert!(has_pair("-app", "2379780"));
        assert!(has_pair("-depot", "2379781"));
        assert!(has_pair("-manifest", "111"));
        assert!(has_pair("-depot", "228989"));
        assert!(has_pair("-branch", "beta"));
        assert!(has_pair("-branchpassword", "pw2"));
        assert!(has_pair("-os", "linux"));
        assert!(has_pair("-dir", "/tmp/dl"));
        assert!(args.contains(&"-validate".to_string()));
        assert!(!args.join(" ").contains("secret"));
    }
}
