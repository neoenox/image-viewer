# image-viewer

Native Windows image viewer built with Rust + egui. Single `.exe`, no runtime required.

## Build

Requires Rust (MSVC target) + Windows 10 SDK.

```powershell
cargo build --release
```

Run tests:

```powershell
cargo test
```

Association script regression tests use a temporary registry subtree and temporary
backup directory, preserving the user's real file associations:

```powershell
powershell -NoProfile -File tests/association-scripts.ps1
```
