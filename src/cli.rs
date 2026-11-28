//! Single authority for process arguments.
//!
//! `main` and the viewer previously parsed the same flags independently: the
//! binary looked only at the first argument for command flags while the app
//! scanned every argument. This module keeps that exact contract in one place.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Command {
    #[default]
    Viewer,
    PrintAssocConfig,
    CheckUpdate,
}

#[derive(Debug, Default)]
pub struct Cli {
    pub command: Command,
    pub path: Option<PathBuf>,
    pub stats: bool,
    pub no_update: bool,
}

impl Cli {
    /// Parse the current process arguments, excluding the executable name.
    pub fn from_env() -> Self {
        Self::parse(std::env::args_os().skip(1))
    }

    fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Self {
        let mut args = args.into_iter();
        let first = args.next();
        let command = match first.as_deref() {
            Some(arg) if arg == OsStr::new("--print-assoc-config") => Command::PrintAssocConfig,
            Some(arg) if arg == OsStr::new("--check-update") => Command::CheckUpdate,
            _ => Command::Viewer,
        };
        if command != Command::Viewer {
            return Self {
                command,
                ..Self::default()
            };
        }

        let mut cli = Self::default();
        for arg in first.into_iter().chain(args) {
            if arg == "--stats" {
                cli.stats = true;
                continue;
            }
            if arg == "--no-update" {
                cli.no_update = true;
                continue;
            }
            if cli.path.is_none() {
                let candidate = PathBuf::from(arg);
                if is_valid_launch_path(&candidate) {
                    cli.path = Some(candidate);
                }
            }
        }
        cli
    }
}

fn is_valid_launch_path(path: &std::path::Path) -> bool {
    path.exists() && (path.is_dir() || crate::files::is_supported(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(list: &[&str]) -> Vec<OsString> {
        list.iter().copied().map(OsString::from).collect()
    }

    #[test]
    fn viewer_flags_are_order_independent() {
        assert_eq!(Cli::parse(os_args(&[])).command, Command::Viewer);

        let dir = std::env::temp_dir().join(format!("iv-cli-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("photo.png");
        image::RgbaImage::new(2, 2).save(&png).unwrap();
        let arg = png.to_string_lossy().into_owned();

        let cli = Cli::parse(os_args(&[&arg, "--stats"]));
        assert!(cli.stats);
        assert_eq!(cli.path, Some(png.clone()));

        let cli = Cli::parse(os_args(&["--stats", &arg]));
        assert!(cli.stats);
        assert_eq!(cli.path, Some(png));

        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn only_existing_supported_paths_are_accepted() {
        let dir = std::env::temp_dir().join(format!("iv-cli-filter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("note.txt");
        let png = dir.join("sample.png");
        std::fs::write(&txt, b"not an image").unwrap();
        std::fs::write(&png, b"placeholder").unwrap();
        assert!(is_valid_launch_path(&dir));
        assert!(is_valid_launch_path(&png));
        assert!(!is_valid_launch_path(&txt));
        assert!(!is_valid_launch_path(&dir.join("missing.png")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_update_flag_is_recognised_anywhere() {
        assert!(!Cli::parse(os_args(&["photo.png"])).no_update);
        assert!(Cli::parse(os_args(&["--no-update"])).no_update);
        assert!(Cli::parse(os_args(&["photo.png", "--no-update"])).no_update);
    }

    #[test]
    fn command_flags_only_apply_to_the_first_argument() {
        assert_eq!(
            Cli::parse(os_args(&["--print-assoc-config"])).command,
            Command::PrintAssocConfig
        );
        assert_eq!(
            Cli::parse(os_args(&["--check-update", "--stats"])).command,
            Command::CheckUpdate
        );
        assert_eq!(
            Cli::parse(os_args(&["photo.png", "--print-assoc-config"])).command,
            Command::Viewer
        );
    }
}
