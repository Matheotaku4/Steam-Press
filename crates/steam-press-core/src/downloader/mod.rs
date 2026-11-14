//! Downloader abstraction shared by SteamCMD and DepotDownloader.

pub mod depotdownloader;
pub(crate) mod process;
pub mod steamcmd;

use crate::config::Settings;
use crate::events::EventSink;
use crate::platform::Platform;
use crate::queue::{GameJob, WorkshopJob};
use crate::steam::DepotEntry;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloaderKind {
    #[default]
    SteamCmd,
    DepotDownloader,
}

impl DownloaderKind {
    pub fn setting(&self) -> &'static str {
        match self {
            Self::SteamCmd => "steamcmd",
            Self::DepotDownloader => "depotdownloader",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::SteamCmd => "SteamCMD",
            Self::DepotDownloader => "DepotDownloader",
        }
    }

    pub fn from_setting(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "steamcmd" => Some(Self::SteamCmd),
            "depotdownloader" | "depot" => Some(Self::DepotDownloader),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: Option<String>,
}

impl Credentials {
    pub fn anonymous() -> Self {
        Self {
            username: "anonymous".to_string(),
            password: None,
        }
    }

    pub fn is_anonymous(&self) -> bool {
        self.username.trim().is_empty() || self.username.trim().eq_ignore_ascii_case("anonymous")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotSelector {
    pub depot_id: String,
    pub manifest_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub appid: String,
    pub platform: Platform,
    pub branch: String,
    pub branch_password: Option<String>,
    pub credentials: Credentials,
    pub work_dir: PathBuf,
    /// Steam install directory. DepotDownloader writes files at the root of
    /// `work_dir`, so the engine uses this to recreate Steam's
    /// `steamapps/common/<install_dir>` layout.
    pub install_dir: Option<String>,
    pub depots: Vec<DepotSelector>,
    pub validate: bool,
}

impl DownloadRequest {
    pub fn for_job(job: GameJob, credentials: Credentials, work_dir: impl Into<PathBuf>) -> Self {
        let branch_password = if job.branch_password.trim().is_empty() {
            None
        } else {
            Some(job.branch_password.clone())
        };
        Self {
            appid: job.appid,
            platform: job.platform,
            branch: job.branch,
            branch_password,
            credentials,
            work_dir: work_dir.into(),
            install_dir: None,
            depots: Vec::new(),
            validate: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkshopRequest {
    pub appid: String,
    pub workshop_id: String,
    pub credentials: Credentials,
    pub work_dir: PathBuf,
}

impl WorkshopRequest {
    pub fn new(job: WorkshopJob, credentials: Credentials, work_dir: impl Into<PathBuf>) -> Self {
        Self {
            appid: job.appid,
            workshop_id: job.workshop_id,
            credentials,
            work_dir: work_dir.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadReport {
    pub content_dir: PathBuf,
    pub app_manifest: Option<PathBuf>,
    pub depots: Vec<DepotEntry>,
    pub build_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopReport {
    pub content_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    MissingTool { tool: String, expected: String },
    InvalidCredentials,
    RateLimited,
    NoSubscription,
    SteamGuardRequired,
    Failed(String),
    Cancelled,
}

impl fmt::Display for DownloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTool { tool, expected } => {
                write!(formatter, "{tool} not found ({expected})")
            }
            Self::InvalidCredentials => {
                formatter.write_str("invalid Steam credentials or Steam Guard required")
            }
            Self::RateLimited => {
                formatter.write_str("Steam is temporarily rate limiting downloads")
            }
            Self::NoSubscription => formatter.write_str("this account does not own the game"),
            Self::SteamGuardRequired => {
                formatter.write_str("Steam Guard must be approved before continuing")
            }
            Self::Failed(message) => formatter.write_str(message),
            Self::Cancelled => formatter.write_str("operation cancelled"),
        }
    }
}

impl std::error::Error for DownloadError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStatus {
    pub available: bool,
    pub path: Option<PathBuf>,
    pub detail: String,
}

pub trait Downloader: Send {
    fn kind(&self) -> DownloaderKind;
    fn check(&self) -> ToolStatus;
    fn download(
        &mut self,
        request: &DownloadRequest,
        sink: &EventSink,
    ) -> Result<DownloadReport, DownloadError>;
    fn download_workshop(
        &mut self,
        request: &WorkshopRequest,
        sink: &EventSink,
    ) -> Result<WorkshopReport, DownloadError>;
    fn cancel(&self);
    fn cancel_token(&self) -> crate::events::CancellationToken;
}

/// Builds the configured engine. Both engines are stateless apart from their
/// cancel token, so callers may create one per queue.
pub fn create(kind: DownloaderKind, settings: &Settings) -> Box<dyn Downloader> {
    match kind {
        DownloaderKind::SteamCmd => Box::new(steamcmd::SteamCmd::from_settings(settings)),
        DownloaderKind::DepotDownloader => {
            Box::new(depotdownloader::DepotDownloader::from_settings(settings))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloader_kind_settings_round_trip() {
        assert_eq!(
            DownloaderKind::from_setting("steamcmd"),
            Some(DownloaderKind::SteamCmd)
        );
        assert_eq!(
            DownloaderKind::from_setting("DepotDownloader"),
            Some(DownloaderKind::DepotDownloader)
        );
        assert_eq!(DownloaderKind::SteamCmd.setting(), "steamcmd");
    }

    #[test]
    fn anonymous_detection_and_branch_password() {
        assert!(Credentials::anonymous().is_anonymous());
        assert!(!Credentials {
            username: "alice".into(),
            password: None
        }
        .is_anonymous());

        let request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "10").with_branch("beta", "pw"),
            Credentials::anonymous(),
            "/tmp/work",
        );
        assert_eq!(request.branch_password.as_deref(), Some("pw"));
    }
}
