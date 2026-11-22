// Embed the Windows version resource (file/product version from Cargo.toml),
// and the app icon, so the exe shows them in Explorer and its Properties.
fn main() {
    // Build scripts are compiled for the host, so a `cfg(windows)` gate here would
    // describe the host and drop the resource when cross-compiling. Cargo
    // passes the *target* OS as an environment variable instead.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set("ProductName", "Image Viewer");
    res.set("FileDescription", "Image Viewer");
    res.set_icon("assets/icon.ico");
    res.compile()
        .expect("failed to embed Windows version resource");
}
