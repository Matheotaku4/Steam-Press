//! Application directories and the SuperSteamPacker-compatible `settings.ini`.

use crate::compressor::Compressor;
use crate::credentials::generate_key;
use crate::ini::Ini;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub work_dir: PathBuf,
    pub output_dir: PathBuf,
}

impl Paths {
    /// `STEAM_PRESS_HOME`, then XDG directories, then `~/SteamPress`.
    pub fn discover() -> Self {
        if let Some(root) = std::env::var_os("STEAM_PRESS_HOME") {
            return Self::under(root);
        }

        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let config_dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("steam-press");
        let data_dir = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("steam-press");
        let work_dir = home.join("SteamPress");

        Self {
            config_dir,
            data_dir,
            work_dir: work_dir.clone(),
            output_dir: work_dir.join("Completed"),
        }
    }

    /// Hermetic layout for tests and `STEAM_PRESS_HOME`.
    pub fn under(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            work_dir: root.join("work"),
            output_dir: root.join("outputs"),
        }
    }

    pub fn ensure(&self) -> io::Result<()> {
        for directory in [
            &self.config_dir,
            &self.data_dir,
            &self.work_dir,
            &self.output_dir,
        ] {
            fs::create_dir_all(directory)?;
        }
        Ok(())
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.ini")
    }

    pub fn credentials_file(&self) -> PathBuf {
        self.config_dir.join("credentials.bin")
    }

    pub fn depots_file(&self) -> PathBuf {
        self.config_dir.join("depots.ini")
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.config_dir.join("templates")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub compressor: Compressor,
    pub custom_compress_options: Option<String>,
    pub skip_compression: bool,
    pub dark_mode: bool,
    pub upload_crew_mode: bool,
    pub uploader: String,
    pub filehost: String,
    pub output_dir: Option<PathBuf>,
    pub steamcmd_path: Option<PathBuf>,
    pub depotdownloader_path: Option<PathBuf>,
    pub sevenzip_path: Option<PathBuf>,
    pub rar_path: Option<PathBuf>,
    pub compress_password: String,
    pub template: Option<PathBuf>,
    pub key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            compressor: Compressor::SevenZip,
            custom_compress_options: None,
            skip_compression: false,
            dark_mode: true,
            upload_crew_mode: false,
            uploader: String::new(),
            filehost: String::new(),
            output_dir: None,
            steamcmd_path: None,
            depotdownloader_path: None,
            sevenzip_path: None,
            rar_path: None,
            compress_password: "cs.rin.ru".to_string(),
            template: None,
            key: String::new(),
        }
    }
}

fn text(ini: &Ini, key: &str) -> String {
    ini.get("SSP", key).unwrap_or("").trim().to_string()
}

