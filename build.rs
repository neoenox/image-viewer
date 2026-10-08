// Embed the Windows version resource (file/product version from Cargo.toml),
// and the app icon, so the exe shows them in Explorer and its Properties.
#[cfg(windows)]
fn main() {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "Image Viewer");
        res.set("FileDescription", "Image Viewer");
        res.set_icon("assets/icon.ico");
        res.compile()
            .expect("failed to embed Windows version resource");
}

#[cfg(not(windows))]
fn main() {}
