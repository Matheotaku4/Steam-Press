//! Progress events shared by the CLI, the desktop app and the pipeline.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Preflight,
    Download,
    Normalize,
    Manifest,
    Archive,
    Template,
    Done,
}

impl Stage {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Preflight => "Préflight",
            Self::Download => "Téléchargement",
            Self::Normalize => "Nettoyage",
            Self::Manifest => "Manifests",
            Self::Archive => "Compression",
            Self::Template => "Informations",
            Self::Done => "Terminé",
        }
    }
}

#[derive(Debug, Clone)]
pub enum PackEvent {
    Stage {
        job: String,
        stage: Stage,
        message: String,
    },
    Progress {
        job: String,
        percent: Option<f32>,
        message: String,
    },
    Log(String),
    Finished {
        job: String,
        outcome: String,
        output: Option<PathBuf>,
    },
}

/// Thread-safe callback receiving pipeline events.
#[derive(Clone)]
pub struct EventSink(Arc<dyn Fn(PackEvent) + Send + Sync>);

impl EventSink {
    pub fn new<F>(handler: F) -> Self
    where
        F: Fn(PackEvent) + Send + Sync + 'static,
    {
        Self(Arc::new(handler))
    }

    pub fn emit(&self, event: PackEvent) {
        (self.0)(event);
    }

    pub fn noop() -> Self {
        Self::new(|_| {})
    }

    /// Sink paired with a receiver, used by the GUI worker.
    pub fn channel() -> (Self, Receiver<PackEvent>) {
        let (sender, receiver) = mpsc::channel();
        let sink = Self::new(move |event| {
            let _ = sender.send(event);
        });
        (sink, receiver)
    }
}

impl std::fmt::Debug for EventSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EventSink")
    }
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_forwards_events_and_cancel_is_sticky() {
        let (sink, receiver) = EventSink::channel();
        sink.emit(PackEvent::Stage {
            job: "job".into(),
            stage: Stage::Download,
            message: "go".into(),
        });
        assert!(matches!(receiver.try_recv(), Ok(PackEvent::Stage { .. })));

        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }
}
