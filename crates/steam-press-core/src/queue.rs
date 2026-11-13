//! SuperSteamPacker-compatible `.SSPQ` and `.SSWQ` queue files.

use crate::platform::Platform;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameJob {
    pub platform: Platform,
    pub appid: String,
    pub branch: String,
    pub branch_password: String,
}

impl GameJob {
    pub fn new(platform: Platform, appid: impl Into<String>) -> Self {
        Self {
            platform,
            appid: appid.into(),
            branch: "Public".to_string(),
            branch_password: String::new(),
        }
    }

    pub fn with_branch(mut self, branch: impl Into<String>, password: impl Into<String>) -> Self {
        self.branch = branch.into();
        self.branch_password = password.into();
        self
    }

    pub fn is_public_branch(&self) -> bool {
        self.branch.trim().eq_ignore_ascii_case("public")
    }

    pub fn validate(&self) -> Result<(), String> {
        match self.appid.trim().parse::<u64>() {
            Ok(value) if value > 0 => Ok(()),
            _ => Err(format!("AppID invalide : {}", self.appid)),
        }
    }

    pub fn encode(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.platform.key(),
            self.appid.trim(),
            self.branch.trim(),
            self.branch_password
        )
    }

    pub fn decode(line: &str) -> Result<Self, String> {
        let parts: Vec<&str> = line.trim().split('|').collect();
        if parts.len() < 2 {
            return Err(format!("ligne de file invalide : {line}"));
        }
        let platform = Platform::from_key(parts[0])
            .ok_or_else(|| format!("plateforme inconnue : {}", parts[0]))?;
        let branch = parts
            .get(2)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or("Public")
            .to_string();
        let job = Self {
            platform,
            appid: parts[1].trim().to_string(),
            branch,
            branch_password: parts.get(3).map(|value| value.trim().to_string()).unwrap_or_default(),
        };
        job.validate()?;
        Ok(job)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopJob {
    pub appid: String,
    pub workshop_id: String,
}

impl WorkshopJob {
    pub fn new(appid: impl Into<String>, workshop_id: impl Into<String>) -> Self {
        Self {
            appid: appid.into(),
            workshop_id: workshop_id.into(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let appid_ok = self.appid.trim().parse::<u64>().map(|value| value > 0).unwrap_or(false);
        let workshop_ok = self
            .workshop_id
            .trim()
            .parse::<u64>()
            .map(|value| value > 0)
            .unwrap_or(false);
        if !appid_ok {
            return Err(format!("AppID invalide : {}", self.appid));
        }
        if !workshop_ok {
            return Err(format!("Workshop ID invalide : {}", self.workshop_id));
        }
        Ok(())
    }

    pub fn encode(&self) -> String {
        format!("{}|{}", self.appid.trim(), self.workshop_id.trim())
    }

    pub fn decode(line: &str) -> Result<Self, String> {
        let line = line.trim();
        let job = if let Some(rest) = line.strip_prefix("workshop_download_item") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() < 2 {
                return Err(format!("ligne Workshop invalide : {line}"));
            }
            Self::new(parts[0], parts[1])
        } else {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 2 {
                return Err(format!("ligne Workshop invalide : {line}"));
            }
            Self::new(parts[0], parts[1])
        };
        job.validate()?;
        Ok(job)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueLoad<T> {
    pub jobs: Vec<T>,
    pub warnings: Vec<String>,
}

impl<T> Default for QueueLoad<T> {
    fn default() -> Self {
        Self {
            jobs: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

pub fn parse_game_queue(content: &str) -> QueueLoad<GameJob> {
        let mut load = QueueLoad::default();
    for (index, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        match GameJob::decode(line) {
            Ok(job) => {
                if !load.jobs.contains(&job) {
                    load.jobs.push(job);
                }
            }
            Err(error) => load.warnings.push(format!("ligne {} : {error}", index + 1)),
        }
    }
    load
}

pub fn parse_workshop_queue(content: &str) -> QueueLoad<WorkshopJob> {
    let mut load = QueueLoad::default();
    for (index, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        match WorkshopJob::decode(line) {
            Ok(job) => {
                if !load.jobs.contains(&job) {
                    load.jobs.push(job);
                }
            }
            Err(error) => load.warnings.push(format!("ligne {} : {error}", index + 1)),
        }
    }
    load
}

pub fn load_game_queue(path: impl AsRef<Path>) -> Result<QueueLoad<GameJob>, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    Ok(parse_game_queue(&content))
}

pub fn load_workshop_queue(path: impl AsRef<Path>) -> Result<QueueLoad<WorkshopJob>, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    Ok(parse_workshop_queue(&content))
}

pub fn save_game_queue(path: impl AsRef<Path>, jobs: &[GameJob]) -> Result<(), String> {
    let mut content = String::new();
    for job in jobs {
        content.push_str(&job.encode());
        content.push('\n');
    }
    fs::write(path, content).map_err(|error| error.to_string())
}

pub fn save_workshop_queue(path: impl AsRef<Path>, jobs: &[WorkshopJob]) -> Result<(), String> {
    let mut content = String::new();
    for job in jobs {
        content.push_str(&job.encode());
        content.push('\n');
    }
    fs::write(path, content).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_sspq_lines() {
        let job = GameJob::new(Platform::Linux64, "2379780").with_branch("public", "");
        assert_eq!(job.encode(), "lin64|2379780|public|");
        assert_eq!(GameJob::decode(&job.encode()).unwrap(), job);
    }

    #[test]
    fn tolerates_short_and_legacy_lines() {
        let job = GameJob::decode("win64|123").unwrap();
        assert_eq!(job.branch, "Public");
        let workshop = WorkshopJob::decode("workshop_download_item 730 123456 validate").unwrap();
        assert_eq!(
            workshop,
            WorkshopJob {
                appid: "730".into(),
                workshop_id: "123456".into()
            }
        );
    }

    #[test]
    fn rejects_non_numeric_appids() {
        assert!(GameJob::decode("lin64|abc|public|").unwrap_err().contains("AppID"));
        assert!(WorkshopJob::decode("730|notanid").is_err());
    }

    #[test]
    fn parses_files_and_skips_comments_and_duplicates() {
        let content = "; queue\nlin64|10|public|\n\nlin64|10|public|\nwin64|20|beta|pw\nbroken\n";
        let load = parse_game_queue(content);
        assert_eq!(load.jobs.len(), 2);
        assert_eq!(load.jobs[1].branch, "beta");
        assert_eq!(load.warnings.len(), 1);
    }

    #[test]
    fn saves_and_loads_both_queue_types() {
        let dir = crate::test_support::temp_dir("queues");
        let game_path = dir.join("queue.sspq");
        save_game_queue(&game_path, &[GameJob::new(Platform::Mac, "42")]).unwrap();
        let loaded = load_game_queue(&game_path).unwrap();
        assert_eq!(loaded.jobs.len(), 1);

        let workshop_path = dir.join("queue.sswq");
        save_workshop_queue(&workshop_path, &[WorkshopJob::new("730", "999")]).unwrap();
        let loaded = load_workshop_queue(&workshop_path).unwrap();
        assert_eq!(loaded.jobs[0].workshop_id, "999");
    }
}
