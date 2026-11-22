//! SteamCMD engine: scripts, execution and log classification.

use super::process::run_streaming;
use super::{
    DownloadError, DownloadReport, DownloadRequest, Downloader, DownloaderKind, ToolStatus,
    WorkshopReport, WorkshopRequest,
};
use crate::config::Settings;
use crate::events::{CancellationToken, EventSink, PackEvent};
use crate::tooling;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct SteamCmd {
    binary: Option<PathBuf>,
    cancel: CancellationToken,
}

impl SteamCmd {
    pub fn new(binary: Option<PathBuf>) -> Self {
        Self {
            binary,
            cancel: CancellationToken::new(),
        }
    }

    pub fn from_settings(settings: &Settings) -> Self {
        Self::new(settings.steamcmd_path.clone())
    }

    fn locate(&self) -> Result<PathBuf, DownloadError> {
        tooling::find_executable(self.binary.as_deref(), &["steamcmd.sh", "steamcmd"]).ok_or_else(
            || DownloadError::MissingTool {
                tool: "SteamCMD".to_string(),
                expected: "steamcmd.sh/steamcmd in PATH or `steamcmdpath` in the settings"
                    .to_string(),
            },
        )
    }

    fn login_lines(
        credentials: &super::Credentials,
        steam_guard_code: Option<&str>,
    ) -> Vec<String> {
        if credentials.is_anonymous() {
            return vec!["login anonymous".to_string()];
        }
        let user = credentials.username.trim();
        let password = credentials
            .password
            .as_deref()
            .map(str::trim)
            .filter(|password| !password.is_empty());
        let code = steam_guard_code
            .map(str::trim)
            .filter(|code| !code.is_empty());

        match (password, code) {
            (Some(password), Some(code)) => vec![format!("login {user} {password} {code}")],
            (Some(password), None) => vec![format!("login {user} {password}")],
            (None, Some(code)) => vec![
                format!("set_steam_guard_code {code}"),
                format!("login {user}"),
            ],
            (None, None) => vec![format!("login {user}")],
        }
    }
}

impl Downloader for SteamCmd {
    fn kind(&self) -> DownloaderKind {
        DownloaderKind::SteamCmd
    }

