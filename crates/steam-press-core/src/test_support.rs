//! Small helpers shared by unit tests.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Creates a unique temporary directory so parallel tests never collide.
pub fn temp_dir(name: &str) -> PathBuf {
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "steam-press-test-{}-{}-{}",
        name,
        std::process::id(),
        counter
    ));
    std::fs::create_dir_all(&dir).expect("create temporary test directory");
    dir
}
