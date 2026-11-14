//! Steam depot names (`depots.ini`, SteamDepotNames format).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

const DEPOT_NAMES_URL: &str =
    "https://raw.githubusercontent.com/Masquerade64/SteamDepotNames/main/depots.ini";
const USER_AGENT: &str = "SteamPress/0.1";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DepotNames {
    entries: BTreeMap<u32, String>,
}

impl DepotNames {
    pub fn parse(input: &str) -> Self {
        let mut entries = BTreeMap::new();
        let mut in_depots = true;

        for raw in input.lines() {
            let line = raw.trim().trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                in_depots = name.trim().eq_ignore_ascii_case("depots");
                continue;
            }
            if !in_depots {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let Ok(depot_id) = key.trim().parse::<u32>() else {
                continue;
            };
            let name = value.trim();
            if name.is_empty() || name.eq_ignore_ascii_case("unknown") {
                continue;
            }
            entries.insert(depot_id, name.to_string());
        }

        Self { entries }
    }

    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self::parse(&fs::read_to_string(path)?))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, self.to_ini_string())
    }

    pub fn get(&self, depot_id: &str) -> Option<&str> {
        depot_id
            .trim()
            .parse::<u32>()
            .ok()
            .and_then(|id| self.entries.get(&id))
            .map(String::as_str)
    }

    pub fn name_or<'a>(&'a self, depot_id: &str, fallback: &'a str) -> &'a str {
        self.get(depot_id).unwrap_or(fallback)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = (u32, &str)> {
        self.entries.iter().map(|(id, name)| (*id, name.as_str()))
    }

    /// Adds names that are absent locally. Returns how many were added.
    pub fn merge(&mut self, other: &DepotNames) -> usize {
        let mut added = 0;
        for (id, name) in &other.entries {
            if !self.entries.contains_key(id) {
                self.entries.insert(*id, name.clone());
                added += 1;
            }
        }
        added
    }

    pub fn to_ini_string(&self) -> String {
        let mut output = String::from("; steam-press-format=1\n[depots]\n");
        for (id, name) in &self.entries {
            output.push_str(&format!("{id} = {name}\n"));
        }
        output
    }

    pub fn from_url(url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| format!("HTTP client failed: {error}"))?;
        let response = client
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .map_err(|error| format!("could not download depot names: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("depot names download: HTTP {}", response.status()));
        }
        let body = response
            .text()
            .map_err(|error| format!("could not read depot names: {error}"))?;
        Ok(Self::parse(&body))
    }

    pub fn from_official_source() -> Result<Self, String> {
        Self::from_url(DEPOT_NAMES_URL)
    }

    /// Reloads the local file from the official SteamDepotNames source.
    pub fn sync(&mut self) -> Result<usize, String> {
        let fresh = Self::from_official_source()?;
        *self = fresh;
        Ok(self.len())
    }
}

pub fn official_url() -> &'static str {
    DEPOT_NAMES_URL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_depot_names_and_merges_without_unknown() {
        let mut names = DepotNames::parse("[depots]\n10 = Counter-Strike\n11 = Unknown\n");
        assert_eq!(names.get("10"), Some("Counter-Strike"));
        assert!(names.get("11").is_none());

        let other = DepotNames::parse("[depots]\n10 = Other\n12 = New\n");
        assert_eq!(names.merge(&other), 1);
        assert_eq!(names.get("10"), Some("Counter-Strike"));
        assert_eq!(names.get("12"), Some("New"));
    }

    #[test]
    fn ignores_other_sections_and_comments() {
        let names =
            DepotNames::parse("; comment\n[metadata]\n10 = Wrong\n[depots]\n# note\n20 = Right\n");
        assert!(names.get("10").is_none());
        assert_eq!(names.get("20"), Some("Right"));
    }

    #[test]
    fn writes_a_sorted_supersteampacker_file() {
        let names = DepotNames::parse("[depots]\n20 = B\n10 = A\n");
        let text = names.to_ini_string();
        assert_eq!(text, "; steam-press-format=1\n[depots]\n10 = A\n20 = B\n");
        assert_eq!(DepotNames::parse(&text), names);
        let legacy = DepotNames::parse("[depots]\n10 = A\n20 = B\n");
        assert_eq!(legacy, names);
    }

    #[test]
    fn saves_and_loads_a_file() {
        let dir = crate::test_support::temp_dir("depots");
        let path = dir.join("depots.ini");
        let names = DepotNames::parse("[depots]\n10 = Counter-Strike\n");
        names.save(&path).unwrap();
        assert_eq!(DepotNames::load(&path).unwrap(), names);
    }
}
