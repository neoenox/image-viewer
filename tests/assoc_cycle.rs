//! Registry association integration for PR #65.
//!
//! Covers the required cycle as reproducible integration tests using only
//! isolated HKCU test keys (never production keys) with guaranteed cleanup:
//!
//! 1. register -> update -> unregister -> re-register cycle (different ext
//!    sets, `FileAssociations`/`SupportedTypes` verified at each step)
//! 2. shell-notify conditions (notify after successful register/unregister,
//!    never on validation failure; fire-and-forget, never fails/panics)
//! 3. isolation guarantee (unique `ImageViewerAssocCycle*` ProgIDs, caps and
//!    exts per test; RAII guard deletes everything on drop, even on panic)
//!
//! Safety: all keys live under `HKCU\Software\Classes\<TestProg>`,
//! `HKCU\Software\Classes\Applications\<fake-test-exe>`,
//! `HKCU\<TestCaps>` and `HKCU\Software\Classes\.<test-ext>` with
//! pid+seq-unique names. Production keys (`ImageViewer.App`,
//! `Software\ImageViewer\Capabilities`, real exts) never appear here -
//! enforced by `tests/guard-registry-keys.ps1` in CI.
//!
//! RESIDUAL RISK: `SHChangeNotify` is fire-and-forget (no return, infallible);
//! real Explorer refresh still needs a real Windows shell.

// The registry cycle is inherently Windows-only: `winreg` is a
// `[target.'cfg(windows)'.dependencies]` dependency (see PR #47 and issue
// #35), and the non-Windows `assoc` mirror has no registry to exercise. Gate
// the whole file so `cargo test --all-targets` stays green on macOS/Linux;
// the non-Windows API shape is guarded by `assoc-mirror-parity.rs` instead.
#![cfg(windows)]

use image_viewer::assoc::{is_default, register, sanitize_ext, unregister};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

/// RAII isolation guard: unique test ProgID/caps/exe, deletes everything on
/// drop so a panic never leaks keys into the user's HKCU.
struct IsolatedAssoc {
    prog: String,
    app: String,
    caps: String,
    exe: PathBuf,
    ext_classes: Vec<String>,
}

impl IsolatedAssoc {
    fn new(tag: &str) -> Self {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let prog = format!("ImageViewerAssocCycle{pid}-{seq}-{tag}.App");
        let app = format!("Assoc Cycle Test {pid}-{seq}-{tag}");
        let caps = format!(r"Software\ImageViewerAssocCycle{pid}-{seq}-{tag}\Capabilities");
        // Fake exe path: unique file name per test so the
        // `Applications/<exe>` key never collides across parallel tests.
        let exe = PathBuf::from(format!(r"C:\fake\assoc-cycle-{pid}-{seq}-{tag}.exe"));
        Self {
            prog,
            app,
            caps,
            exe,
            ext_classes: Vec::new(),
        }
    }

    fn test_ext(&self, suffix: &str) -> String {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        format!("cyc{}{}{}", std::process::id() % 100000, seq % 1000, suffix)
    }

    fn track_ext_class(&mut self, ext: &str) {
        self.ext_classes.push(format!(r"Software\Classes\.{ext}"));
    }

    fn app_exe_name(&self) -> String {
        self.exe
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap()
    }

    fn assert_gone(&self) {
        let hkcu = hkcu();
        assert!(
            hkcu.open_subkey(format!(r"Software\Classes\{}", self.prog))
                .is_err(),
            "ProgID leaked: {}",
            self.prog
        );
        assert!(
            hkcu.open_subkey(&self.caps).is_err(),
            "caps leaked: {}",
            self.caps
        );
        assert!(
            hkcu.open_subkey(format!(
                r"Software\Classes\Applications\{}",
                self.app_exe_name()
            ))
            .is_err(),
            "Applications key leaked"
        );
    }
}

impl Drop for IsolatedAssoc {
    fn drop(&mut self) {
        let hkcu = hkcu();
        hkcu.delete_subkey_all(format!(r"Software\Classes\{}", self.prog))
            .ok();
        hkcu.delete_subkey_all(format!(
            r"Software\Classes\Applications\{}",
            self.app_exe_name()
        ))
        .ok();
        hkcu.delete_subkey_all(&self.caps).ok();
        if let Ok((k, _)) = hkcu.create_subkey(r"Software\RegisteredApplications") {
            k.delete_value(&self.app).ok();
        }
        for classes in &self.ext_classes {
            hkcu.delete_subkey_all(classes).ok();
        }
    }
}

fn supported_type(hkcu: &RegKey, exe_name: &str, ext: &str) -> Option<String> {
    hkcu.open_subkey(format!(
        r"Software\Classes\Applications\{exe_name}\SupportedTypes"
    ))
    .ok()?
    .get_value(format!(".{ext}"))
    .ok()
}

fn file_assoc(hkcu: &RegKey, caps: &str, ext: &str) -> Option<String> {
    hkcu.open_subkey(format!(r"{caps}\FileAssociations"))
        .ok()?
        .get_value(format!(".{ext}"))
        .ok()
}

