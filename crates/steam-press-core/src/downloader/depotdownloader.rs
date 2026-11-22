//! DepotDownloader engine: deterministic depot selection through the sidecar.

use super::process::{run_streaming_with_input, CommandOutput};
use super::{
    DownloadError, DownloadReport, DownloadRequest, Downloader, DownloaderKind, ToolStatus,
    WorkshopReport, WorkshopRequest,
};
use crate::config::Settings;
use crate::events::{CancellationToken, EventSink, PackEvent};
use crate::{naming, tooling};
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
        let input = stdin_input(&request.credentials, request.steam_guard_code.as_deref())?;
        let output = run_streaming_with_input(command, &self.cancel, sink, input)?;

        if output.code != 0 {
            return Err(classify_failure(output));
        }

        let content_dir = stage_download(request)?;
        let manifest = content_dir.join(format!("appmanifest_{}.acf", request.appid));
        Ok(DownloadReport {
            content_dir,
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
            if has_steam_guard_code(request.steam_guard_code.as_deref()) {
                command.arg("-no-mobile");
            }
        }

        let input = stdin_input(&request.credentials, request.steam_guard_code.as_deref())?;
        let output = run_streaming_with_input(command, &self.cancel, sink, input)?;
        if output.code != 0 {
            return Err(classify_failure(output));
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

/// DepotDownloader writes the files at the root of `-dir`, unlike SteamCMD's
/// `steamapps` layout. Move them into `steamapps/common/<installdir>` and drop
/// the sidecar metadata so the normalizer can produce a Steam library.
fn stage_download(request: &DownloadRequest) -> Result<PathBuf, DownloadError> {
    let Some(raw_install_dir) = request.install_dir.as_deref() else {
        return Ok(request.work_dir.clone());
    };
    let install_dir = naming::sanitize_install_dir(raw_install_dir);
    if install_dir.is_empty() {
        return Ok(request.work_dir.clone());
    }

    let steamapps = request.work_dir.join("steamapps");
    let common = steamapps.join("common").join(&install_dir);
    fs::create_dir_all(&common)
        .map_err(|error| DownloadError::Failed(format!("content directory: {error}")))?;

    let entries = fs::read_dir(&request.work_dir)
        .map_err(|error| DownloadError::Failed(format!("content directory: {error}")))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| DownloadError::Failed(format!("content directory: {error}")))?;
        let name = entry.file_name();
        if name == "steamapps" || name == ".DepotDownloader" {
            continue;
        }
        let destination = common.join(&name);
        if destination.exists() {
            if destination.is_dir() {
                fs::remove_dir_all(&destination).map_err(|error| {
                    DownloadError::Failed(format!("staging downloaded content: {error}"))
                })?;
            } else {
                fs::remove_file(&destination).map_err(|error| {
                    DownloadError::Failed(format!("staging downloaded content: {error}"))
                })?;
            }
        }
        fs::rename(entry.path(), &destination).map_err(|error| {
            DownloadError::Failed(format!("staging downloaded content: {error}"))
        })?;
    }

    let metadata = request.work_dir.join(".DepotDownloader");
    if metadata.is_dir() {
        fs::remove_dir_all(&metadata).map_err(|error| {
            DownloadError::Failed(format!("removing DepotDownloader metadata: {error}"))
        })?;
    }
    Ok(steamapps)
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
        if has_steam_guard_code(request.steam_guard_code.as_deref()) {
            args.push("-no-mobile".to_string());
        }
    }

    args
}

fn has_steam_guard_code(code: Option<&str>) -> bool {
    code.map(str::trim).is_some_and(|code| !code.is_empty())
}

fn classify_failure(output: CommandOutput) -> DownloadError {
    let text = output.output.to_ascii_lowercase();
    if text.contains("steam guard")
        || text.contains("2-factor auth code")
        || text.contains("2 factor auth code")
    {
        DownloadError::SteamGuardRequired
    } else {
        DownloadError::Failed(format!("DepotDownloader exited with code {}", output.code))
    }
}

