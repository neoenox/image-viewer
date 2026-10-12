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
    // Same shape as the unregister call in `view.rs` (exe path).
    let _ = mirror::unregister(mirror::PROG_ID, mirror::APP_NAME, mirror::CAPS_BASE, &exe);
    let _ = mirror::open_default_apps();
    // Same shape as the config call in `main.rs`.
    let _ = mirror::print_assoc_config();
    assert!(!mirror::is_default("jpg", mirror::PROG_ID));
    assert_eq!(mirror::exe_path().unwrap(), exe);
}
