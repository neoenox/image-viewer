//! Single source of truth for association identifiers and extension rules.
//!
//! Both `assoc.rs` and its non-Windows mirror re-export these definitions so
//! shared callers and the parity test see identical values on every platform.

/// Production ProgID used by the binary and both installers.
pub const PROG_ID: &str = "ImageViewer.App";
/// Display name used in registry values and installer metadata.
pub const APP_NAME: &str = "画像ビューワー";
/// Capabilities registry path, relative to HKCU.
pub const CAPS_BASE: &str = r"Software\ImageViewer\Capabilities";

/// Normalize an extension for registry use: trim, strip leading dots, lowercase,
/// ASCII-alphanumeric non-empty only.
pub fn sanitize_ext(ext: &str) -> Option<String> {
    let clean = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if !clean.is_empty() && clean.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Some(clean)
    } else {
        None
    }
}