/// DepotDownloader prompts for the password and then for the Steam Guard code
/// on stdin. Writing both lines up front keeps the password out of the process
/// arguments while still answering the prompts in order.
fn stdin_input(
    credentials: &super::Credentials,
    steam_guard_code: Option<&str>,
) -> Result<Option<String>, DownloadError> {
    if credentials.is_anonymous() {
        return Ok(None);
    }
    let Some(password) = credentials
        .password
        .as_deref()
        .filter(|password| !password.is_empty())
    else {
        return Err(DownloadError::Failed(
            "DepotDownloader needs the account password; run `steam-press login` again and enter it"
                .to_string(),
        ));
    };
    let mut input = format!("{password}\n");
    if let Some(code) = steam_guard_code
        .map(str::trim)
        .filter(|code| !code.is_empty())
    {
        input.push_str(code);
        input.push('\n');
    }
    Ok(Some(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::{Credentials, DepotSelector};
    use crate::platform::Platform;
    use crate::queue::GameJob;
    use std::path::Path;

    #[cfg(unix)]
    #[test]
    fn downloads_through_a_fake_sidecar() {
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::test_support::temp_dir("depotdownloader-fake");
        let script = dir.join("fake-DepotDownloader.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nset -eu\ndir=\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    -dir) dir=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nmkdir -p \"$dir/game\" \"$dir/.DepotDownloader\"\nprintf 'downloaded' > \"$dir/game/file.bin\"\nprintf 'metadata' > \"$dir/.DepotDownloader/state\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let work_dir = dir.join("work");
        let mut request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials::anonymous(),
            &work_dir,
        );
        request.install_dir = Some("Spacewar".to_string());
        let mut downloader = DepotDownloader::new(Some(script));
        let report = downloader
            .download(&request, &EventSink::noop())
            .expect("fake download succeeds");
        assert_eq!(report.content_dir, work_dir.join("steamapps"));
        assert!(work_dir
            .join("steamapps/common/Spacewar/game/file.bin")
            .is_file());
        assert!(!work_dir.join(".DepotDownloader").exists());
    }

    #[cfg(unix)]
    #[test]
    fn reports_a_fake_sidecar_failure() {
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::test_support::temp_dir("depotdownloader-failure");
        let script = dir.join("fake-DepotDownloader.sh");
        std::fs::write(&script, "#!/bin/sh\necho 'Invalid Password'\nexit 3\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials::anonymous(),
            dir.join("work"),
        );
        request.install_dir = Some("Spacewar".to_string());
        let mut downloader = DepotDownloader::new(Some(script));
        match downloader.download(&request, &EventSink::noop()) {
            Err(DownloadError::Failed(message)) => assert!(message.contains("code 3"), "{message}"),
            other => panic!("expected a sidecar failure, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn sends_the_password_and_guard_code_on_stdin() {
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::test_support::temp_dir("depotdownloader-guard");
        let script = dir.join("fake-DepotDownloader.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nset -eu\ndir=\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    -dir) dir=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nIFS= read -r password\nIFS= read -r code\n[ \"$password\" = 'secret' ]\n[ \"$code\" = 'ABCDE' ]\nmkdir -p \"$dir/game\"\nprintf 'downloaded' > \"$dir/game/file.bin\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let work_dir = dir.join("work");
        let mut request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials {
                username: "alice".into(),
                password: Some("secret".into()),
            },
            &work_dir,
        );
        request.install_dir = Some("Spacewar".to_string());
        request.steam_guard_code = Some("ABCDE".to_string());
        let mut downloader = DepotDownloader::new(Some(script));
        let report = downloader
            .download(&request, &EventSink::noop())
            .expect("the password and guard code should reach stdin");
        assert!(report
            .content_dir
            .join("common/Spacewar/game/file.bin")
            .is_file());
    }

    #[test]
    fn stdin_input_orders_the_password_before_the_code() {
        let credentials = Credentials {
            username: "alice".into(),
            password: Some("secret".into()),
        };
        assert_eq!(
            stdin_input(&credentials, Some(" ABCDE ")).unwrap(),
            Some("secret\nABCDE\n".to_string())
        );
        assert_eq!(
            stdin_input(&credentials, None).unwrap(),
            Some("secret\n".to_string())
        );
        assert_eq!(
            stdin_input(
                &Credentials {
                    username: "alice".into(),
                    password: Some(" secret ".into()),
                },
                None
            )
            .unwrap(),
            Some(" secret \n".to_string())
        );
        assert_eq!(
            stdin_input(&Credentials::anonymous(), Some("ABCDE")).unwrap(),
            None
        );
    }

    #[test]
    fn build_args_adds_no_mobile_only_with_a_code() {
        let mut request = DownloadRequest::for_job(
            GameJob::new(Platform::Linux64, "2379780"),
            Credentials {
                username: "alice".into(),
                password: Some("secret".into()),
            },
            Path::new("/tmp/dl"),
        );
        assert!(!build_args(&request).contains(&"-no-mobile".to_string()));
        request.steam_guard_code = Some("ABCDE".into());
        assert!(build_args(&request).contains(&"-no-mobile".to_string()));
        assert!(!build_args(&request).join(" ").contains("ABCDE"));
    }

    #[test]
    fn classifies_steam_guard_prompts_as_guard_required() {
        use crate::downloader::process::CommandOutput;

        let output = CommandOutput {
            code: 1,
            output: "STEAM GUARD! Please enter your 2-factor auth code".to_string(),
        };
        assert_eq!(classify_failure(output), DownloadError::SteamGuardRequired);
    }

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
