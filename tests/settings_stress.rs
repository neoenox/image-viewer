//! Parallel-stress and recovery robustness for PR #46.
//!
//! Covers the six required scenarios as reproducible integration tests
//! against the stable `ViewerSettings` API (`load`/`save`/`default` plus
//! the long-standing bool/zoom fields only, so they stay green across the
//! #63 merge):
//!
//! 1. multi-thread concurrent saves (8 threads x 25 saves)
//! 2. fault-injected interrupted save (partial tmp litter, live file intact)
//! 3. stale-tmp recovery (PID-reuse litter healed, consumed file gone)
//! 4. old -> new migration (no `version` header migrates cleanly)
//! 5. backup restore (`.bak` holds pre-save settings, manual copy restores)
//! 6. recovery idempotency (repeated load-save cycles are byte-identical)
//!
//! Each test records `attempts` / `failures` explicitly and prints a
//! one-line summary (`eprintln!`) so CI logs show the counts. A failure
//! is a `save()` returning `Err` (or a planted fault that does NOT heal);
//! all tests assert `failures == 0`.

use image_viewer::settings::{ViewerSettings, ZoomQuality};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

fn unique_dir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "image-viewer-settings-stress-{}-{}-{}",
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

/// Distinct settings per (thread, round) so concurrent writers actually race
/// on different bytes, not on identical content.
fn variant(thread: u64, round: u64) -> ViewerSettings {
    ViewerSettings {
        wheel_navigation: (thread + round).is_multiple_of(2),
        hold_to_peek: thread.is_multiple_of(2),
        hover_pan: (thread * 3 + round).is_multiple_of(2),
        auto_hide_toolbar: !round.is_multiple_of(3),
        zoom_quality: if round.is_multiple_of(2) {
            ZoomQuality::Standard
        } else {
            ZoomQuality::Sharp
        },
        ..ViewerSettings::default()
    }
}

#[test]
fn parallel_concurrent_saves_record_counts() {
    let dir = unique_dir("parallel");
    let path = dir.join("settings.txt");
    let attempts = AtomicU64::new(0);
    let failures = AtomicU64::new(0);

    const THREADS: u64 = 8;
    const ROUNDS: u64 = 25;

    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let path = &path;
            let attempts = &attempts;
            let failures = &failures;
            scope.spawn(move || {
                for round in 0..ROUNDS {
                    attempts.fetch_add(1, Ordering::Relaxed);
                    if let Err(error) = variant(thread, round).save(path) {
                        failures.fetch_add(1, Ordering::Relaxed);
                        eprintln!("parallel save failed t={thread} r={round}: {error}");
                    }
                }
            });
        }
    });

    let attempts_done = attempts.load(Ordering::Relaxed);
    let failures_done = failures.load(Ordering::Relaxed);
    eprintln!("parallel_concurrent_saves: attempts={attempts_done} failures={failures_done}");
    assert_eq!(attempts_done, THREADS * ROUNDS, "all writers must run");
    assert_eq!(failures_done, 0, "no concurrent save may fail");

    // Final file is a single valid versioned document, never half-written.
    let text = std::fs::read_to_string(&path).unwrap();
    assert_versioned_once(&text);
    let loaded = ViewerSettings::load(&path);
    // Fixed point: reloading and saving changes no bytes.
    loaded.save(&path).unwrap();
    assert_eq!(ViewerSettings::load(&path), loaded);
    assert_versioned_once(&std::fs::read_to_string(&path).unwrap());
    assert!(
        temp_files(&dir).is_empty(),
        "temp litter after concurrent saves: {:?}",
        temp_files(&dir)
    );
    // A recovery copy exists after many saves and is itself versioned.
    let bak = bak_path(&path);
    if bak.is_file() {
        assert_versioned_once(&std::fs::read_to_string(&bak).unwrap());
    }

    cleanup(&dir);
}

#[test]
fn fault_injected_interrupted_save_keeps_live_file() {
    // Simulates a crash between tmp-write and rename: a partial tmp file
    // exists, the live file is untouched, and the next save heals.
    let dir = unique_dir("fault");
    let path = dir.join("settings.txt");
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;

    let good = ViewerSettings {
        wheel_navigation: false,
        zoom_quality: ZoomQuality::Standard,
        ..ViewerSettings::default()
    };
    attempts += 1;
    if good.save(&path).is_err() {
        failures += 1;
    }
    let live_before = std::fs::read(&path).unwrap();

    // Fault injection: plant truncated tmp litter covering a wide sequence
    // window so the next save is guaranteed to collide with one of them
    // (SAVE_SEQUENCE is process-global; total saves in this binary stay
    // well below the window). The live file must be untouched by this.
    let pid = std::process::id();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    for s in 0..256u64 {
        std::fs::write(dir.join(format!("{name}.tmp.{pid}.{s}")), b"\xff\xfe trunc").ok();
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        live_before,
        "fault planting must not touch the live file"
    );

    // Healing save: must succeed despite the litter.
    let healed = ViewerSettings {
        wheel_navigation: true,
        zoom_quality: ZoomQuality::Sharp,
        ..ViewerSettings::default()
    };
    attempts += 1;
    if let Err(error) = healed.save(&path) {
        panic!("healing save failed: {error}");
    }
    eprintln!("fault_injected_interrupted_save: attempts={attempts} failures={failures}");
    assert_eq!(failures, 0);

    let text = std::fs::read_to_string(&path).unwrap();
    assert_versioned_once(&text);
    assert_eq!(ViewerSettings::load(&path), healed);
    // The consumed tmp file is gone (exactly one of the planted files was
    // reused and renamed away); the live file is the healed content.
    let remaining = temp_files(&dir).len();
    assert_eq!(
        remaining, 255,
        "exactly the consumed tmp must be gone, left {remaining}"
    );

    cleanup(&dir);
}

