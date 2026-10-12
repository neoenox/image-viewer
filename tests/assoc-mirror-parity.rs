//! Windows/non-Windows association API parity guard.
//!
//! Shared code (`view.rs`) calls into `assoc` on every platform; on
//! non-Windows that module is the mirror in `assoc_nonwindows.rs`. Every call
//! below uses the exact shapes the shared code uses, so a signature drift on
//! either side fails compilation here on all CI platforms (Windows included,
//! where this exercises only the mirror, never the registry).

#[path = "../src/assoc_nonwindows.rs"]
mod mirror;

#[test]
fn nonwindows_mirror_accepts_shared_call_shapes() {
    let exe = std::env::current_exe().unwrap();
    // Same shape as the register call in `view.rs`.
    let _ = mirror::register(
        mirror::PROG_ID,
        mirror::APP_NAME,
        mirror::CAPS_BASE,
        &exe,
        &["jpg"],
    );
    // Same shape as the unregister call in `view.rs` (exe file name).
    let name = exe
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let _ = mirror::unregister(mirror::PROG_ID, mirror::APP_NAME, mirror::CAPS_BASE, &name);
    let _ = mirror::open_default_apps();
    assert!(!mirror::is_default("jpg", mirror::PROG_ID));
    assert_eq!(mirror::exe_path().unwrap(), exe);
}
