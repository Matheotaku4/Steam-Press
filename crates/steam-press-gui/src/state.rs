//! Testable view state for the desktop application.

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
            Self::Ready => "Prêt",
            Self::Running => "En cours",
            Self::Done => "Terminé",
            Self::Skipped => "Ignoré",
            Self::Failed(_) => "Échec",
            Self::Cancelled => "Annulé",
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

    pub fn reset_statuses(&mut self) {
        for entry in &mut self.entries {
            entry.status = EntryStatus::Ready;
        }
    }
}

pub type GameQueue = JobQueue<GameJob>;
pub type WorkshopQueue = JobQueue<WorkshopJob>;

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
}
