//! Windows/non-Windows association API parity guard.
//!
//! Shared code (`view.rs`, `main.rs`) calls into `assoc` on every platform; on
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
    // Same shape as the unregister call in `view.rs` (tracks #65: `exe: &Path`).
    let _ = mirror::unregister(mirror::PROG_ID, mirror::APP_NAME, mirror::CAPS_BASE, &exe);
    let _ = mirror::open_default_apps();
    // Same shape as the `--print-assoc-config` call in `main.rs`.
    let _ = mirror::print_assoc_config();
    assert!(!mirror::is_default("jpg", mirror::PROG_ID));
    assert_eq!(mirror::exe_path().unwrap(), exe);
}

#[test]
fn mirror_sanitize_matches_windows_rules() {
    assert_eq!(mirror::sanitize_ext("jpg"), Some("jpg".to_owned()));
    assert_eq!(mirror::sanitize_ext(".JPG"), Some("jpg".to_owned()));
    assert_eq!(mirror::sanitize_ext("  tif "), Some("tif".to_owned()));
    assert_eq!(
        mirror::sanitize_ext("jpeg2000"),
        Some("jpeg2000".to_owned())
    );
    assert_eq!(mirror::sanitize_ext(""), None);
    assert_eq!(mirror::sanitize_ext("."), None);
    assert_eq!(mirror::sanitize_ext("a/b"), None);
    assert_eq!(mirror::sanitize_ext(r"a\b"), None);
    assert_eq!(mirror::sanitize_ext("jp g"), None);
    assert_eq!(mirror::sanitize_ext("jp-g"), None);
    assert_eq!(mirror::sanitize_ext("\u{e9}"), None);
    // Invalid extensions are never default, mirroring the Windows guard.
    assert!(!mirror::is_default("a/b", mirror::PROG_ID));
    assert!(!mirror::is_default("", mirror::PROG_ID));
}

#[cfg(windows)]
#[test]
fn mirror_sanitize_parity_with_windows_assoc() {
    // The mirror must stay in lockstep with the real Windows implementation.
    for ext in [
        "jpg", ".JPG", "  tif ", "jpeg2000", "", ".", "a/b", r"a\b", "jp g", "jp-g", "\u{e9}",
    ] {
        assert_eq!(
            mirror::sanitize_ext(ext),
            image_viewer::assoc::sanitize_ext(ext),
            "sanitize drift for {ext:?}"
        );
    }
}

#[test]
fn mirror_noop_reports_unsupported_without_side_effects() {
    use std::io::ErrorKind;
    let exe = std::env::current_exe().unwrap();
    // Fake backend: valid inputs reach the platform gate (Unsupported) and
    // must not create anything.
    let err = mirror::register(
        mirror::PROG_ID,
        mirror::APP_NAME,
        mirror::CAPS_BASE,
        &exe,
        &["jpg", "png"],
    )
    .expect_err("non-Windows register must fail");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let err = mirror::unregister(mirror::PROG_ID, mirror::APP_NAME, mirror::CAPS_BASE, &exe)
        .expect_err("non-Windows unregister must fail");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let err = mirror::open_default_apps().expect_err("open_default_apps must fail");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let err = mirror::print_assoc_config().expect_err("print_assoc_config must fail");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    // No-op reads.
    assert!(!mirror::is_default("jpg", mirror::PROG_ID));
    assert!(!mirror::is_default(".PNG", mirror::PROG_ID));
    assert_eq!(mirror::exe_path().unwrap(), exe);
}

#[test]
fn mirror_register_validates_before_platform_gate() {
    use std::io::ErrorKind;
    let exe = std::env::current_exe().unwrap();
    // Tracks #65: invalid extensions fail with InvalidInput without writing,
    // even on the non-Windows fake backend.
    let err = mirror::register(
        mirror::PROG_ID,
        mirror::APP_NAME,
        mirror::CAPS_BASE,
        &exe,
        &["jpg", "bogus/ext"],
    )
    .expect_err("must reject invalid extensions");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[test]
fn windows_only_deps_are_target_gated() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).expect("read Cargo.toml");
    // `winreg` / `raw-window-handle` must live under target cfg, never as
    // unconditional dependencies (issue #35).
    let windows_deps = section(&text, "[target.'cfg(windows)'.dependencies]");
    assert!(
        windows_deps.contains("winreg"),
        "winreg must be Windows-only"
    );
    assert!(
        windows_deps.contains("raw-window-handle"),
        "raw-window-handle must be Windows-only"
    );
    let top_deps = section(&text, "[dependencies]");
    assert!(
        !top_deps.contains("winreg"),
        "winreg must not be unconditional"
    );
    assert!(
        !top_deps.contains("raw-window-handle"),
        "raw-window-handle must not be unconditional"
    );
    // `winresource` must live under target build-dependencies.
    let windows_build = section(&text, "[target.'cfg(windows)'.build-dependencies]");
    assert!(
        windows_build.contains("winresource"),
        "winresource must be Windows-only build-dep"
    );
    if let Some(top_build) = try_section(&text, "[build-dependencies]") {
        assert!(
            !top_build.contains("winresource"),
            "winresource must not be unconditional build-dep"
        );
    }
}

#[test]
fn build_script_embeds_resources_on_windows_only() {
    let build_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs");
    let text = std::fs::read_to_string(&build_rs).expect("read build.rs");
    // Resource embedding must be cfg-gated so non-Windows builds never need
    // `winresource` (issue #35).
    assert!(
        text.contains("#[cfg(windows)]"),
        "build.rs must gate Windows resource embedding"
    );
    assert!(
        text.contains("#[cfg(not(windows))]"),
        "build.rs must provide a non-Windows no-op main"
    );
    assert!(
        text.contains("winresource"),
        "build.rs must still embed the Windows resource"
    );
}

fn section<'a>(toml: &'a str, header: &str) -> &'a str {
    let start = toml
        .find(header)
        .unwrap_or_else(|| panic!("missing section {header}"));
    let rest = &toml[start + header.len()..];
    let end = rest.find("\n[").map(|i| start + header.len() + i);
    match end {
        Some(end) => &toml[start..end],
        None => &toml[start..],
    }
}

fn try_section<'a>(toml: &'a str, header: &str) -> Option<&'a str> {
    let start = toml.find(header)?;
    let rest = &toml[start + header.len()..];
    let end = rest.find("\n[").map(|i| start + header.len() + i);
    match end {
        Some(end) => Some(&toml[start..end]),
        None => Some(&toml[start..]),
    }
}
