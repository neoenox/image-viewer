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
data directory, preserving the user's real file associations:

```powershell
powershell -NoProfile -File tests/association-scripts.ps1
```

## Viewing

- Wheel: previous/next image; Ctrl+wheel: zoom at the pointer.
- Hold middle mouse: temporarily inspect at original size, release to return.
- Mouse position: pan across the image; disable this in Settings to use dragging.
- Move to the top edge to reveal controls. Tab pins/unpins the toolbar.
- T opens the thumbnail list. Click a thumbnail or filename to select it.
- Arrow keys: previous/next; 0: fit; R / Shift+R: rotate; F: fullscreen; Space: slideshow.
- Settings apply for the current session.

Loading, preloading, and original-size region decoding run in background workers.
Rapid navigation replaces pending selection work and rejects obsolete results.
The main preview cache is limited to 64 MiB; thumbnails have a separate cache
with the same ceiling and only retain visible rows. Preview resolution is capped
at 2048 pixels, with visible original-resolution regions loaded on demand.
GIF playback uses a two-frame queue and one displayed texture, with frames capped
at 1024 pixels. Frame counts are learned after the first loop; long animations
are never truncated at 200 frames. As in the previous viewer, GIFs loop continuously.

Decoding rejects images whose estimated expansion exceeds the 256 MiB per-job
limit. This is not a process-wide memory ceiling: decoder working memory, display
textures, GIF decoding, and thumbnail decoding also use memory. Region decoding
still temporarily expands the source image because the generic decoder does not
support reading only a rectangle. An in-flight decode finishes before cancellation
is observed; pending current-image work takes precedence over detail and preloads.

## Source layout

- `files.rs`: supported formats, folder scanning, natural ordering.
- `imaging.rs`: bounded decoding, resizing, rotation.
- `loader.rs`: priority scheduling, cache, original-resolution regions.
- `animation.rs`: bounded GIF streaming and cancellation.
- `app.rs`: viewer state and completed-result application.
- `input.rs`, `settings.rs`, `view.rs`: controls, settings, and rendering.
- `assoc.rs`: Windows file association registration and restoration.

The implementation plan and acceptance criteria are in
[docs/responsive-viewer-plan.md](docs/responsive-viewer-plan.md).