fn optional_path(ini: &Ini, key: &str) -> Option<PathBuf> {
    let value = text(ini, key);
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn flag(ini: &Ini, key: &str) -> bool {
    text(ini, key) == "1"
}

impl Settings {
    pub fn load(paths: &Paths) -> Self {
        let ini = Ini::load(paths.settings_file()).unwrap_or_default();
        let mut settings = Self::default();

        if let Some(compressor) = Compressor::from_setting(&text(&ini, "compressor")) {
            settings.compressor = compressor;
        }
        let custom = text(&ini, "customcompressoption");
        if !custom.is_empty() {
            settings.custom_compress_options = Some(custom);
        }
        settings.skip_compression = flag(&ini, "skipcompression");
        if ini.get("SSP", "darkmode").is_some() {
            settings.dark_mode = flag(&ini, "darkmode");
        }
        settings.upload_crew_mode = flag(&ini, "uploadcrewmode");
        settings.uploader = text(&ini, "rinruusername");
        settings.filehost = text(&ini, "filehost");
        settings.output_dir = optional_path(&ini, "outputdir");
        settings.steamcmd_path = optional_path(&ini, "steamcmdpath");
        settings.depotdownloader_path = optional_path(&ini, "depotdownloaderpath");
        settings.sevenzip_path = optional_path(&ini, "sevenzippath");
        settings.rar_path = optional_path(&ini, "rarpath");
        settings.template = optional_path(&ini, "template");
        settings.key = text(&ini, "key");
        let password = text(&ini, "compresspassword");
        if !password.is_empty() {
            settings.compress_password = password;
        }

        settings
    }

    pub fn save(&self, paths: &Paths) -> io::Result<()> {
        paths.ensure()?;
        let mut ini = Ini::default();

        ini.set("SSP", "compressor", self.compressor.setting());
        if let Some(options) = &self.custom_compress_options {
            ini.set("SSP", "customcompressoption", options.trim());
        }
        ini.set(
            "SSP",
            "skipcompression",
            if self.skip_compression { "1" } else { "0" },
        );
        ini.set("SSP", "darkmode", if self.dark_mode { "1" } else { "0" });
        ini.set(
            "SSP",
            "uploadcrewmode",
            if self.upload_crew_mode { "1" } else { "0" },
        );
        ini.set("SSP", "rinruusername", self.uploader.trim());
        ini.set("SSP", "filehost", self.filehost.trim());
        ini.set("SSP", "compresspassword", self.archive_password());
        ini.set("SSP", "key", &self.key);

        for (key, value) in [
            ("outputdir", &self.output_dir),
            ("steamcmdpath", &self.steamcmd_path),
            ("depotdownloaderpath", &self.depotdownloader_path),
            ("sevenzippath", &self.sevenzip_path),
            ("rarpath", &self.rar_path),
            ("template", &self.template),
        ] {
            match value {
                Some(path) if !path.as_os_str().is_empty() => {
                    ini.set("SSP", key, &path.to_string_lossy());
                }
                _ => {
                    ini.remove("SSP", key);
                }
            }
        }

        ini.save(paths.settings_file())
    }

    pub fn ensure_key(&mut self) {
        if self.key.trim().is_empty() {
            self.key = generate_key();
        }
    }

    /// Archive password, falling back to the community default.
    pub fn archive_password(&self) -> &str {
        if self.compress_password.trim().is_empty() {
            "cs.rin.ru"
        } else {
            self.compress_password.as_str()
        }
    }

    pub fn output_dir(&self, paths: &Paths) -> PathBuf {
        self.output_dir
            .clone()
            .unwrap_or_else(|| paths.output_dir.clone())
    }

    pub fn work_dir<'a>(&self, paths: &'a Paths) -> &'a Path {
        &paths.work_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_defaults() {
        let dir = crate::test_support::temp_dir("settings");
        let paths = Paths::under(&dir);
        let mut settings = Settings {
            compressor: Compressor::Rar,
            skip_compression: true,
            uploader: "alice".into(),
            filehost: "filehost.example".into(),
            custom_compress_options: Some("-mx5".into()),
            ..Default::default()
        };
        settings.ensure_key();
        settings.save(&paths).unwrap();

        let loaded = Settings::load(&paths);
        assert_eq!(loaded.compressor, Compressor::Rar);
        assert!(loaded.skip_compression);
        assert_eq!(loaded.uploader, "alice");
        assert_eq!(loaded.filehost, "filehost.example");
        assert_eq!(loaded.custom_compress_options.as_deref(), Some("-mx5"));
        assert_eq!(loaded.compress_password, "cs.rin.ru");
        assert!(!loaded.key.is_empty());
        assert_eq!(loaded, settings);
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = crate::test_support::temp_dir("settings-default");
        let paths = Paths::under(&dir);
        let loaded = Settings::load(&paths);
        assert_eq!(loaded.compressor, Compressor::SevenZip);
        assert!(loaded.dark_mode);
        assert_eq!(loaded.archive_password(), "cs.rin.ru");
        assert!(loaded.output_dir(&paths).ends_with("outputs"));
    }

    #[test]
    fn blank_password_uses_community_default() {
        let settings = Settings {
            compress_password: "   ".into(),
            ..Default::default()
        };
        assert_eq!(settings.archive_password(), "cs.rin.ru");
    }
}
