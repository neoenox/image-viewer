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

## Releasing

Pushing a `v*` tag runs `.github/workflows/release.yml`, which tests, builds, and
attaches the exe and `SHA256SUMS.txt` to a GitHub Release. To check the workflow
without publishing anything, run it manually; the exe is then kept as a workflow
artifact only:

```powershell
gh workflow run release.yml --ref main
```

## Viewing

- Wheel: previous/next image; Ctrl+wheel: zoom at the pointer.
- Hold middle mouse: temporarily inspect at original size, release to return.
- Mouse position: pan across the image; disable this in Settings to use dragging.
- Move to the top edge to reveal controls. Tab pins/unpins the toolbar.
- T opens the thumbnail list. Click a thumbnail or filename to select it.
- Arrow keys (or A / D): previous/next; Home / End: first/last; + / - (or E / Q): zoom; 0: fit;
  R / Shift+R: rotate; F / F11: fullscreen; Space: slideshow; Ctrl+O: open; Esc: leave fullscreen or stop the slideshow.
- Settings apply for the current session.

The image being opened, neighbour preloads, and original-size region decoding each
run on their own background worker, so opening an image never waits behind a preload
or a slow region decode (up to three full-size decodes can be in memory at once).
Rapid navigation replaces pending selection work and rejects obsolete results.
The main preview cache is limited to 80 MiB (room for the current image and two
neighbours on each side at the 2048 pixel cap) and evicts the least recently used
image first; thumbnails have a separate cache with the same ceiling, only retain
visible rows, and are decoded by three workers in parallel (so up to three full-size
decodes can be in memory at once while the list fills). A failed load is retried after 5 seconds, so a file that was still
being written does not stay marked as broken. Preview resolution is capped
at 2048 pixels, with visible original-resolution regions loaded on demand.
GIF playback uses a two-frame queue and one displayed texture, with frames capped
at 1024 pixels. Frame counts are learned after the first loop; long animations
are never truncated at 200 frames. As in the previous viewer, GIFs loop continuously.

Decoding rejects images whose estimated expansion exceeds the 256 MiB per-job
limit. This is not a process-wide memory ceiling: decoder working memory, display
textures, GIF decoding, and thumbnail decoding also use memory. Region decoding
expands the whole source image because the generic decoder does not support reading
only a rectangle; the expanded image (at most 128 MiB) is kept while you pan across
the same image and rotation, and dropped when you move to another file, rotate, or
the file changes on disk. An in-flight decode finishes before cancellation
is observed; pending current-image work takes precedence over preloads.

## Latency benchmark

An ignored test measures how long the background workers take to produce an image:
cold and preloaded loads, navigation while a large region decode is running, panning
at original size, a page of thumbnails, and the first frame of a GIF. It covers the
workers only, not rendering or texture upload:

```powershell
cargo test --release loader::bench -- --ignored --nocapture --test-threads=1
```

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