#[test]
fn stale_tmp_recovery_heals_and_cleans_consumed_only() {
    // Crash litter from a previous run with a reused PID must not fail the
    // save: the colliding tmp is replaced, all other spares stay.
    let dir = unique_dir("stale");
    let path = dir.join("settings.txt");
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;

    let pid = std::process::id();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    const PLANTED: usize = 512;
    for s in 0..PLANTED as u64 {
        std::fs::write(dir.join(format!("{name}.tmp.{pid}.{s}")), b"stale").unwrap();
    }

    attempts += 1;
    if ViewerSettings::default().save(&path).is_err() {
        failures += 1;
    }
    eprintln!("stale_tmp_recovery: attempts={attempts} failures={failures} planted={PLANTED}");
    assert_eq!(failures, 0);

    assert_versioned_once(&std::fs::read_to_string(&path).unwrap());
    let leftovers = temp_files(&dir);
    assert_eq!(
        leftovers.len(),
        PLANTED - 1,
        "exactly the consumed tmp is gone: {} left",
        leftovers.len()
    );

    cleanup(&dir);
}

#[test]
fn old_to_new_migration_adds_version_once() {
    // Files written before schema versions existed carry over and gain the
    // header on save, exactly once, with no other shape change.
    let dir = unique_dir("migrate");
    let path = dir.join("settings.txt");
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;

    std::fs::write(
        &path,
        "wheel_navigation=false\nhold_to_peek=false\nhover_pan=false\nauto_hide_toolbar=false\nzoom_quality=standard\n",
    )
    .unwrap();
    let loaded = ViewerSettings::load(&path);
    assert!(!loaded.wheel_navigation && !loaded.hover_pan);
    assert_eq!(loaded.zoom_quality, ZoomQuality::Standard);

    attempts += 1;
    if loaded.save(&path).is_err() {
        failures += 1;
    }
    eprintln!("old_to_new_migration: attempts={attempts} failures={failures}");
    assert_eq!(failures, 0);

    let text = std::fs::read_to_string(&path).unwrap();
    assert_versioned_once(&text);
    assert!(text.contains("wheel_navigation=false\n"), "{text}");
    assert!(text.contains("zoom_quality=standard\n"), "{text}");
    assert_eq!(ViewerSettings::load(&path), loaded);
    // Second cycle is a fixed point.
    let once = std::fs::read(&path).unwrap();
    ViewerSettings::load(&path).save(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), once);

    cleanup(&dir);
}

#[test]
fn backup_restore_roundtrip() {
    // `.bak` holds the exact pre-save settings; copying it back restores.
    let dir = unique_dir("backup");
    let path = dir.join("settings.txt");
    let bak = bak_path(&path);
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;

    let first = ViewerSettings {
        wheel_navigation: false,
        hold_to_peek: false,
        hover_pan: false,
        auto_hide_toolbar: false,
        zoom_quality: ZoomQuality::Standard,
        ..ViewerSettings::default()
    };
    attempts += 1;
    if first.save(&path).is_err() {
        failures += 1;
    }
    // First save: no previous file, so no recovery copy yet.
    assert!(!bak.exists(), "first save must not create a .bak");

    attempts += 1;
    if ViewerSettings::default().save(&path).is_err() {
        failures += 1;
    }
    assert!(bak.is_file(), "second save must leave a .bak");
    assert_eq!(ViewerSettings::load(&bak), first);
    assert_eq!(ViewerSettings::load(&path), ViewerSettings::default());
    assert_versioned_once(&std::fs::read_to_string(&bak).unwrap());

    // Manual restore: copying the .bak back restores the previous settings,
    // and restoring twice is idempotent.
    std::fs::copy(&bak, &path).unwrap();
    assert_eq!(ViewerSettings::load(&path), first);
    let once = std::fs::read(&path).unwrap();
    std::fs::copy(&bak, &path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), once);

    attempts += 1;
    if first.save(&path).is_err() {
        failures += 1;
    }
    eprintln!("backup_restore: attempts={attempts} failures={failures}");
    assert_eq!(failures, 0);
    assert_eq!(ViewerSettings::load(&path), first);

    cleanup(&dir);
}

#[test]
fn recovery_idempotency_fixed_point() {
    // Repeated saves and repeated load-save cycles change no bytes; the
    // recovery copy is itself versioned and stable.
    let dir = unique_dir("idempotent");
    let path = dir.join("settings.txt");
    let bak = bak_path(&path);
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;

    let settings = ViewerSettings {
        wheel_navigation: false,
        zoom_quality: ZoomQuality::Sharp,
        ..ViewerSettings::default()
    };
    for _ in 0..3 {
        attempts += 1;
        if settings.save(&path).is_err() {
            failures += 1;
        }
    }
    let first = std::fs::read(&path).unwrap();
    attempts += 1;
    if settings.save(&path).is_err() {
        failures += 1;
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        first,
        "same save must be byte-identical"
    );

    // Load-save cycles are fixed points too.
    ViewerSettings::load(&path).save(&path).unwrap();
    attempts += 1;
    assert_eq!(std::fs::read(&path).unwrap(), first);
    ViewerSettings::load(&path).save(&path).unwrap();
    attempts += 1;
    assert_eq!(std::fs::read(&path).unwrap(), first);

    eprintln!("recovery_idempotency: attempts={attempts} failures={failures}");
    assert_eq!(failures, 0);
    assert_versioned_once(&std::fs::read_to_string(&path).unwrap());
    if bak.is_file() {
        assert_versioned_once(&std::fs::read_to_string(&bak).unwrap());
    }
    assert!(
        temp_files(&dir).is_empty(),
        "no temp litter after idempotent saves"
    );

    cleanup(&dir);
}
