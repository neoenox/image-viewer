#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    // Association constants for the PS wrappers; exits without a window.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--print-assoc-config")) {
        if let Err(error) = image_viewer::assoc::print_assoc_config() {
            eprintln!("--print-assoc-config failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    image_viewer::run()
}
