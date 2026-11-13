//! Testable view state for the desktop application.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use steam_press_core::events::PackEvent;
use steam_press_core::queue::{GameJob, WorkshopJob};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryStatus {
    Ready,
    Running,
    Done,
    Skipped,
    Failed(String),
    Cancelled,
}

impl EntryStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Running => "Running",
            Self::Done => "Completed",
            Self::Skipped => "Skipped",
            Self::Failed(_) => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Step,
    Success,
    Warning,
    Error,
}

impl LogLevel {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Step => "STEP",
            Self::Success => "DONE",
            Self::Warning => "WARN",
            Self::Error => "ERR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: LogLevel,
    pub text: String,
}

impl LogEntry {
    pub fn new(level: LogLevel, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueEntry<T> {
    pub job: T,
    pub status: EntryStatus,
}

#[derive(Debug, Clone)]
pub struct JobQueue<T> {
    entries: Vec<QueueEntry<T>>,
}

impl<T> Default for JobQueue<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T: PartialEq + Clone> JobQueue<T> {
    /// Adds a job unless it is already queued.
    pub fn push(&mut self, job: T) -> bool {
        if self.entries.iter().any(|entry| entry.job == job) {
            return false;
        }
        self.entries.push(QueueEntry {
            job,
            status: EntryStatus::Ready,
        });
        true
    }

    pub fn entries(&self) -> &[QueueEntry<T>] {
        &self.entries
    }

    pub fn jobs(&self) -> Vec<T> {
        self.entries.iter().map(|entry| entry.job.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn remove(&mut self, index: usize) -> Option<QueueEntry<T>> {
        if index < self.entries.len() {
            Some(self.entries.remove(index))
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn move_up(&mut self, index: usize) {
        if index > 0 && index < self.entries.len() {
            self.entries.swap(index, index - 1);
        }
    }

    pub fn move_down(&mut self, index: usize) {
        if index + 1 < self.entries.len() {
            self.entries.swap(index, index + 1);
        }
    }

    pub fn set_status<F>(&mut self, key: F, status: EntryStatus)
    where
        F: Fn(&T) -> bool,
    {
        for entry in &mut self.entries {
            if key(&entry.job) {
                entry.status = status.clone();
            }
        }
    }

    pub fn cancel_unfinished(&mut self) {
        for entry in &mut self.entries {
            if matches!(entry.status, EntryStatus::Ready | EntryStatus::Running) {
                entry.status = EntryStatus::Cancelled;
            }
        }
    }

    pub fn reset_statuses(&mut self) {
        for entry in &mut self.entries {
            entry.status = EntryStatus::Ready;
        }
    }
}

pub type GameQueue = JobQueue<GameJob>;
pub type WorkshopQueue = JobQueue<WorkshopJob>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub game: Option<GameJob>,
    pub workshop: Option<WorkshopJob>,
    pub status: String,
    pub outcome: String,
    pub output: Option<PathBuf>,
    pub finished_at: u64,
    #[serde(default)]
    pub log: Vec<String>,
}

impl HistoryEntry {
    pub fn label(&self) -> String {
        if let Some(game) = &self.game {
            format!("{} · {}", game.appid, game.platform.label())
        } else if let Some(workshop) = &self.workshop {
            format!("{}/{}", workshop.appid, workshop.workshop_id)
        } else {
            "Unknown job".to_string()
        }
    }

    pub fn is_failed(&self) -> bool {
        self.status == "failed"
    }

    pub fn new(
        game: Option<GameJob>,
        workshop: Option<WorkshopJob>,
        status: impl Into<String>,
        outcome: impl Into<String>,
        output: Option<PathBuf>,
    ) -> Self {
        Self {
            game,
            workshop,
            status: status.into(),
            outcome: outcome.into(),
            output,
            finished_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
            log: Vec::new(),
        }
    }
}

pub fn load_history(path: impl AsRef<Path>) -> Vec<HistoryEntry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_history(path: impl AsRef<Path>, history: &[HistoryEntry]) -> Result<(), String> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    let text = serde_json::to_string_pretty(history).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Event(PackEvent),
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use steam_press_core::Platform;

    #[test]
    fn queue_state_adds_deduplicates_and_reorders() {
        let mut state = GameQueue::default();
        assert!(state.push(GameJob::new(Platform::Linux64, "10")));
        assert!(!state.push(GameJob::new(Platform::Linux64, "10")));
        state.push(GameJob::new(Platform::Win64, "20"));
        state.move_up(1);
        assert_eq!(state.jobs()[0].appid, "20");
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn statuses_update_for_matching_jobs() {
        let mut state = WorkshopQueue::default();
        state.push(WorkshopJob::new("730", "1"));
        state.push(WorkshopJob::new("730", "2"));
        state.set_status(|job| job.workshop_id == "2", EntryStatus::Done);
        assert_eq!(state.entries()[0].status, EntryStatus::Ready);
        assert_eq!(state.entries()[1].status, EntryStatus::Done);
        state.reset_statuses();
        assert_eq!(state.entries()[1].status, EntryStatus::Ready);
    }

    #[test]
    fn cancellation_marks_only_unfinished_jobs() {
        let mut state = GameQueue::default();
        state.push(GameJob::new(Platform::Linux64, "10"));
        state.push(GameJob::new(Platform::Linux64, "20"));
        state.set_status(|job| job.appid == "10", EntryStatus::Done);
        state.cancel_unfinished();
        assert_eq!(state.entries()[0].status, EntryStatus::Done);
        assert_eq!(state.entries()[1].status, EntryStatus::Cancelled);
    }

    #[test]
    fn history_round_trips_and_keeps_recent_entries() {
        let dir = steam_press_core::test_support::temp_dir("history");
        let path = dir.join("history.json");
        let entries = vec![
            HistoryEntry::new(
                Some(GameJob::new(steam_press_core::Platform::Linux64, "10")),
                None,
                "completed",
                "ok",
                Some(dir.join("out.7z")),
            ),
            HistoryEntry::new(
                None,
                Some(WorkshopJob::new("730", "1")),
                "failed",
                "broken",
                None,
            ),
        ];
        save_history(&path, &entries).unwrap();
        let loaded = load_history(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].label(), "10 · Linux x64");
        assert!(loaded[1].is_failed());
    }

    #[test]
    fn log_entry_levels_have_tags() {
        assert_eq!(LogLevel::Info.tag(), "INFO");
        assert_eq!(LogLevel::Step.tag(), "STEP");
        assert_eq!(LogLevel::Success.tag(), "DONE");
        assert_eq!(LogLevel::Warning.tag(), "WARN");
        assert_eq!(LogLevel::Error.tag(), "ERR");
    }
}