    fn check(&self) -> ToolStatus {
        match tooling::find_executable(self.binary.as_deref(), &["steamcmd.sh", "steamcmd"]) {
            Some(path) => ToolStatus {
                available: true,
                path: Some(path),
                detail: "SteamCMD found".to_string(),
            },
            None => ToolStatus {
                available: false,
                path: None,
                detail: "SteamCMD not found".to_string(),
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

        let script_path = request
            .work_dir
            .join(format!(".steam-press-{}.job", request.appid));
        tooling::write_private(&script_path, &build_script(request))
            .map_err(|error| DownloadError::Failed(format!("script SteamCMD : {error}")))?;

        sink.emit(PackEvent::Log(format!(
            "SteamCMD : app_update {} ({})",
            request.appid,
            request.platform.label()
        )));

        let mut command = Command::new(&binary);
        command
            .arg("+force_install_dir")
            .arg(&request.work_dir)
            .arg("+runscript")
            .arg(&script_path)
            .current_dir(&request.work_dir);

        let result = run_streaming(command, &self.cancel, sink);
        let _ = fs::remove_file(&script_path);
        let output = result?;

        let steamapps = request.work_dir.join("steamapps");
        classify_run(
            output.code,
            &output.output,
            "",
            steamapps.is_dir() || output.code == 0,
            true,
        )?;

        let manifest = steamapps.join(format!("appmanifest_{}.acf", request.appid));
        Ok(DownloadReport {
            content_dir: steamapps,
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

        let script_path = request
            .work_dir
            .join(format!(".steam-press-workshop-{}.job", request.workshop_id));
        tooling::write_private(&script_path, &build_workshop_script(request))
            .map_err(|error| DownloadError::Failed(format!("script SteamCMD : {error}")))?;

        sink.emit(PackEvent::Log(format!(
            "SteamCMD : workshop_download_item {} {}",
            request.appid, request.workshop_id
        )));

        let mut command = Command::new(&binary);
        command
            .arg("+force_install_dir")
            .arg(&request.work_dir)
            .arg("+runscript")
            .arg(&script_path)
            .current_dir(&request.work_dir);

        let result = run_streaming(command, &self.cancel, sink);
        let _ = fs::remove_file(&script_path);
        let output = result?;
        classify_run(output.code, &output.output, "", true, true)?;

        let content = request
            .work_dir
            .join("steamapps/workshop/content")
            .join(&request.appid)
            .join(&request.workshop_id);
        Ok(WorkshopReport {
            content_dir: if content.is_dir() {
                content
            } else {
                request.work_dir.clone()
            },
        })
    }

    fn cancel(&self) {
        self.cancel.cancel();
    }

    fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

/// Builds the `+runscript` content for a game download.
pub fn build_script(request: &DownloadRequest) -> String {
    let mut lines =
        SteamCmd::login_lines(&request.credentials, request.steam_guard_code.as_deref());
    lines.extend(request.platform.steamcmd_directives());

    let mut update = format!("app_update {}", request.appid.trim());
    if !request.branch.trim().eq_ignore_ascii_case("public") {
        update.push_str(&format!(" -beta {}", request.branch.trim()));
        if let Some(password) = request
            .branch_password
            .as_deref()
            .map(str::trim)
            .filter(|password| !password.is_empty())
        {
            update.push_str(&format!(" -betapassword {password}"));
        }
    }
    if request.validate {
        update.push_str(" validate");
    }
    lines.push(update);
    lines.push("quit".to_string());
    lines.join("\n")
}

pub fn build_workshop_script(request: &WorkshopRequest) -> String {
    let mut lines =
        SteamCmd::login_lines(&request.credentials, request.steam_guard_code.as_deref());
    lines.push(format!(
        "workshop_download_item {} {} validate",
        request.appid.trim(),
        request.workshop_id.trim()
    ));
    lines.push("quit".to_string());
    lines.join("\n")
}

/// Maps SteamCMD output to a typed error. Priority mirrors SuperSteamPacker.
pub fn classify_run(
    exit_code: i32,
    content_log: &str,
    connection_log: &str,
    has_sitelicense: bool,
    has_compat_log: bool,
) -> Result<(), DownloadError> {
    let combined = format!("{content_log}\n{connection_log}");

    if combined.contains("Rate Limit Exceeded") {
        return Err(DownloadError::RateLimited);
    }
    if combined.contains("No subscription") {
        return Err(DownloadError::NoSubscription);
    }
    if combined.contains("Invalid Password") {
        return Err(DownloadError::InvalidCredentials);
    }
    let lower = combined.to_ascii_lowercase();
    if lower.contains("steam guard") {
        return Err(DownloadError::SteamGuardRequired);
    }
    if lower.contains("failed to connect")
        || lower.contains("connection timeout")
        || lower.contains("no connection")
    {
        return Err(DownloadError::Failed(
            "could not reach Steam (network error); check the proxy and firewall".to_string(),
        ));
    }
    if exit_code != 0 {
        if !has_sitelicense && !has_compat_log {
            return Err(DownloadError::SteamGuardRequired);
        }
        return Err(DownloadError::Failed(format!(
            "steamcmd exited with code {exit_code}"
        )));
    }
    Ok(())
}

pub fn log_paths(root: &Path) -> (PathBuf, PathBuf) {
    (
        root.join("logs/content_log.txt"),
        root.join("logs/connection_log.txt"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::Credentials;
    use crate::platform::Platform;
    use crate::queue::GameJob;

    #[test]
    fn builds_public_script_with_platform_and_validate() {
        let request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials {
                username: "alice".into(),
                password: Some("pw".into()),
            },
            "/tmp/dl",
        );
        assert_eq!(
            build_script(&request),
            "login alice pw\n@sSteamCmdForcePlatformType linux\n@sSteamCmdForcePlatformBitness 64\napp_update 2379780 validate\nquit"
        );
    }

    #[test]
    fn builds_protected_beta_script() {
        let request = DownloadRequest::for_job(
            GameJob::new(Platform::Win64, "123").with_branch("beta", "pw2"),
            Credentials {
                username: "alice".into(),
                password: None,
            },
            "/tmp/dl",
        );
        assert!(
            build_script(&request).contains("app_update 123 -beta beta -betapassword pw2 validate")
        );
    }

    #[test]
    fn builds_workshop_scripts() {
        let request = WorkshopRequest::new(
            crate::queue::WorkshopJob::new("730", "999"),
            Credentials::anonymous(),
            "/tmp/dl",
        );
        assert_eq!(
            build_workshop_script(&request),
            "login anonymous\nworkshop_download_item 730 999 validate\nquit"
        );
    }

    #[test]
    fn builds_script_with_a_steam_guard_code() {
        let mut request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials {
                username: "alice".into(),
                password: Some("pw".into()),
            },
            "/tmp/dl",
        );
        request.steam_guard_code = Some("  ABCDE  ".into());
        assert!(build_script(&request).starts_with("login alice pw ABCDE\n"));
    }

    #[test]
    fn builds_script_with_a_guard_code_and_a_cached_login() {
        let mut request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials {
                username: "alice".into(),
                password: None,
            },
            "/tmp/dl",
        );
        request.steam_guard_code = Some("ABCDE".into());
        assert!(build_script(&request).starts_with("set_steam_guard_code ABCDE\nlogin alice\n"));
    }

    #[cfg(unix)]
    #[test]
    fn passes_the_steam_guard_code_to_the_script() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("steam-press-steamcmd-guard-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("fake-steamcmd.sh");
        fs::write(
            &binary,
            "#!/bin/sh\nscript=\"$4\"\nif grep -q 'login alice pw ABCDE' \"$script\"; then\n  mkdir -p steamapps\n  echo \"Success! App '2379780' fully installed.\"\n  exit 0\nfi\necho 'Steam Guard required'\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();

        let mut request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials {
                username: "alice".into(),
                password: Some("pw".into()),
            },
            &dir,
        );
        request.steam_guard_code = Some("ABCDE".into());
        let mut downloader = SteamCmd::new(Some(binary));
        let report = downloader
            .download(&request, &EventSink::noop())
            .expect("the guard code should be accepted");
        assert!(report.content_dir.is_dir());
    }

    #[test]
    fn classifies_log_failures() {
        assert_eq!(
            classify_run(1, "", "", true, true),
            Err(DownloadError::Failed("steamcmd exited with code 1".into()))
        );
        assert_eq!(
            classify_run(0, "", "Rate Limit Exceeded", true, true),
            Err(DownloadError::RateLimited)
        );
        assert_eq!(
            classify_run(0, "No subscription", "", true, true),
            Err(DownloadError::NoSubscription)
        );
        assert_eq!(
            classify_run(0, "", "Invalid Password", true, true),
            Err(DownloadError::InvalidCredentials)
        );
        assert_eq!(
            classify_run(1, "Steam Guard required", "", true, true),
            Err(DownloadError::SteamGuardRequired)
        );
        match classify_run(1, "Failed to connect to Steam", "", true, true) {
            Err(DownloadError::Failed(message)) => assert!(message.contains("network error")),
            other => panic!("expected a network failure, got {other:?}"),
        }
        assert_eq!(
            classify_run(0, "Success! App '2379780' fully installed.", "", true, true),
            Ok(())
        );
    }
}
