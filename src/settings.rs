use std::path::{Path, PathBuf};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// How the image is drawn when magnified past its source pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomQuality {
    /// GPU bilinear stretch only (fastest).
    Standard,
    /// Lanczos3 resample of the visible part to screen pixels on a background thread.
    High,
    /// `High` followed by a light unsharp mask for crisper edges.
    Sharp,
}
impl ZoomQuality {
    fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::High => "high",
            Self::Sharp => "sharp",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        match value {
            "standard" => Some(Self::Standard),
            "high" => Some(Self::High),
            "sharp" => Some(Self::Sharp),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerSettings {
    pub wheel_navigation: bool,
    pub hold_to_peek: bool,
    pub hover_pan: bool,
    pub auto_hide_toolbar: bool,
    pub zoom_quality: ZoomQuality,
}
impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            wheel_navigation: true,
            hold_to_peek: true,
            hover_pan: true,
            auto_hide_toolbar: true,
            zoom_quality: ZoomQuality::High,
        }
    }
}
impl ViewerSettings {
    /// `%APPDATA%\image-viewer\settings.txt`, or `None` when APPDATA is unset.
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(|dir| PathBuf::from(dir).join("image-viewer").join("settings.txt"))
    }
    /// Reads `key=value` lines. Missing or unreadable files, unknown keys and bad
    /// values fall back to the defaults, so an old or hand-edited file never blocks
    /// startup.
    pub fn load(path: &Path) -> Self {
        let mut settings = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return settings;
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if key == "zoom_quality" {
                if let Some(quality) = ZoomQuality::parse(value) {
                    settings.zoom_quality = quality;
                }
                continue;
            }
            let value = match value {
                "true" => true,
                "false" => false,
                _ => continue,
            };
            match key {
                "wheel_navigation" => settings.wheel_navigation = value,
                "hold_to_peek" => settings.hold_to_peek = value,
                "hover_pan" => settings.hover_pan = value,
                "auto_hide_toolbar" => settings.auto_hide_toolbar = value,
                _ => {}
            }
        }
        settings
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = format!(
            "version=1\nwheel_navigation={}\nhold_to_peek={}\nhover_pan={}\nauto_hide_toolbar={}\nzoom_quality={}\n",
            self.wheel_navigation,
            self.hold_to_peek,
            self.hover_pan,
            self.auto_hide_toolbar,
            self.zoom_quality.as_str()
        );
        // Stage in the same directory and rename; never truncate the live file.
        let name = path.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "settings file name is missing")
        })?.to_string_lossy();
        let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_file_name(format!("{name}.tmp.{}.{}", std::process::id(), sequence));
        let result = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            // Keep a copy of the last readable settings for recovery.
            if path.is_file() {
                std::fs::copy(path, path.with_extension("bak"))?;
            }
            std::fs::rename(&temp, path)
        })();
        if result.is_err() {
            std::fs::remove_file(&temp).ok();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_tolerate_bad_files() {
        let dir = std::env::temp_dir().join(format!("viewer-settings-{}", std::process::id()));
        let path = dir.join("nested").join("settings.txt");
        // Missing file: defaults.
        assert_eq!(ViewerSettings::load(&path), ViewerSettings::default());
        let changed = ViewerSettings {
            wheel_navigation: false,
            hold_to_peek: true,
            hover_pan: false,
            auto_hide_toolbar: false,
            zoom_quality: ZoomQuality::Standard,
        };
        changed.save(&path).unwrap();
        assert_eq!(ViewerSettings::load(&path), changed);
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("version=1\\n"));
        // A second save creates a recovery copy before the replacement.
        ViewerSettings::default().save(&path).unwrap();
        assert!(path.with_extension("bak").is_file());
        assert_eq!(ViewerSettings::load(&path), ViewerSettings::default());
        changed.save(&path).unwrap();
        // Unknown keys and bad values are ignored; known good lines still apply.
        std::fs::write(
            &path,
            "hover_pan=false\nfuture_option=true\nhold_to_peek=maybe\nzoom_quality=ultra\ngarbage\n",
        )
        .unwrap();
        let loaded = ViewerSettings::load(&path);
        assert!(!loaded.hover_pan);
        assert!(loaded.hold_to_peek && loaded.wheel_navigation && loaded.auto_hide_toolbar);
        assert_eq!(loaded.zoom_quality, ZoomQuality::High);
        std::fs::write(&path, "zoom_quality=standard\n").unwrap();
        assert_eq!(
            ViewerSettings::load(&path).zoom_quality,
            ZoomQuality::Standard
        );
        std::fs::write(&path, "zoom_quality=sharp\n").unwrap();
        assert_eq!(ViewerSettings::load(&path).zoom_quality, ZoomQuality::Sharp);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
