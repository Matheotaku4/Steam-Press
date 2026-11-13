//! Core logic for Steam Press.
//!
//! This crate contains every business rule behind the CLI and the desktop
//! application: settings, Steam metadata, manifests, downloader engines,
//! archives and release templates. It has no GUI dependency.

pub mod acf;
pub mod compressor;
pub mod config;
pub mod credentials;
pub mod depots;
pub mod ini;
pub mod naming;
pub mod platform;
pub mod queue;
pub mod steam;
pub mod test_support;
pub mod tooling;
pub mod vdf;

pub use compressor::Compressor;
pub use config::{Paths, Settings};
pub use depots::DepotNames;
pub use platform::Platform;
pub use queue::{GameJob, QueueLoad, WorkshopJob};
pub use steam::{AppInfo, DepotEntry, StoreInfo};
