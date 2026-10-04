// Embed the Windows version resource (file/product version from Cargo.toml),
// so the exe's Properties > Details shows which release it is.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "Image Viewer");
        res.set("FileDescription", "Image Viewer");
        res.compile()
            .expect("failed to embed Windows version resource");
    }
}
