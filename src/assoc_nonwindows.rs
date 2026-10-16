//! Non-Windows compatibility interface. File associations are only implemented on Windows.
//!
//! This mirror exposes the same public call shapes as `assoc.rs` so shared
//! code (`view.rs`, `main.rs`) compiles on every platform. All operations
//! that would touch the Windows registry report `Unsupported`, except input
//! validation which mirrors the Windows side (`InvalidInput`).
use std::io;
use std::path::{Path, PathBuf};

pub const PROG_ID: &str = "ImageViewer.App";
pub const APP_NAME: &str = "画像ビューワー";
pub const CAPS_BASE: &str = "Software\\ImageViewer\\Capabilities";

/// Same normalization as the Windows side: trim, strip leading dots,
/// lowercase, ASCII-alphanumeric non-empty only.
pub fn sanitize_ext(ext: &str) -> Option<String> {
    let clean = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if !clean.is_empty() && clean.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Some(clean)
    } else {
        None
    }
}

pub fn exe_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}
pub fn is_default(ext: &str, _prog_id: &str) -> bool {
    // Mirror the Windows guard: invalid extensions are never default.
    if sanitize_ext(ext).is_none() {
        return false;
    }
    false
}
pub fn register(
    _prog_id: &str,
    _app_name: &str,
    _caps_base: &str,
    _exe: &Path,
    exts: &[&str],
) -> io::Result<()> {
    // Validate first so callers get the same `InvalidInput` as Windows.
    for ext in exts {
        if sanitize_ext(ext).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid extension: {ext}"),
            ));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file associations require Windows",
    ))
}
pub fn unregister(
    _prog_id: &str,
    _app_name: &str,
    _caps_base: &str,
    _exe: &Path,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file associations require Windows",
    ))
}
pub fn open_default_apps() -> io::Result<std::process::Child> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "default-app settings require Windows",
    ))
}

/// Non-Windows `--print-assoc-config` reports Unsupported (exit 1), so the
/// corresponding e2e test runs on Windows only.
pub fn print_assoc_config() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file associations require Windows",
    ))
}
