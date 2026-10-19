# scripts

Local Windows scripts for building, verifying and registering the viewer. None of
these ship in the exe; they are developer and CI helpers.

| Script | Purpose |
| --- | --- |
| `preflight.ps1` | Fail fast if the machine cannot build (missing Rust toolchain, MSVC linker or Windows SDK). Exit 0 = ready to build. |
| `new-acceptance-images.ps1` | Generate the #48 memory-bounds test images into `target\acceptance` (`8k.png`, `over8k.png`, `4k.png`, `corrupt.gif`). |
| `gui-smoke.ps1` | Drive the real window with automated input and verify state transitions. Runs nightly in `.github/workflows/gui-smoke.yml`; see below. |
| `associate.ps1` | Register the viewer for image types under `HKCU` (no admin). |
| `unassociate.ps1` | Undo `associate.ps1`. |
| `open-default-apps.ps1` | Open Settings > Default apps to finish association in the UI. Run `associate.ps1` first. |

## gui-smoke.ps1

Exercises the real window with automated input and asserts *state transitions*
rather than a single final frame, with the app isolated from the user's real
settings.

```powershell
# generate the input images first
pwsh -File scripts/new-acceptance-images.ps1

pwsh -File scripts/gui-smoke.ps1 `
  -Exe target/release/image-viewer.exe `
  -ImgDir target/acceptance -ShotDir target/gui-shots
```

What it does:

- Launches the app with `APPDATA` redirected to a throwaway directory, then
  asserts the real `%APPDATA%\image-viewer\settings.txt` hash is unchanged, so a
  run can never read or write real state.
- Identifies the main window (sized and titled), then verifies four transitions
  against the window title:
  - `RIGHT` -> `8k.png`, then `RIGHT` -> `corrupt.gif` (arrow navigation)
  - one wheel notch up -> `8k.png` (previous), one wheel notch down -> `corrupt.gif` (next)
- Wheel input is posted as `WM_MOUSEWHEEL` straight to the app's HWND, because
  injected wheel input is routed by the OS to whatever window the cursor
  hit-tests against, which a background window can steal.
- Each step retries up to 5 times, counts the attempts, and records a precise
  reason on failure (`app exited`, `window unavailable (process alive)`,
  `focus not confirmed`, `title did not change`) instead of a single catch-all.

Options:

- `-ContendFocus` repeatedly steals the foreground during the run, to prove the
  focus-confirm and retry loop actually recovers rather than masking a dropped
  keystroke.
- `-StartupTimeoutSec` bounds how long to wait for the first window (default 40).

The exit code is non-zero if any assertion fails, so CI can gate on it. On
success it prints `PASS: gui-smoke (N transitions verified)`.

### Scheduled runs

`.github/workflows/gui-smoke.yml` runs this script on a schedule (nightly,
18:17 UTC = 03:17 JST) and on demand. It is deliberately **not** a
`pull_request` check: the run drives a real window with injected input, which is
more timing- and desktop-dependent than the unit and bench gates in `ci.yml`, so
a red run there is a triage signal, not a merge blocker. The job is named
`GUI smoke - scheduled check, not a PR gate` to make that hard to misread.

```powershell
# run the same job by hand
gh workflow run gui-smoke.yml --ref main

# stress run that also steals the foreground (-ContendFocus)
gh workflow run gui-smoke.yml --ref main -f contend_focus=true

gh run list --workflow gui-smoke.yml --limit 5
gh run watch   # follow the newest run
```

The workflow builds `target/debug/image-viewer.exe`, generates the acceptance
images into `target/acceptance`, then runs the script with
`-Exe target/debug/image-viewer.exe -ImgDir target/acceptance -ShotDir target/gui-shots`.
Artifacts are uploaded even when the run fails (`if: always()`), as the
`gui-smoke-<run id>` artifact (paths relative to `target/`):

- `gui-shots/…` — the `smoke-*.png` window captures.
- `gui-shots/gui-smoke-app.out.txt`, `gui-shots/gui-smoke-app.err.txt` — the
  viewer's own stdout/stderr, captured by the script. The app is a
  console-subsystem binary in debug builds and `main` returns `eframe::Result`,
  so a startup failure (for example no usable GPU/GL context) prints
  `Error: …` there and exits 1; the script also repeats the first line in the
  `FAIL:` summary so the reason is visible without downloading anything.
- `gui-smoke.log` — the full script output, including the isolated-settings
  before/after diff (the throwaway settings directory is deleted by the script,
  so the log is the record) and, on failure, the `FAIL:` summary naming each
  failed stage and its retry reason.
- `acceptance/*.png`, `acceptance/*.gif` — the generated inputs, so a failure can
  be reproduced locally with the same images.

The run summary also repeats the `PASS:`/`FAIL:` lines and the exit code.

Requirements: an interactive Windows desktop. The script brings a window to the
foreground and captures it with `PrintWindow`, so it cannot run on a headless
runner. `windows-latest` normally provides a desktop session; the workflow
probes for it first (`Environment probe`) so that if a run does fail that way,
the log says so instead of looking like a viewer defect. This has not yet been
observed on a hosted runner.

Known environment caveat: on some desktops the app's window stops being
enumerable roughly 40-50s into a run while the process stays alive. The script
reports that precisely -- a step fails as `window unavailable (process alive)`,
or the run ends with a `note: liveness: process alive but window no longer
reachable` -- rather than mislabelling it as an app exit. If a run fails this
way, re-run it; it is an environment condition, not a viewer defect.
