//! Non-Windows compatibility interface. File associations are only implemented on Windows.
use std::io;
use std::path::{Path, PathBuf};

pub const PROG_ID: &str = "ImageViewer.App";
pub const APP_NAME: &str = "画像ビューワー";
pub const CAPS_BASE: &str = "Software\\ImageViewer\\Capabilities";

pub fn exe_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}
pub fn is_default(_ext: &str, _prog_id: &str) -> bool {
    false
}
pub fn register(
    _prog_id: &str,
    _app_name: &str,
    _caps_base: &str,
    _exe: &Path,
    _exts: &[&str],
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file associations require Windows",
    ))
}
pub fn unregister(
    _prog_id: &str,
    _app_name: &str,
    _caps_base: &str,
    _exe_name: &str,
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
