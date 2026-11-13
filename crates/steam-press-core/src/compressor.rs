//! Archive backends supported by Steam Press.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compressor {
    #[default]
    #[serde(rename = "7z")]
    SevenZip,
    #[serde(rename = "rar")]
    Rar,
}

impl Compressor {
    /// Reads the SuperSteamPacker `compressor` setting.
    pub fn from_setting(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "7z" | "7-zip" | "sevenzip" => Some(Self::SevenZip),
            "rar" => Some(Self::Rar),
            _ => None,
        }
    }

    /// Value written back to `settings.ini`.
    pub fn setting(&self) -> &'static str {
        match self {
            Self::SevenZip => "7z",
            Self::Rar => "RAR",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::SevenZip => "7z",
            Self::Rar => "rar",
        }
    }

    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "7z" | "001" => Some(Self::SevenZip),
            "rar" => Some(Self::Rar),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::SevenZip => "7-Zip",
            Self::Rar => "RAR",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_supersteampacker_values() {
        assert_eq!(Compressor::from_setting("7z"), Some(Compressor::SevenZip));
        assert_eq!(Compressor::from_setting("RAR"), Some(Compressor::Rar));
        assert_eq!(Compressor::from_setting("zip"), None);
        assert_eq!(Compressor::SevenZip.setting(), "7z");
        assert_eq!(Compressor::Rar.extension(), "rar");
    }
}
