//! Shared fixtures for the settings contract and stress test crates.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn unique_dir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "image-viewer-settings-test-{}-{}-{}",
        std::process::id(),
        tag,
        seq
    ));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).ok();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn cleanup(dir: &Path) {
    std::fs::remove_dir_all(dir).ok();
}

pub fn temp_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().contains(".tmp.") {
                out.push(entry.path());
            }
        }
    }
    out
}

pub fn bak_path(path: &Path) -> PathBuf {
    path.with_extension("bak")
}

pub fn assert_versioned_once(text: &str) {
    assert!(text.starts_with("version=1\n"), "missing header: {text:?}");
    assert_eq!(
        text.matches("version=1\n").count(),
        1,
        "version header duplicated: {text:?}"
    );
}
