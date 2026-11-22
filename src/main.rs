#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    match std::env::args_os().nth(1).as_deref() {
        Some(arg) if arg == std::ffi::OsStr::new("--print-assoc-config") => {
            if let Err(error) = image_viewer::assoc::print_assoc_config() {
                eprintln!("--print-assoc-config failed: {error}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some(arg) if arg == std::ffi::OsStr::new("--check-update") => {
            image_viewer::check_updates_once();
            return Ok(());
        }
        _ => {}
    }
    image_viewer::run()
}
