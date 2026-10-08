mod animation;
mod app;
#[cfg(windows)]
pub mod assoc;
#[cfg(not(windows))]
#[path = "assoc_nonwindows.rs"]
pub mod assoc;
mod files;
mod imaging;
mod input;
mod loader;
mod settings;
mod sync;
mod upscale;
mod view;
mod win_icon;
pub use app::{run, setup_jp_font, ViewerApp, PEEK_MAGNIFICATION, TOOLBAR_ICONS};
pub use files::{collect_siblings, first_image_in_dir, is_supported, natural_cmp, SUPPORTED_EXTS};
pub use imaging::decode_image;
pub use loader::{CachedImage, ImageLoader};
