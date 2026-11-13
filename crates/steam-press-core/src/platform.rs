//! Steam platform targets and their SteamCMD representation.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    #[serde(rename = "win64")]
    Win64,
    #[serde(rename = "win32")]
    Win32,
    #[serde(rename = "lin64")]
    Linux64,
    #[serde(rename = "lin32")]
    Linux32,
    #[serde(rename = "macos")]
    Mac,
}

impl Default for Platform {
    fn default() -> Self {
        Self::Linux64
    }
}

impl Platform {
    /// SuperSteamPacker queue key.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Win64 => "win64",
            Self::Win32 => "win32",
            Self::Linux64 => "lin64",
            Self::Linux32 => "lin32",
            Self::Mac => "macos",
        }
    }

    /// Label shown in the CLI and the desktop app.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Win64 => "Windows x64",
            Self::Win32 => "Windows x86",
            Self::Linux64 => "Linux x64",
            Self::Linux32 => "Linux x86",
            Self::Mac => "Mac",
        }
    }

    /// Tag used in output names, matching SuperSteamPacker.
    pub fn os_tag(&self) -> &'static str {
        match self {
            Self::Win64 => "Win64",
            Self::Win32 => "Win32",
            Self::Linux64 => "Linux64",
            Self::Linux32 => "Linux32",
            Self::Mac => "Mac",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "win64" | "windows x64" | "windows64" => Some(Self::Win64),
            "win32" | "windows x86" | "windows32" => Some(Self::Win32),
            "lin64" | "linux64" | "linux x64" => Some(Self::Linux64),
            "lin32" | "linux32" | "linux x86" => Some(Self::Linux32),
            "macos" | "mac" | "osx" => Some(Self::Mac),
            _ => None,
        }
    }

    pub fn from_label(value: &str) -> Option<Self> {
        Self::from_key(value)
    }

    /// SteamCMD `@sSteamCmdForcePlatformType` value.
    pub fn steamcmd_platform(&self) -> &'static str {
        match self {
            Self::Win64 | Self::Win32 => "windows",
            Self::Linux64 | Self::Linux32 => "linux",
            Self::Mac => "macos",
        }
    }

    pub fn bitness(&self) -> Option<u8> {
        match self {
            Self::Win64 | Self::Linux64 => Some(64),
            Self::Win32 | Self::Linux32 => Some(32),
            Self::Mac => None,
        }
    }

    /// DepotDownloader `-os` value.
    pub fn depotdownloader_os(&self) -> &'static str {
        match self {
            Self::Win64 | Self::Win32 => "windows",
            Self::Linux64 | Self::Linux32 => "linux",
            Self::Mac => "macos",
        }
    }

    pub fn steamcmd_directives(&self) -> Vec<String> {
        let mut directives = vec![format!(
            "@sSteamCmdForcePlatformType {}",
            self.steamcmd_platform()
        )];
        if let Some(bitness) = self.bitness() {
            directives.push(format!("@sSteamCmdForcePlatformBitness {bitness}"));
        }
        directives
    }

    pub fn all() -> &'static [Platform] {
        &[
            Self::Win64,
            Self::Win32,
            Self::Linux64,
            Self::Linux32,
            Self::Mac,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_platform_keys_labels_and_tags() {
        assert_eq!(Platform::from_key("win64"), Some(Platform::Win64));
        assert_eq!(Platform::from_label("Linux x64"), Some(Platform::Linux64));
        assert_eq!(Platform::Linux64.os_tag(), "Linux64");
        assert_eq!(Platform::Mac.key(), "macos");
        assert_eq!(Platform::from_key("nope"), None);
    }

    #[test]
    fn emits_steamcmd_directives() {
        assert_eq!(
            Platform::Win64.steamcmd_directives(),
            vec![
                "@sSteamCmdForcePlatformType windows",
                "@sSteamCmdForcePlatformBitness 64"
            ]
        );
        assert_eq!(
            Platform::Mac.steamcmd_directives(),
            vec!["@sSteamCmdForcePlatformType macos"]
        );
    }

    #[test]
    fn maps_depotdownloader_os() {
        assert_eq!(Platform::Win32.depotdownloader_os(), "windows");
        assert_eq!(Platform::Linux32.depotdownloader_os(), "linux");
        assert_eq!(Platform::Mac.depotdownloader_os(), "macos");
    }
}
