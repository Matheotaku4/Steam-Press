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
use std::time::{Duration, SystemTime};

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
        reset_download_state(&request.work_dir);

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

        let started = SystemTime::now();
        let result = run_streaming(command, &self.cancel, sink);
        let _ = fs::remove_file(&script_path);
        let output = result?;

        let steamapps = request.work_dir.join("steamapps");
        let log_dir = log_directory(&output.output, &request.work_dir);
        let (content_log, connection_log) = read_logs(&log_dir, started);
        let content_log = format!("{}\n{content_log}", output.output);
        let has_login_logs =
            request.credentials.is_anonymous() || login_logs_present(&log_dir, started);
        if let Err(error) = classify_run(
            output.code,
            &content_log,
            &connection_log,
            steamapps.is_dir(),
            has_login_logs,
        ) {
            cleanup_failed_download(&request.work_dir);
            return Err(error);
        }

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
        reset_download_state(&request.work_dir);

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

        let started = SystemTime::now();
        let result = run_streaming(command, &self.cancel, sink);
        let _ = fs::remove_file(&script_path);
        let output = result?;
        let content = request
            .work_dir
            .join("steamapps/workshop/content")
            .join(&request.appid)
            .join(&request.workshop_id);
        let log_dir = log_directory(&output.output, &request.work_dir);
        let (content_log, connection_log) = read_logs(&log_dir, started);
        let content_log = format!("{}\n{content_log}", output.output);
        let has_login_logs =
            request.credentials.is_anonymous() || login_logs_present(&log_dir, started);
        if let Err(error) = classify_run(
            output.code,
            &content_log,
            &connection_log,
            content.is_dir(),
            has_login_logs,
        ) {
            cleanup_failed_download(&request.work_dir);
            return Err(error);
        }

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
///
/// `has_content` is true when the engine produced its expected output
/// directory. A Steam Guard line is only ignored when this run also logged
/// SteamCMD's success marker: a stale `steamapps` directory from a previous
/// attempt is not enough, because SteamCMD keeps
/// "Steam Guard must be approved before continuing" in the log of a successful
/// mobile confirmation. `has_login_logs` mirrors SuperSteamPacker: SteamCMD
/// writes `sitelicense_steamcmd.txt` or `compat_log.txt` only after a
/// successful login.
pub fn classify_run(
    exit_code: i32,
    content_log: &str,
    connection_log: &str,
    has_content: bool,
    has_login_logs: bool,
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
    let succeeded = lower.contains("success! app")
        || lower.contains("fully installed")
        || lower.contains("success. downloaded item");
    if lower.contains("steam guard") && (exit_code != 0 || !succeeded) {
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
        if !has_content && !has_login_logs {
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

fn reset_download_state(root: &Path) {
    for name in ["steamapps", "depotcache", "logs"] {
        let path = root.join(name);
        if path.is_dir() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// SteamCMD writes its logs to the directory it prints at startup. On Linux
/// that is `$HOME/Steam/logs`, not the working directory, so parsing the line
/// is more reliable than assuming `work_dir/logs`.
fn log_directory(output: &str, work_dir: &Path) -> PathBuf {
    const PREFIX: &str = "Logging directory:";
    for line in output.lines() {
        let Some(index) = line.find(PREFIX) else {
            continue;
        };
        let path = line[index + PREFIX.len()..]
            .trim()
            .trim_matches(|character| character == '\'' || character == '"');
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    work_dir.join("logs")
}

fn is_fresh(path: &Path, started: SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| modified + Duration::from_secs(5) >= started)
        .unwrap_or(false)
}

fn read_fresh(path: &Path, started: SystemTime) -> String {
    if is_fresh(path, started) {
        fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    }
}

fn read_logs(dir: &Path, started: SystemTime) -> (String, String) {
    (
        read_fresh(&dir.join("content_log.txt"), started),
        read_fresh(&dir.join("connection_log.txt"), started),
    )
}

/// SteamCMD creates these logs only after a successful login. Their absence
/// is the same guard signal SuperSteamPacker uses when the process exits
/// without an explicit "Steam Guard" line. The freshness check keeps a log
/// from an earlier attempt from counting as this run's login.
fn login_logs_present(dir: &Path, started: SystemTime) -> bool {
    ["sitelicense_steamcmd.txt", "compat_log.txt"]
        .iter()
        .any(|name| is_fresh(&dir.join(name), started))
}

/// A failed attempt must not leave content behind: a later run could treat
/// stale `steamapps` or `depotcache` as the result of the new attempt.
fn cleanup_failed_download(root: &Path) {
    for name in ["steamapps", "depotcache"] {
        let path = root.join(name);
        if path.is_dir() {
            let _ = fs::remove_dir_all(path);
        }
    }
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
            "#!/bin/sh\ndir=\"$2\"\nscript=\"$4\"\nif grep -q 'login alice pw ABCDE' \"$script\"; then\n  logdir=\"$dir/global-logs\"\n  mkdir -p \"$dir/steamapps\" \"$logdir\"\n  echo \"Logging directory: '$logdir'\"\n  echo \"Steam Guard must be approved before continuing\"\n  printf \"Success! App '2379780' fully installed.\\n\" > \"$logdir/content_log.txt\"\n  exit 0\nfi\necho 'Steam Guard required'\nexit 1\n",
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

    #[cfg(unix)]
    #[test]
    fn reads_steamcmd_logs_for_login_errors() {
        use std::os::unix::fs::PermissionsExt;

        let cases = [
            ("No subscription", "", DownloadError::NoSubscription),
            ("", "Rate Limit Exceeded", DownloadError::RateLimited),
            ("", "Invalid Password", DownloadError::InvalidCredentials),
        ];
        for (index, (content, connection, expected)) in cases.into_iter().enumerate() {
            let dir = std::env::temp_dir().join(format!(
                "steam-press-steamcmd-logs-{}-{index}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let binary = dir.join("fake-steamcmd.sh");
            fs::write(
                &binary,
                format!(
                    "#!/bin/sh\ndir=\"$2\"\nlogdir=\"$dir/global-logs\"\nmkdir -p \"$logdir\"\necho \"Logging directory: '$logdir'\"\nprintf '%s\\n' '{content}' > \"$logdir/content_log.txt\"\nprintf '%s\\n' '{connection}' > \"$logdir/connection_log.txt\"\nexit 0\n"
                ),
            )
            .unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();

            let request = DownloadRequest::for_job(
                GameJob::new(Platform::Linux64, "2379780"),
                Credentials {
                    username: "alice".into(),
                    password: Some("pw".into()),
                },
                &dir,
            );
            let mut downloader = SteamCmd::new(Some(binary));
            assert_eq!(
                downloader
                    .download(&request, &EventSink::noop())
                    .unwrap_err(),
                expected
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_download_removes_partial_content() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "steam-press-steamcmd-cleanup-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("fake-steamcmd.sh");
        fs::write(
            &binary,
            "#!/bin/sh\ndir=\"$2\"\nmkdir -p \"$dir/steamapps/partial\" \"$dir/logs\"\nprintf 'Steam Guard required\\n' > \"$dir/logs/content_log.txt\"\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();

        let request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials {
                username: "alice".into(),
                password: Some("pw".into()),
            },
            &dir,
        );
        let mut downloader = SteamCmd::new(Some(binary));
        assert_eq!(
            downloader
                .download(&request, &EventSink::noop())
                .unwrap_err(),
            DownloadError::SteamGuardRequired
        );
        assert!(!dir.join("steamapps").exists());
    }

    #[test]
    fn finds_the_steamcmd_log_directory() {
        assert_eq!(
            log_directory(
                "Logging directory: '/home/alice/Steam/logs'\n",
                Path::new("/work")
            ),
            PathBuf::from("/home/alice/Steam/logs")
        );
        assert_eq!(
            log_directory("no log line", Path::new("/work")),
            PathBuf::from("/work/logs")
        );
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
        assert_eq!(
            classify_run(
                0,
                "Steam Guard must be approved before continuing\nSuccess! App '2379780' fully installed.",
                "",
                true,
                true
            ),
            Ok(())
        );
        assert_eq!(
            classify_run(
                0,
                "Steam Guard must be approved before continuing",
                "",
                false,
                true
            ),
            Err(DownloadError::SteamGuardRequired)
        );
        assert_eq!(
            classify_run(
                0,
                "Steam Guard must be approved before continuing",
                "",
                true,
                true
            ),
            Err(DownloadError::SteamGuardRequired)
        );
        assert_eq!(
            classify_run(
                0,
                "Steam Guard must be approved before continuing\nSuccess. Downloaded item 999 to /tmp/dl",
                "",
                true,
                true
            ),
            Ok(())
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
