//! Integration contract for PR #46 after PR #63 semantics.
//!
//! Locks the combined settings file contract from outside the crate:
//! atomic write (temp + rename, no litter) + `version=1` header +
//! recovery copy (`.bak`) + window/restore keys surviving as opaque lines
//! (typed after #63, `unknown` before) + corrupt-file fallback and healing.
//!
//! These tests use only the file format and the stable `ViewerSettings`
//! API (`load`/`save`/`default` plus the long-standing bool/zoom fields),
//! so they compile and pass both before and after the #63 merge. They fail
//! on `origin/main` (no version header, no `.bak`, no unknown preservation),
//! which is verified by stashing `src/settings.rs`.

use image_viewer::settings::{ViewerSettings, ZoomQuality};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

fn unique_dir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "image-viewer-settings-contract-{}-{}-{}",
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

fn cleanup(dir: &Path) {
    std::fs::remove_dir_all(dir).ok();
}

fn temp_files(dir: &Path) -> Vec<PathBuf> {
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

fn bak_path(path: &Path) -> PathBuf {
    path.with_extension("bak")
}

fn assert_versioned_once(text: &str) {
    assert!(text.starts_with("version=1\n"), "missing header: {text:?}");
    assert_eq!(
        text.matches("version=1\n").count(),
        1,
        "version header duplicated: {text:?}"
    );
}

#[test]
fn atomic_save_emits_version_header_and_no_temp_litter() {
    let dir = unique_dir("atomic");
    let path = dir.join("settings.txt");

    // First save: no previous file, so no recovery copy yet.
    ViewerSettings::default().save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_versioned_once(&text);
    assert!(
        temp_files(&dir).is_empty(),
        "temp litter after first save: {:?}",
        temp_files(&dir)
    );
    assert!(
        !bak_path(&path).exists(),
        "first save must not create a .bak"
    );

    // Second save with different values: recovery copy holds the pre-save file.
    let changed = ViewerSettings {
        wheel_navigation: false,
        hover_pan: false,
        zoom_quality: ZoomQuality::Standard,
        ..ViewerSettings::default()
    };
    changed.save(&path).unwrap();
    let text2 = std::fs::read_to_string(&path).unwrap();
    assert_versioned_once(&text2);
    assert!(temp_files(&dir).is_empty(), "temp litter after save");
    assert!(bak_path(&path).is_file(), "second save must leave a .bak");
    assert_eq!(
        ViewerSettings::load(&bak_path(&path)),
        ViewerSettings::default()
    );
    assert_eq!(ViewerSettings::load(&path), changed);

    // Idempotence: saving the same settings twice changes no bytes.
    let before = std::fs::read(&path).unwrap();
    changed.save(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_versioned_once(&std::fs::read_to_string(&path).unwrap());

    cleanup(&dir);
}

#[test]
fn recovery_copy_roundtrip_and_manual_restore() {
    let dir = unique_dir("recovery");
    let path = dir.join("settings.txt");
    let bak = bak_path(&path);

    let first = ViewerSettings {
        wheel_navigation: false,
        hold_to_peek: false,
        hover_pan: false,
        auto_hide_toolbar: false,
        zoom_quality: ZoomQuality::Standard,
        ..ViewerSettings::default()
    };
    first.save(&path).unwrap();
    ViewerSettings::default().save(&path).unwrap();

    assert!(bak.is_file());
    assert_eq!(ViewerSettings::load(&bak), first);
    assert_eq!(ViewerSettings::load(&path), ViewerSettings::default());
    assert_versioned_once(&std::fs::read_to_string(&path).unwrap());
    assert_versioned_once(&std::fs::read_to_string(&bak).unwrap());

    // Manual recovery: copying the .bak back restores the previous settings.
    std::fs::copy(&bak, &path).unwrap();
    assert_eq!(ViewerSettings::load(&path), first);

    cleanup(&dir);
}

#[test]
fn restore_opt_out_bridge_survives_save() {
    // File shape as written after #63 (typed window fields) and as preserved
    // before #63 (opaque `unknown` lines). Either way a load-save cycle must
    // keep every line exactly once and reach a fixed point. `restore_window`
    // off still keeps the geometry in the file (the viewport ignores it at
    // startup, but the file must not lose it).
    let dir = unique_dir("restore-bridge");
    let path = dir.join("settings.txt");
    std::fs::write(
        &path,
        "version=1\nwheel_navigation=false\nwindow_width=1280\nwindow_height=800\nwindow_x=100\nwindow_y=-20\nwindow_maximized=true\nrestore_window=false\nfuture_option=123\n",
    )
    .unwrap();

    ViewerSettings::load(&path).save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_versioned_once(&text);
    for line in [
        "wheel_navigation=false\n",
        "window_width=1280\n",
        "window_height=800\n",
        "window_x=100\n",
        "window_y=-20\n",
        "window_maximized=true\n",
        "restore_window=false\n",
        "future_option=123\n",
    ] {
        assert!(text.contains(line), "missing {line:?} in {text:?}");
        assert_eq!(
            text.matches(line).count(),
            1,
            "duplicated {line:?}: {text:?}"
        );
    }

    // Second cycle is a fixed point: no duplication drift.
    let once = std::fs::read(&path).unwrap();
    ViewerSettings::load(&path).save(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), once);
    assert!(temp_files(&dir).is_empty(), "temp litter after bridge save");

    cleanup(&dir);
}

#[test]
fn corrupt_settings_fallback_and_heal() {
    let dir = unique_dir("corrupt");

    // Missing file falls back to defaults.
    let missing = dir.join("missing-settings.txt");
    assert_eq!(ViewerSettings::load(&missing), ViewerSettings::default());

    // Empty file falls back to defaults.
    let empty = dir.join("empty.txt");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(ViewerSettings::load(&empty), ViewerSettings::default());

    // Lines without `key=value` are ignored, so pure garbage is defaults.
    let garbage = dir.join("garbage.txt");
    std::fs::write(&garbage, b"not an image at all\n???\n").unwrap();
    assert_eq!(ViewerSettings::load(&garbage), ViewerSettings::default());

    // Invalid UTF-8 is unreadable and falls back to defaults (never panics).
    let invalid = dir.join("invalid.txt");
    std::fs::write(&invalid, [0xff, 0xfe, 0x00, b'a', b'=']).unwrap();
    assert_eq!(ViewerSettings::load(&invalid), ViewerSettings::default());

    // Partial file: known good lines apply, the rest stays default.
    let partial = dir.join("partial.txt");
    std::fs::write(&partial, "version=1\nwheel_navigation=false\n").unwrap();
    let loaded = ViewerSettings::load(&partial);
    assert!(!loaded.wheel_navigation);
    assert!(loaded.hold_to_peek && loaded.hover_pan && loaded.auto_hide_toolbar);
    assert_eq!(loaded.zoom_quality, ZoomQuality::High);

    // Healing: saving over a corrupt file produces a valid versioned file
    // with no temp litter, and the healed file reloads to what was saved.
    let heal = dir.join("heal.txt");
    std::fs::write(&heal, b"\xff\xfe truncated \nwheel_navigation=").unwrap();
    assert_eq!(ViewerSettings::load(&heal), ViewerSettings::default());
    let changed = ViewerSettings {
        wheel_navigation: false,
        zoom_quality: ZoomQuality::Sharp,
        ..ViewerSettings::default()
    };
    changed.save(&heal).unwrap();
    let healed = std::fs::read_to_string(&heal).unwrap();
    assert_versioned_once(&healed);
    assert_eq!(ViewerSettings::load(&heal), changed);
    assert!(temp_files(&dir).is_empty(), "temp litter after heal");

    cleanup(&dir);
}

#[test]
fn concurrent_saves_leave_valid_versioned_file() {
    let dir = unique_dir("concurrent");
    let path = dir.join("settings.txt");

    // Overlapping saves from several threads must all succeed (per-save
    // unique temp names) and the final file must be a single valid
    // versioned document with no temp litter.
    std::thread::scope(|scope| {
        for thread in 0..8 {
            let path = &path;
            scope.spawn(move || {
                for round in 0..20 {
                    let settings = ViewerSettings {
                        wheel_navigation: (thread + round) % 2 == 0,
                        hold_to_peek: thread % 2 == 0,
                        zoom_quality: if round % 2 == 0 {
                            ZoomQuality::Standard
                        } else {
                            ZoomQuality::High
                        },
                        ..ViewerSettings::default()
                    };
                    settings.save(path).unwrap();
                }
            });
        }
    });

    let text = std::fs::read_to_string(&path).unwrap();
    assert_versioned_once(&text);
    // The file must parse (never half-written) and be a fixed point.
    let loaded = ViewerSettings::load(&path);
    loaded.save(&path).unwrap();
    let reloaded = std::fs::read_to_string(&path).unwrap();
    assert_versioned_once(&reloaded);
    assert_eq!(ViewerSettings::load(&path), loaded);
    assert!(
        temp_files(&dir).is_empty(),
        "temp litter after concurrent saves: {:?}",
        temp_files(&dir)
    );
    // A recovery copy exists after more than one save and is itself versioned.
    let bak = bak_path(&path);
    if bak.is_file() {
        assert_versioned_once(&std::fs::read_to_string(&bak).unwrap());
    }

    cleanup(&dir);
}
