//! Guards the installer's file-type contract against the Rust authority.
//!
//! `src/files.rs` is the single source of truth for supported formats. The
//! Inno Setup script must register exactly those extensions and must not claim
//! video support, which the viewer deliberately rejects.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mov", "avi", "wmv", "webm"];

fn installer_script() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("installer")
        .join("image-viewer.iss");
    fs::read_to_string(path).expect("installer script must exist")
}

fn registered_extensions(script: &str) -> BTreeSet<String> {
    let marker = r"Software\Classes\.";
    script
        .lines()
        .filter_map(|line| {
            let start = line.find(marker)? + marker.len();
            let ext = line[start..].split('\\').next()?;
            if ext.is_empty() || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
                None
            } else {
                Some(ext.to_ascii_lowercase())
            }
        })
        .collect()
}

#[test]
fn installer_registers_exactly_supported_image_types() {
    let script = installer_script();
    let registered = registered_extensions(&script);
    let supported: BTreeSet<String> = image_viewer::SUPPORTED_EXTS
        .iter()
        .map(|ext| (*ext).to_ascii_lowercase())
        .collect();
    assert_eq!(registered, supported, "installer/SUPPORTED_EXTS drift");
}

#[test]
fn installer_never_claims_video_support() {
    let script = installer_script();
    assert!(!script.contains("動画"), "stale video wording in installer");
    for ext in VIDEO_EXTS {
        assert!(
            !registered_extensions(&script).contains(*ext),
            "video extension .{ext} must not be associated"
        );
        assert!(
            !image_viewer::is_supported(std::path::Path::new(&format!("clip.{ext}"))),
            "video extension .{ext} must not be supported"
        );
    }
}
