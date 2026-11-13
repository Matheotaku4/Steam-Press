//! Core logic for Steam Press.
//!
//! This crate contains every business rule behind the CLI and the desktop
//! application: settings, Steam metadata, manifests, downloader engines,
//! archives and release templates. It has no GUI dependency.

pub mod compressor;
pub mod config;
pub mod credentials;
pub mod ini;
pub mod naming;
pub mod platform;
pub mod queue;
pub mod test_support;
pub mod tooling;

pub use compressor::Compressor;
pub use config::{Paths, Settings};
pub use platform::Platform;
pub use queue::{GameJob, QueueLoad, WorkshopJob};
