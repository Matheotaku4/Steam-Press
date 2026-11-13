//! Core logic for Steam Press.
//!
//! This crate contains every business rule behind the CLI and the desktop
//! application: settings, Steam metadata, manifests, downloader engines,
//! archives and release templates. It has no GUI dependency.

pub mod acf;
pub mod archive;
pub mod compressor;
pub mod config;
pub mod credentials;
pub mod depots;
pub mod downloader;
pub mod events;
pub mod ini;
pub mod naming;
pub mod pack;
pub mod platform;
pub mod queue;
pub mod steam;
pub mod template;
pub mod test_support;
pub mod tooling;
pub mod vdf;
pub mod workshop;

pub use compressor::Compressor;
pub use config::{Paths, Settings};
pub use depots::DepotNames;
pub use downloader::{Downloader, DownloaderKind};
pub use events::{CancellationToken, EventSink, PackEvent, Stage};
pub use pack::{PackOptions, PackOutcome, PackStatus, Packer, PipelineError, Preflight};
pub use platform::Platform;
pub use queue::{GameJob, QueueLoad, WorkshopJob};
pub use steam::{AppInfo, DepotEntry, StoreInfo, StoreSearchItem};
pub use workshop::{WorkshopOutcome, WorkshopPacker};
