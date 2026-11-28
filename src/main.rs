#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    image_viewer::install_panic_hook();
    let cli = image_viewer::Cli::from_env();
    match cli.command {
        image_viewer::Command::PrintAssocConfig => {
            if let Err(error) = image_viewer::assoc::print_assoc_config() {
                eprintln!("--print-assoc-config failed: {error}");
                std::process::exit(1);
            }
        }
        image_viewer::Command::CheckUpdate => image_viewer::check_updates_once(),
        image_viewer::Command::Viewer => return image_viewer::run(),
    }
    Ok(())
}
