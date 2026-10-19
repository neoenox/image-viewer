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

Release notes are generated from merged PRs and grouped by label
(`enhancement`, `bug`, `documentation`; anything else goes under "その他"), as set in
`.github/release.yml`. Bump `version` in `Cargo.toml` before tagging; it is embedded
in the exe's file properties.

## Viewing

- Wheel: previous/next image; Ctrl+wheel: zoom at the pointer.
- Hold middle mouse: temporarily magnify 2x (relative to the current view) around the
  pointer, release to return.
- Zoom quality (Settings): "High" (default) re-renders the visible part with Lanczos
  at screen resolution when magnified past the source pixels, a moment after the view
  stops moving; "Sharp" adds a light unsharp mask on top (wider the more you zoom in)
  for crisper edges; "Standard" keeps the GPU's bilinear stretch only.
- Mouse position: pan across the image; disable this in Settings to use dragging.
- Move to the top edge to reveal controls. Tab pins/unpins the toolbar.
- T opens the thumbnail list. Click a thumbnail or filename to select it.
- Arrow keys (or A / D): previous/next; Home / End: first/last; + / - (or E / Q): zoom; 0: fit;
  R / Shift+R: rotate; F / F11: fullscreen; Space: slideshow; Ctrl+O: open; Esc: leave fullscreen or stop the slideshow.
- F12 shows the display-latency overlay (see [Latency benchmark](#latency-benchmark)); it can also be
  enabled at startup with `image-viewer.exe --stats`.
- Photos open upright: the EXIF orientation that cameras and phones record (JPEG, PNG,
  WebP, TIFF) is applied when decoding, so thumbnails, zoom and R / Shift+R start from
  the upright image.
- Settings are saved to `%APPDATA%\image-viewer\settings.txt` when changed and loaded at
  startup. Delete the file to return to the defaults. The window size, position and
  maximized state are saved there too, so the viewer reopens where you left it.

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
only a rectangle (the `image` crate's JPEG decoder implements `read_image`, not a
rectangle read, and its `zune-jpeg` backend exposes no row-skip or scaled-decode
entry point). The expanded source has its own 128 MiB budget, tighter than the
per-job decode ceiling, and the cropped region is bounded by the 80 MiB preview
cache budget. Measured on a 3000x2000 image (release, one machine): the first
original-resolution region costs **77 ms**, and every later region of the same
image **1 ms**, because the expanded source is kept while you pan across the same
image and rotation and dropped when you move to another file, rotate, or the file
changes on disk.

For 8-bit sources both limits fall on the same pixel count (33.5 megapixels), so
the tighter budget only ever decides the outcome for wider samples - a 16-bit
source whose decode buffer is larger than its RGBA estimate. When the source
cannot be expanded at all, the viewer keeps the enlarged preview, says why in the
status line, and stops asking for original pixels for that file until another
image is opened: every region of such a file fails the same way, so retrying on
each 128-pixel pan step would only repeat the check and the message. An in-flight
decode finishes before cancellation is observed; pending current-image work takes
precedence over preloads.

## Latency benchmark

An ignored test measures how long the background workers take to produce an image:
cold and preloaded loads, navigation while a large region decode is running, panning
at original size, a page of thumbnails, and the first frame of a GIF. It covers the
workers only, not rendering or texture upload:

```powershell
cargo test --release loader::bench -- --ignored --nocapture --test-threads=1
```

What the window actually costs is measured in the app instead: F12 (or
`--stats`) opens an overlay with the frame time (median and p95 of the last 120
frames), the time from selecting an image to its texture being displayed
(median and latest), and the texture upload time. It measures the whole path
including rendering, which the benchmark above deliberately excludes, so the two
are not comparable numbers. Measurement only runs while the overlay is open:
with it closed the viewer takes no timestamps at all. A load that fails does not
record a sample, so a broken file cannot look like a slow one. The overlay is
unstyled on purpose - it is a measuring instrument, not a feature of the UI.

## Local verification scripts

Windows helper scripts for preflight checks, generating the acceptance images,
the GUI smoke test and file association are documented in
[scripts/README.md](scripts/README.md). The GUI smoke test also runs nightly, and
on demand, in `.github/workflows/gui-smoke.yml`; that workflow is not a
`pull_request` check, so a red run there is a triage signal rather than a merge
blocker.

## Source layout

- `files.rs`: supported formats, folder scanning, natural ordering.
- `imaging.rs`: bounded decoding, resizing, rotation.
- `loader.rs`: priority scheduling, cache, original-resolution regions.
- `animation.rs`: bounded GIF streaming and cancellation.
- `upscale.rs`: high-quality (Lanczos) magnification of the visible region.
- `app.rs`: viewer state and completed-result application.
- `input.rs`, `settings.rs`, `view.rs`: controls, settings, and rendering.
- `stats.rs`: display-latency measurement behind the F12 overlay (bounded sample window).
- `sync.rs`: lock helpers that survive a poisoned mutex; panic-safe worker decoding.
- `win_icon.rs`: window/taskbar icon applied via Win32 (keeps eframe's icon code out of the exe).
- `assoc.rs`: Windows file association registration and restoration.

The implementation plan and acceptance criteria are in
[docs/responsive-viewer-plan.md](docs/responsive-viewer-plan.md).
