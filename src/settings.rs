use std::path::{Path, PathBuf};

/// Default window size in UI points: first launch, and when the saved value
/// is missing or invalid.
pub const DEFAULT_WINDOW_WIDTH: u32 = 1100;
pub const DEFAULT_WINDOW_HEIGHT: u32 = 750;
/// Saved window sizes outside this range are rejected (a hand-edited or stale
/// file must never produce an unusable window); the defaults are used instead.
pub const MIN_WINDOW_SIZE: u32 = 200;
pub const MAX_WINDOW_SIZE: u32 = 7680;
/// Saved window positions outside this range are rejected (a hand-edited file
/// must never open the window far off-screen); the window is centered instead.
pub const MIN_WINDOW_POS: i32 = -10_000;
pub const MAX_WINDOW_POS: i32 = 10_000;

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
    /// Last normal (non-maximized, non-fullscreen) window size in UI points,
    /// restored at the next startup.
    pub window_width: u32,
    pub window_height: u32,
    /// Last outer (frame/chrome top-left) window position in UI points.
    /// `None` centers the window (first launch, or nothing saved yet).
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    /// Whether the window was maximized when it was last closed.
    pub window_maximized: bool,
}
impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            wheel_navigation: true,
            hold_to_peek: true,
            hover_pan: true,
            auto_hide_toolbar: true,
            zoom_quality: ZoomQuality::High,
            window_width: DEFAULT_WINDOW_WIDTH,
            window_height: DEFAULT_WINDOW_HEIGHT,
            window_x: None,
            window_y: None,
            window_maximized: false,
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
            if key == "window_width" || key == "window_height" {
                if let Ok(size) = value.parse::<u32>() {
                    if (MIN_WINDOW_SIZE..=MAX_WINDOW_SIZE).contains(&size) {
                        if key == "window_width" {
                            settings.window_width = size;
                        } else {
                            settings.window_height = size;
                        }
                    }
                }
                continue;
            }
            if key == "window_x" || key == "window_y" {
                if let Ok(pos) = value.parse::<i32>() {
                    if (MIN_WINDOW_POS..=MAX_WINDOW_POS).contains(&pos) {
                        if key == "window_x" {
                            settings.window_x = Some(pos);
                        } else {
                            settings.window_y = Some(pos);
                        }
                    }
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
                "window_maximized" => settings.window_maximized = value,
                _ => {}
            }
        }
        settings
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = format!(
            "wheel_navigation={}\nhold_to_peek={}\nhover_pan={}\nauto_hide_toolbar={}\nzoom_quality={}\nwindow_width={}\nwindow_height={}\n",
            self.wheel_navigation,
            self.hold_to_peek,
            self.hover_pan,
            self.auto_hide_toolbar,
            self.zoom_quality.as_str(),
            self.window_width,
            self.window_height
        );
        if let Some(x) = self.window_x {
            text.push_str(&format!("window_x={x}\n"));
        }
        if let Some(y) = self.window_y {
            text.push_str(&format!("window_y={y}\n"));
        }
        text.push_str(&format!("window_maximized={}\n", self.window_maximized));
        std::fs::write(path, text)
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
            window_width: 1280,
            window_height: 800,
            window_x: Some(100),
            window_y: Some(-20),
            window_maximized: true,
        };
        changed.save(&path).unwrap();
        assert_eq!(ViewerSettings::load(&path), changed);
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
        // Window size round-trips; out-of-range or unparsable values fall
        // back to the defaults without touching the other size.
        std::fs::write(&path, "window_width=1280\nwindow_height=800\n").unwrap();
        let loaded = ViewerSettings::load(&path);
        assert_eq!((loaded.window_width, loaded.window_height), (1280, 800));
        std::fs::write(
            &path,
            "window_width=abc\nwindow_height=0\nwindow_width=99999\nwindow_height=800\n",
        )
        .unwrap();
        let loaded = ViewerSettings::load(&path);
        assert_eq!(
            (loaded.window_width, loaded.window_height),
            (DEFAULT_WINDOW_WIDTH, 800)
        );
        // Position and the maximized flag round-trip; bad values are ignored.
        std::fs::write(&path, "window_x=100\nwindow_y=-20\nwindow_maximized=true\n").unwrap();
        let loaded = ViewerSettings::load(&path);
        assert_eq!((loaded.window_x, loaded.window_y), (Some(100), Some(-20)));
        assert!(loaded.window_maximized);
        std::fs::write(
            &path,
            "window_x=abc\nwindow_y=99999\nwindow_maximized=yes\nwindow_x=-50\n",
        )
        .unwrap();
        let loaded = ViewerSettings::load(&path);
        assert_eq!((loaded.window_x, loaded.window_y), (Some(-50), None));
        assert!(!loaded.window_maximized);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
