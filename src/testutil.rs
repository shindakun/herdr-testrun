//! Helpers for unit tests. Compiled only under `cfg(test)`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

/// A fresh empty directory under the system temp dir. Not removed; the OS
/// cleans temp, and leaving it helps when a test fails.
pub fn tempdir(tag: &str) -> PathBuf {
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("herdr-testrun-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The argv of a `Command`, program first, for asserting on built commands.
pub fn argv(cmd: &std::process::Command) -> Vec<String> {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|s| s.to_string_lossy().into_owned())
        .collect()
}
