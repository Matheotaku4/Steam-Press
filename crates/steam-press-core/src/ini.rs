//! Minimal INI reader and writer.
//!
//! The format is the one SuperSteamPacker uses: `[section]` headers, flat
//! `key=value` entries, `;`/`#` comments and no escaping.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    pub fn parse(input: &str) -> Self {
        let mut ini = Ini::default();
        let mut current: Option<usize> = None;
        let input = input.strip_prefix('\u{feff}').unwrap_or(input);

        for raw in input.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }

            if let Some(rest) = line.strip_prefix('[') {
                if let Some(name) = rest.strip_suffix(']') {
                    let name = name.trim().to_string();
                    current = Some(
                        match ini
                            .sections
                            .iter()
                            .position(|(n, _)| n.eq_ignore_ascii_case(&name))
                        {
                            Some(index) => index,
                            None => {
                                ini.sections.push((name, Vec::new()));
                                ini.sections.len() - 1
                            }
                        },
                    );
                    continue;
                }
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            let value = value.trim();

            if let Some(index) = current {
                let entries = &mut ini.sections[index].1;
                match entries
                    .iter_mut()
                    .find(|(existing, _)| existing.eq_ignore_ascii_case(key))
                {
                    Some(entry) => entry.1 = value.to_string(),
                    None => entries.push((key.to_string(), value.to_string())),
                }
            }
        }

        ini
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self::parse(&fs::read_to_string(path)?))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, self.to_string())
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        let (_, entries) = self
            .sections
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(section))?;
        entries
            .iter()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let index = match self
            .sections
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(section))
        {
            Some(index) => index,
            None => {
                self.sections.push((section.to_string(), Vec::new()));
                self.sections.len() - 1
            }
        };
        let entries = &mut self.sections[index].1;
        match entries
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(key))
        {
            Some(entry) => entry.1 = value.to_string(),
            None => entries.push((key.to_string(), value.to_string())),
        }
    }

    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        let Some((_, entries)) = self
            .sections
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(section))
        else {
            return false;
        };
        let before = entries.len();
        entries.retain(|(existing, _)| !existing.eq_ignore_ascii_case(key));
        entries.len() != before
    }

    pub fn section_names(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|(name, _)| name.as_str())
    }

    pub fn entries(&self, section: &str) -> Option<&[(String, String)]> {
        self.sections
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(section))
            .map(|(_, entries)| entries.as_slice())
    }
}

impl fmt::Display for Ini {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (name, entries)) in self.sections.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            writeln!(f, "[{name}]")?;
            for (key, value) in entries {
                writeln!(f, "{key}={value}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bom_comments_crlf_and_duplicate_keys() {
        let doc = Ini::parse(
            "\u{feff}; comment\r\n[SSP]\r\ncompressor=7z\r\ncompressor=RAR\r\n\r\n[depots]\r\n10 = Counter-Strike\r\n",
        );
        assert_eq!(doc.get("SSP", "compressor"), Some("RAR"));
        assert_eq!(doc.get("depots", "10"), Some("Counter-Strike"));
        assert!(doc.get("SSP", "missing").is_none());
    }

    #[test]
    fn round_trips_sections_and_values() {
        let mut doc = Ini::parse("[SSP]\nlanguage=English\n");
        doc.set("SSP", "compressor", "RAR");
        let reparsed = Ini::parse(&doc.to_string());
        assert_eq!(reparsed.get("SSP", "language"), Some("English"));
        assert_eq!(reparsed.get("SSP", "compressor"), Some("RAR"));
    }

    #[test]
    fn keys_and_sections_are_case_insensitive() {
        let mut doc = Ini::parse("[ssp]\nCOMPRESSOR=7z\n");
        assert_eq!(doc.get("SSP", "compressor"), Some("7z"));
        doc.set("Ssp", "Compressor", "RAR");
        assert_eq!(doc.get("ssp", "compressor"), Some("RAR"));
    }

    #[test]
    fn saves_and_loads_a_file() {
        let dir = crate::test_support::temp_dir("ini");
        let path = dir.join("settings.ini");
        let mut doc = Ini::default();
        doc.set("SSP", "compressor", "7z");
        doc.save(&path).unwrap();
        let loaded = Ini::load(&path).unwrap();
        assert_eq!(loaded.get("SSP", "compressor"), Some("7z"));
    }
}
