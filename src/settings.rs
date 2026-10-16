use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Windows sharing-violation (32) / access-denied (5) from concurrent
/// `copy` vs `rename` on the same live file. Retryable with a short sleep;
/// a persistent failure still returns an error (rename) or proceeds
/// best-effort (backup copy, where the live file matters more).
fn is_retryable_share(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(32) | Some(5))
}

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

/// Keys understood by `load` below. Anything else is carried verbatim (see
/// `unknown`) so a save never drops fields written by a newer release.
const KNOWN_KEYS: &[&str] = &[
    "version",
    "wheel_navigation",
    "hold_to_peek",
    "hover_pan",
    "auto_hide_toolbar",
    "zoom_quality",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerSettings {
    pub wheel_navigation: bool,
    pub hold_to_peek: bool,
    pub hover_pan: bool,
    pub auto_hide_toolbar: bool,
    pub zoom_quality: ZoomQuality,
    /// `key=value` lines from a newer release, kept verbatim and re-emitted
    /// on save. `version` is excluded (always emitted as the header).
    pub unknown: Vec<(String, String)>,
}
impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            wheel_navigation: true,
            hold_to_peek: true,
            hover_pan: true,
            auto_hide_toolbar: true,
            zoom_quality: ZoomQuality::High,
            unknown: Vec::new(),
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
            if key.is_empty() {
                continue;
            }
            if !KNOWN_KEYS.contains(&key) {
                settings.unknown.push((key.to_owned(), value.to_owned()));
                continue;
            }
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
        let mut text = format!(
            "version=1\nwheel_navigation={}\nhold_to_peek={}\nhover_pan={}\nauto_hide_toolbar={}\nzoom_quality={}\n",
            self.wheel_navigation,
            self.hold_to_peek,
            self.hover_pan,
            self.auto_hide_toolbar,
            self.zoom_quality.as_str()
        );
        // Future fields carried verbatim (see `unknown`): re-emit last so a
        // reload parses them back identically.
        for (key, value) in &self.unknown {
            text.push_str(&format!("{key}={value}\n"));
        }
        // Stage in the same directory and rename; never truncate the live file.
        let name = path
            .file_name()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "settings file name is missing",
                )
            })?
            .to_string_lossy();
        let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_file_name(format!("{name}.tmp.{}.{}", std::process::id(), sequence));
        let result = (|| -> std::io::Result<()> {
            // `create_new` guards against clobbering another writer, but a
            // stale temp file (crash litter, then PID reuse) must not fail
            // the save: a live process can never share our pid+suffixed name,
            // so a pre-existing file is dead litter and safe to replace.
            let mut file = match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
            {
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::fs::remove_file(&temp).ok();
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&temp)?
                }
                opened => opened?,
            };
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            // Keep a copy of the last readable settings for recovery.
            // Best-effort under concurrency: retry briefly on sharing
            // violation, then proceed even if the copy failed (the live
            // file matters more; single-threaded saves always copy).
            if path.is_file() {
                let bak = path.with_extension("bak");
                for attempt in 0..10u32 {
                    match std::fs::copy(path, &bak) {
                        Ok(_) => break,
                        Err(error) if is_retryable_share(&error) && attempt + 1 < 10 => {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                        Err(_) => break,
                    }
                }
            }
            // Atomic commit: retry on Windows sharing violation from
            // concurrent readers/writers (copy vs rename racing).
            for attempt in 0..20u32 {
                match std::fs::rename(&temp, path) {
                    Ok(()) => break,
                    Err(error) if is_retryable_share(&error) && attempt + 1 < 20 => {
                        std::thread::sleep(std::time::Duration::from_millis(
                            1 + u64::from(attempt),
                        ));
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
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
            unknown: Vec::new(),
        };
        changed.save(&path).unwrap();
        assert_eq!(ViewerSettings::load(&path), changed);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("version=1\n"));
        // A second save creates a recovery copy before the replacement.
        // Recovery is manual (copy the .bak back); the guarantee locked here
        // is that the .bak holds the exact pre-save settings.
        ViewerSettings::default().save(&path).unwrap();
        assert!(path.with_extension("bak").is_file());
        assert_eq!(ViewerSettings::load(&path.with_extension("bak")), changed);
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

    #[test]
    fn unknown_future_keys_survive_load_save_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "viewer-settings-future-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.load(Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        // Fields from a newer release (e.g. #63's window_*): understood by
        // neither load nor save as settings, yet never dropped either.
        std::fs::write(
            &path,
            "version=1\nwindow_width=1280\nwindow_height=800\nrestore_window=true\n",
        )
        .unwrap();
        let loaded = ViewerSettings::load(&path);
        // Known fields stay at their defaults; only `unknown` differs.
        assert_eq!(
            ViewerSettings {
                unknown: Vec::new(),
                ..loaded.clone()
            },
            ViewerSettings::default()
        );
        assert_eq!(
            loaded.unknown,
            [
                ("window_width".to_owned(), "1280".to_owned()),
                ("window_height".to_owned(), "800".to_owned()),
                ("restore_window".to_owned(), "true".to_owned()),
            ]
        );
        loaded.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("window_width=1280\n"), "{text}");
        assert!(text.contains("window_height=800\n"), "{text}");
        assert!(text.contains("restore_window=true\n"), "{text}");
        // The schema marker is emitted exactly once, never duplicated.
        assert_eq!(text.matches("version=1\n").count(), 1, "{text}");
        // And a reload parses back identically (no duplication drift).
        assert_eq!(ViewerSettings::load(&path).unknown, loaded.unknown);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn old_format_without_version_migrates_cleanly() {
        // Files written before schema versions existed: values carry over,
        // the header is added on save, and nothing else changes shape.
        let dir = std::env::temp_dir().join(format!(
            "viewer-settings-oldfmt-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.load(Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        std::fs::write(
            &path,
            "wheel_navigation=false\nhold_to_peek=false\nhover_pan=false\nauto_hide_toolbar=false\nzoom_quality=standard\n",
        )
        .unwrap();
        let loaded = ViewerSettings::load(&path);
        assert!(!loaded.wheel_navigation && !loaded.hover_pan);
        assert_eq!(loaded.zoom_quality, ZoomQuality::Standard);
        assert!(loaded.unknown.is_empty());
        loaded.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("version=1\n"), "{text}");
        assert!(text.contains("wheel_navigation=false\n"), "{text}");
        assert!(text.contains("zoom_quality=standard\n"), "{text}");
        // Reloading the migrated file yields the same settings.
        assert_eq!(ViewerSettings::load(&path), loaded);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn window_era_keys_survive_any_schema() {
        // Bridge (#63 x #46): these lines are unknown to this version and
        // known fields after #63. Either way a load-save cycle must keep
        // them, exactly once, and reach a fixed point. This stays green on
        // both sides of the #63 integration.
        let dir = std::env::temp_dir().join(format!(
            "viewer-settings-bridge-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.load(Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        std::fs::write(
            &path,
            "version=1\nwindow_width=1280\nwindow_height=800\nrestore_window=true\n",
        )
        .unwrap();
        ViewerSettings::load(&path).save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("window_width=1280\n"), "{text}");
        assert!(text.contains("window_height=800\n"), "{text}");
        assert!(text.contains("restore_window=true\n"), "{text}");
        assert_eq!(text.matches("version=1\n").count(), 1, "{text}");
        // Second cycle changes nothing: a fixed point, no duplication drift.
        let once = std::fs::read(&path).unwrap();
        ViewerSettings::load(&path).save(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), once);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn unicode_and_spaced_paths_round_trip() {
        // Japanese, spaces and parens in the directory: save/load must work
        // byte-identically (Windows user profiles commonly contain these).
        let dir = std::env::temp_dir().join(format!(
            "画像ビューアー設定 (白) {}-{}",
            std::process::id(),
            SAVE_SEQUENCE.load(Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        let changed = ViewerSettings {
            hover_pan: false,
            zoom_quality: ZoomQuality::Sharp,
            ..ViewerSettings::default()
        };
        changed.save(&path).unwrap();
        assert_eq!(ViewerSettings::load(&path), changed);
        let first = std::fs::read(&path).unwrap();
        changed.save(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), first);
        assert!(path.with_extension("bak").is_file());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_heals_stale_temp_file() {
        let dir = std::env::temp_dir().join(format!(
            "viewer-settings-stale-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.load(Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        // Plant crash litter for the upcoming sequence values: SAVE_SEQUENCE
        // is process-global, so cover a window against concurrent tests
        // (each in its own directory, never touching these files).
        let seq = SAVE_SEQUENCE.load(Ordering::Relaxed);
        for s in seq..seq + 64 {
            std::fs::write(
                dir.join(format!("settings.txt.tmp.{}.{s}", std::process::id())),
                b"stale",
            )
            .unwrap();
        }
        // The save must succeed despite the litter, using a healed temp file.
        ViewerSettings::default().save(&path).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("version=1\n"));
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        // Exactly the consumed temp file is gone; the untouched spares remain.
        assert_eq!(leftovers.len(), 63, "leftovers: {leftovers:?}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn failed_save_leaves_no_temp_litter() {
        let dir = std::env::temp_dir().join(format!(
            "viewer-settings-fail-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.load(Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Use a *directory* as the target: the rename can never succeed, so
        // the save takes its retry-exhaustion path and returns an error.
        let target = dir.join("settings.txt");
        std::fs::create_dir(&target).unwrap();
        assert!(ViewerSettings::default().save(&target).is_err());
        // The staging temp file must not survive a failed save.
        let temps: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(temps.is_empty(), "temp litter: {temps:?}");
        // The pre-existing target (a directory) is untouched.
        assert!(target.is_dir());
        std::fs::remove_dir_all(dir).ok();
    }
}
