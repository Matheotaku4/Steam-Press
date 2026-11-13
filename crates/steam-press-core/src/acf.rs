//! Steam app manifests (`.acf`): reading, privacy sanitizing and generation.

use crate::vdf::{self, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledDepot {
    pub depot_id: String,
    pub manifest: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifest {
    pub appid: Option<String>,
    pub name: String,
    pub build_id: String,
    pub installdir: Option<String>,
    pub installed_depots: Vec<InstalledDepot>,
}

pub fn read_manifest(path: &Path) -> Result<AppManifest, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let root = vdf::parse(&text)?;
    let state = root
        .get("AppState")
        .ok_or_else(|| "manifeste sans section AppState".to_string())?;

    let installed_depots = match state.get("InstalledDepots") {
        Some(Value::Object(entries)) => entries
            .iter()
            .filter_map(|(depot_id, depot)| {
                let depot = match depot {
                    Value::Object(_) => depot,
                    Value::Str(_) => return None,
                };
                Some(InstalledDepot {
                    depot_id: depot_id.clone(),
                    manifest: depot.get_str("manifest").unwrap_or_default().to_string(),
                    size: depot
                        .get_str("size")
                        .and_then(|size| size.parse::<u64>().ok()),
                })
            })
            .collect(),
        _ => Vec::new(),
    };

    Ok(AppManifest {
        appid: state.get_str("appid").map(str::to_string),
        name: state.get_str("name").unwrap_or_default().to_string(),
        build_id: state.get_str("buildid").unwrap_or_default().to_string(),
        installdir: state.get_str("installdir").map(str::to_string),
        installed_depots,
    })
}

/// Sets `LastOwner` and `LauncherPath` to `"0"`. Returns whether the file changed.
pub fn rewrite_privacy_fields(path: &Path) -> Result<bool, String> {
    let original = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut root = vdf::parse(&original)?;
    let state = root
        .get_mut("AppState")
        .ok_or_else(|| "manifeste sans section AppState".to_string())?;

    let mut changed = false;
    for key in ["LastOwner", "LauncherPath"] {
        let already_zero = state.get(key).and_then(Value::as_str) == Some("0");
        if !already_zero {
            state.set(key, Value::Str("0".to_string()));
            changed = true;
        }
    }

    if !changed {
        return Ok(false);
    }
    fs::write(path, root.to_vdf_string()).map_err(|error| error.to_string())?;
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcfDepot {
    pub depot_id: String,
    pub manifest_id: String,
    pub size: u64,
    pub dlc_appid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDepot {
    pub depot_id: String,
    pub owner_appid: String,
    pub manifest_id: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcfJob {
    pub appid: String,
    pub name: String,
    pub install_dir: String,
    pub build_id: String,
    pub size_on_disk: u64,
    pub depots: Vec<AcfDepot>,
    pub shared_depots: Vec<SharedDepot>,
    pub install_scripts: Vec<(String, String)>,
}

fn text(key: &str, value: impl Into<String>) -> (String, Value) {
    (key.to_string(), Value::Str(value.into()))
}

fn numeric_build_id(build_id: &str) -> &str {
    if !build_id.is_empty() && build_id.chars().all(|character| character.is_ascii_digit()) {
        build_id
    } else {
        "0"
    }
}

fn app_state(job: &AcfJob, appid: &str, name: &str, install_dir: &str) -> Value {
    let mut installed = Vec::new();
    for depot in &job.depots {
        let mut entries = vec![
            text("manifest", depot.manifest_id.clone()),
            text("size", depot.size.to_string()),
        ];
        if let Some(dlc_appid) = &depot.dlc_appid {
            entries.push(text("dlcappid", dlc_appid.clone()));
        }
        installed.push((depot.depot_id.clone(), Value::Object(entries)));
    }

    let mut entries = vec![
        text("appid", appid),
        text("universe", "1"),
        text("name", name),
        text("StateFlags", "4"),
        text("installdir", install_dir),
        text("LastUpdated", "0"),
        text("LastPlayed", "0"),
        text("SizeOnDisk", job.size_on_disk.to_string()),
        text("StagingSize", "0"),
        text("buildid", numeric_build_id(&job.build_id)),
        text("LastOwner", "0"),
        text("UpdateResult", "0"),
        text("BytesToDownload", "0"),
        text("BytesDownloaded", "0"),
        text("AutoUpdateBehavior", "0"),
        text("AllowOtherDownloadsWhileRunning", "0"),
        text("ScheduledAutoUpdate", "0"),
        ("InstalledDepots".to_string(), Value::Object(installed)),
    ];

    if !job.shared_depots.is_empty() {
        entries.push((
            "SharedDepots".to_string(),
            Value::Object(
                job.shared_depots
                    .iter()
                    .map(|depot| text(&depot.depot_id, depot.owner_appid.clone()))
                    .collect(),
            ),
        ));
    }

    entries.push(("UserConfig".to_string(), Value::Object(Vec::new())));
    entries.push(("MountedConfig".to_string(), Value::Object(Vec::new())));

    Value::Object(entries)
}

pub fn generate_acf(job: &AcfJob) -> String {
    let state = app_state(job, &job.appid, &job.name, &job.install_dir);
    Value::Object(vec![("AppState".to_string(), state)]).to_vdf_string()
}

pub fn write_acf(directory: &Path, job: &AcfJob) -> Result<PathBuf, String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let path = directory.join(format!("appmanifest_{}.acf", job.appid));
    fs::write(&path, generate_acf(job)).map_err(|error| error.to_string())?;
    Ok(path)
}

/// Writes `appmanifest_228980.acf` for Steamworks redistributables.
pub fn write_shared_acf(directory: &Path, job: &AcfJob) -> Result<Option<PathBuf>, String> {
    if job.shared_depots.is_empty() {
        return Ok(None);
    }

    let mut installed = Vec::new();
    for depot in &job.shared_depots {
        installed.push((
            depot.depot_id.clone(),
            Value::Object(vec![
                text("manifest", depot.manifest_id.clone()),
                text("size", depot.size.to_string()),
            ]),
        ));
    }

    let mut entries = vec![
        text("appid", "228980"),
        text("universe", "1"),
        text("name", "Steamworks Common Redistributables"),
        text("StateFlags", "4"),
        text("installdir", "Steamworks Shared"),
        text("LastUpdated", "0"),
        text("LastPlayed", "0"),
        text("SizeOnDisk", "0"),
        text("StagingSize", "0"),
        text("buildid", "0"),
        text("LastOwner", "0"),
        text("DownloadType", "0"),
        text("AutoUpdateBehavior", "0"),
        text("AllowOtherDownloadsWhileRunning", "0"),
        text("ScheduledAutoUpdate", "0"),
        ("InstalledDepots".to_string(), Value::Object(installed)),
    ];

    if !job.install_scripts.is_empty() {
        let mut scripts = job.install_scripts.clone();
        scripts.sort_by(|left, right| left.0.cmp(&right.0));
        entries.push((
            "InstallScripts".to_string(),
            Value::Object(
                scripts
                    .into_iter()
                    .map(|(depot_id, script)| text(&depot_id, script.replace('/', "\\")))
                    .collect(),
            ),
        ));
    }

    entries.push(("UserConfig".to_string(), Value::Object(Vec::new())));
    entries.push(("MountedConfig".to_string(), Value::Object(Vec::new())));

    let root = Value::Object(vec![("AppState".to_string(), Value::Object(entries))]);
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let path = directory.join("appmanifest_228980.acf");
    fs::write(&path, root.to_vdf_string()).map_err(|error| error.to_string())?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_manifest_fields_and_depots() {
        let manifest = read_manifest(Path::new("tests/fixtures/appmanifest_2379780.acf")).unwrap();
        assert_eq!(manifest.name, "Balatro");
        assert_eq!(manifest.build_id, "18674832");
        assert_eq!(manifest.installed_depots[0].manifest, "4851806656204679952");
        assert_eq!(manifest.installed_depots[0].size, Some(1024));
    }

    #[test]
    fn rewrites_privacy_fields_idempotently() {
        let dir = crate::test_support::temp_dir("acf");
        let path = dir.join("appmanifest_2379780.acf");
        fs::write(
            &path,
            include_str!("../tests/fixtures/appmanifest_2379780.acf"),
        )
        .unwrap();
        assert!(rewrite_privacy_fields(&path).unwrap());
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"LastOwner\"\t\t\"0\""));
        assert!(text.contains("\"LauncherPath\"\t\t\"0\""));
        assert!(!rewrite_privacy_fields(&path).unwrap());
    }

    #[test]
    fn generates_clean_acf() {
        let job = AcfJob {
            appid: "2379780".into(),
            name: "Balatro".into(),
            install_dir: "Balatro".into(),
            build_id: "18674832".into(),
            size_on_disk: 1024,
            depots: vec![AcfDepot {
                depot_id: "2379781".into(),
                manifest_id: "4851806656204679952".into(),
                size: 1024,
                dlc_appid: None,
            }],
            shared_depots: vec![],
            install_scripts: vec![],
        };
        let text = generate_acf(&job);
        assert!(text.contains("\"LastOwner\"\t\t\"0\""));
        assert!(text.contains("\"installdir\"\t\t\"Balatro\""));
        assert!(text.contains("\"buildid\"\t\t\"18674832\""));
        assert!(text.contains("\"2379781\""));
        assert!(!text.contains("\"MountedDepots\""));
    }
}