#[test]
fn register_update_unregister_reregister_cycle() {
    let mut guard = IsolatedAssoc::new("cycle");
    let hkcu = hkcu();
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;

    let ext_a = guard.test_ext("a");
    let ext_b = guard.test_ext("b");
    assert!(sanitize_ext(&ext_a).is_some() && sanitize_ext(&ext_b).is_some());

    // A pre-existing default owned by someone else must survive everything.
    let foreign_classes = format!(r"Software\Classes\.{ext_a}");
    guard.track_ext_class(&ext_a);
    guard.track_ext_class(&ext_b);
    hkcu.delete_subkey_all(&foreign_classes).ok();
    let (key, _) = hkcu.create_subkey(&foreign_classes).unwrap();
    key.set_value("", &"Original.App").unwrap();

    // 1. Register with [ext_a].
    attempts += 1;
    if register(&guard.prog, &guard.app, &guard.caps, &guard.exe, &[&ext_a]).is_err() {
        failures += 1;
    }
    let exe_name = guard.app_exe_name();
    assert_eq!(
        supported_type(&hkcu, &exe_name, &ext_a).as_deref(),
        Some("")
    );
    assert_eq!(
        file_assoc(&hkcu, &guard.caps, &ext_a).as_deref(),
        Some(guard.prog.as_str())
    );
    // ext_b not yet registered.
    assert!(supported_type(&hkcu, &exe_name, &ext_b).is_none());
    assert!(file_assoc(&hkcu, &guard.caps, &ext_b).is_none());
    // Foreign default untouched; UserChoice never set by register.
    assert_eq!(
        hkcu.open_subkey(&foreign_classes)
            .unwrap()
            .get_value::<String, _>("")
            .unwrap(),
        "Original.App"
    );
    assert!(!is_default(&ext_a, &guard.prog));

    // 2. Update with [ext_a, ext_b] (register is additive).
    attempts += 1;
    if register(
        &guard.prog,
        &guard.app,
        &guard.caps,
        &guard.exe,
        &[&ext_a, &ext_b],
    )
    .is_err()
    {
        failures += 1;
    }
    assert_eq!(
        supported_type(&hkcu, &exe_name, &ext_a).as_deref(),
        Some("")
    );
    assert_eq!(
        supported_type(&hkcu, &exe_name, &ext_b).as_deref(),
        Some("")
    );
    assert_eq!(
        file_assoc(&hkcu, &guard.caps, &ext_a).as_deref(),
        Some(guard.prog.as_str())
    );
    assert_eq!(
        file_assoc(&hkcu, &guard.caps, &ext_b).as_deref(),
        Some(guard.prog.as_str())
    );

    // 3. Unregister: self-made keys gone, foreign default survives.
    attempts += 1;
    if unregister(&guard.prog, &guard.app, &guard.caps, &guard.exe).is_err() {
        failures += 1;
    }
    guard.assert_gone();
    assert_eq!(
        hkcu.open_subkey(&foreign_classes)
            .unwrap()
            .get_value::<String, _>("")
            .unwrap(),
        "Original.App"
    );
    assert!(!is_default(&ext_a, &guard.prog));

    // 4. Re-register with a different set [ext_b] (clean slate, not merged).
    attempts += 1;
    if register(&guard.prog, &guard.app, &guard.caps, &guard.exe, &[&ext_b]).is_err() {
        failures += 1;
    }
    assert_eq!(
        supported_type(&hkcu, &exe_name, &ext_b).as_deref(),
        Some("")
    );
    assert_eq!(
        file_assoc(&hkcu, &guard.caps, &ext_b).as_deref(),
        Some(guard.prog.as_str())
    );
    // ext_a was never re-added after the clean unregister.
    assert!(file_assoc(&hkcu, &guard.caps, &ext_a).is_none());

    eprintln!("assoc_cycle: attempts={attempts} failures={failures}");
    assert_eq!(failures, 0);

    // Explicit final cleanup (Drop also guarantees it on panic).
    unregister(&guard.prog, &guard.app, &guard.caps, &guard.exe).unwrap();
    hkcu.delete_subkey_all(&foreign_classes).ok();
    guard.assert_gone();
}

#[test]
fn shell_notify_conditions() {
    // Notify is fire-and-forget: register/unregister must at least not panic,
    // and validation failure must write nothing (hence notify nothing).
    let mut guard = IsolatedAssoc::new("notify");
    let hkcu = hkcu();
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;

    // Invalid ext: upfront validation, no writes, no notify side-effects.
    attempts += 1;
    let err = register(
        &guard.prog,
        &guard.app,
        &guard.caps,
        &guard.exe,
        &["jpg", "bogus/ext"],
    )
    .expect_err("must reject invalid extensions");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(hkcu
        .open_subkey(format!(r"Software\Classes\{}", guard.prog))
        .is_err());
    assert!(hkcu.open_subkey(&guard.caps).is_err());

    // Valid register notifies (infallible) and unregister notifies too;
    // unregister is idempotent (second call is a no-op success).
    let ext = guard.test_ext("n");
    guard.track_ext_class(&ext);
    attempts += 1;
    if register(&guard.prog, &guard.app, &guard.caps, &guard.exe, &[&ext]).is_err() {
        failures += 1;
    }
    attempts += 1;
    if unregister(&guard.prog, &guard.app, &guard.caps, &guard.exe).is_err() {
        failures += 1;
    }
    attempts += 1;
    if unregister(&guard.prog, &guard.app, &guard.caps, &guard.exe).is_err() {
        failures += 1;
    }
    guard.assert_gone();

    eprintln!("assoc_notify: attempts={attempts} failures={failures}");
    assert_eq!(failures, 0);
}
