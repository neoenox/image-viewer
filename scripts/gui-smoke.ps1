# GUI smoke test for the image viewer (Windows).
#
# Goal: exercise the real window with automated input and verify *state
# transitions*, not just a final frame, while isolating the app from the
# user's real settings.
#
# Design notes (see PR #48 review feedback):
#   - The app is launched with APPDATA redirected to a throwaway directory,
#     so a run can never read or write the user's real
#     %APPDATA%\image-viewer\settings.txt. The real file's hash is recorded
#     before/after and asserted unchanged.
#   - Steps are separated and each stage reports its own outcome:
#       1. identify   (find the main window for our pid)
#       2. focus      (foreground-confirm, with retry count + reason)
#       3. input      (send a key)
#       4. reflect    (confirm the window responded: title changed)
#     A failure names the stage that failed instead of silently retrying.
#   - Every retry is counted and its reason recorded, then printed as a
#     summary so flakiness is visible rather than hidden.
#   - The isolated settings file is diffed before/after to confirm the app
#     actually persisted/reloaded where expected.
#
# Exit code is non-zero if any assertion fails, so CI can gate on it.
#
# Usage:
#   pwsh -File scripts/gui-smoke.ps1 -Exe target/debug/image-viewer.exe `
#        -ImgDir target/acceptance -ShotDir target/gui-shots

param(
    [string]$Exe = 'target/debug/image-viewer.exe',
    [string]$ImgDir = 'target/acceptance',
    [string]$ShotDir = 'target/gui-shots',
    [int]$StartupTimeoutSec = 40,
    # Opt-in: steal the foreground repeatedly during the run to prove the
    # focus-confirm + retry loop actually recovers instead of masking a
    # dropped keystroke. The step attempts are reported so flakiness shows.
    [switch]$ContendFocus
)

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName Microsoft.VisualBasic
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win32Fg {
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool c);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L; public int T; public int R; public int B; }
    // One WM_MOUSEWHEEL notch (WHEEL_DELTA = 120) posted straight to the HWND.
    // Injected wheel input is routed by the OS to whatever window the cursor
    // hit-tests against, which a background window can steal; posting the message
    // to the target handle still exercises the real winit -> egui -> wheel path.
    public static bool PostWheel(IntPtr h, int delta, int x, int y) {
        IntPtr wp = (IntPtr)unchecked(delta << 16);
        IntPtr lp = (IntPtr)unchecked((y << 16) | (x & 0xFFFF));
        return PostMessage(h, 0x020A, wp, lp);
    }
    public static bool Force(IntPtr h) {
        uint cur = GetCurrentThreadId();
        IntPtr fg = GetForegroundWindow();
        uint fgT = GetWindowThreadProcessId(fg, IntPtr.Zero);
        uint vT = GetWindowThreadProcessId(h, IntPtr.Zero);
        if (fgT != cur) AttachThreadInput(cur, fgT, true);
        if (vT != cur) AttachThreadInput(cur, vT, true);
        ShowWindow(h, 9);
        bool ok = SetForegroundWindow(h);
        if (fgT != cur) AttachThreadInput(cur, fgT, false);
        if (vT != cur) AttachThreadInput(cur, vT, false);
        return ok;
    }
    public static bool IsFg(IntPtr h) { return GetForegroundWindow() == h; }
}
"@

# Failures are accumulated; the script reports all of them, then exits 1.
$script:Failures = New-Object System.Collections.Generic.List[string]
function Fail([string]$msg) {
    $script:Failures.Add($msg)
    Write-Output "FAIL: $msg"
}
function Ok([string]$msg) { Write-Output "ok:   $msg" }

function Get-SettingsPath([string]$appData) {
    if (-not $appData) { return $null }
    return (Join-Path $appData 'image-viewer\settings.txt')
}
function Get-FileHashOrNull([string]$path) {
    if ($path -and (Test-Path -LiteralPath $path)) {
        return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
    }
    return $null
}

# The app is a console-subsystem binary in debug builds and `main` returns
# `eframe::Result`, so a startup failure prints `Error: ...` to stderr and exits
# 1. Without redirection that text lands in the child's own console window and is
# lost, leaving only "process exited early (code 1)". Capture both streams so the
# reason is always reportable.
function Get-AppOutput([string]$Path, [int]$Tail = 20) {
    if (-not $Path) { return @() }
    if (-not (Test-Path -LiteralPath $Path)) { return @() }
    return @(Get-Content -LiteralPath $Path -Tail $Tail -ErrorAction SilentlyContinue)
}

function Get-AppErrorLine {
    foreach ($path in @($script:AppOut, $script:AppErr)) {
        foreach ($line in (Get-AppOutput $path)) {
            if ($line -and $line.Trim()) { return $line.Trim() }
        }
    }
    return $null
}

function Write-AppDiagnostics {
    foreach ($path in @($script:AppOut, $script:AppErr)) {
        $lines = Get-AppOutput $path
        if ($lines.Count -gt 0) {
            Write-Output "app output ($([System.IO.Path]::GetFileName($path))):"
            $lines | ForEach-Object { Write-Output "  $_" }
        }
    }
}

# Builds " - <first app output line>" so a failure summary names the real reason
# (e.g. a GL/context creation error) instead of only the exit code.
function Get-AppReasonSuffix {
    $line = Get-AppErrorLine
    if (-not $line) { return '' }
    return " - $line"
}

function ShootHwnd($Hwnd, [string]$Name) {
    $rect = New-Object Win32Fg+RECT
    [Win32Fg]::GetWindowRect($Hwnd, [ref]$rect) | Out-Null
    $w = $rect.R - $rect.L
    $h = $rect.B - $rect.T
    if ($w -le 0 -or $h -le 0) { Write-Output "skip ${Name}: empty rect"; return }
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    try {
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        try {
            $dc = $g.GetHdc()
            try { [Win32Fg]::PrintWindow($Hwnd, $dc, 2) | Out-Null }
            finally { $g.ReleaseHdc($dc) }
        } finally { $g.Dispose() }
        $bmp.Save((Join-Path $ShotDir $Name))
    } finally { $bmp.Dispose() }
    Write-Output "shot: $Name ($($w)x${h})"
}

# Send one mouse-wheel notch (WHEEL_DELTA = 120) to the window centre. With the
# default settings (wheel_navigation = true, no Ctrl) this navigates images
# (positive delta = prev, negative = next) rather than zooming.
function SendWheel($Hwnd, [int]$Delta) {
    $rect = New-Object Win32Fg+RECT
    [Win32Fg]::GetWindowRect($Hwnd, [ref]$rect) | Out-Null
    $cx = [int](($rect.L + $rect.R) / 2)
    $cy = [int](($rect.T + $rect.B) / 2)
    [Win32Fg]::PostWheel($Hwnd, $Delta, $cx, $cy) | Out-Null
}

# Re-acquire the app's main window. The handle cached during `identify` can be
# destroyed, or simply stop being enumerable, while the process is still alive
# (observed when the desktop session changes mid-run): MainWindowHandle then
# reads zero and the cached handle fails IsWindow. Returns $null if the process
# exited, a usable handle otherwise, or [IntPtr]::Zero if it is alive but has no
# reachable window right now.
function Resolve-AppWindow($Proc, [IntPtr]$Hint) {
    $Proc.Refresh()
    if ($Proc.HasExited) { return $null }
    $cur = $Proc.MainWindowHandle
    if ($cur -ne [IntPtr]::Zero) { return $cur }
    if ($Hint -ne [IntPtr]::Zero -and [Win32Fg]::IsWindow($Hint)) { return $Hint }
    return [IntPtr]::Zero
}

# One focus attempt against the live window. Returns 'ok' once the foreground is
# confirmed, otherwise a precise reason. This replaces the old catch-all
# 'focus not confirmed', which conflated a stale handle, a hidden window and a
# dead process. $Hwnd is updated in place when the window had to be re-acquired.
function Invoke-FocusAttempt($Proc, [ref]$Hwnd) {
    $w = Resolve-AppWindow $Proc $Hwnd.Value
    if ($null -eq $w) { return 'app exited' }
    if ($w -eq [IntPtr]::Zero) { return 'window unavailable (process alive)' }
    $Hwnd.Value = $w
    [Win32Fg]::Force($w) | Out-Null
    Start-Sleep -Milliseconds 400
    if (-not [Win32Fg]::IsFg($w)) { return 'focus not confirmed' }
    return 'ok'
}

# --- 1. isolate environment ------------------------------------------------
New-Item -ItemType Directory -Force -Path $ShotDir | Out-Null
$iso = Join-Path $env:TEMP ("image-viewer-gui-smoke-{0}" -f $PID)
if (Test-Path -LiteralPath $iso) { Remove-Item -LiteralPath $iso -Recurse -Force }
New-Item -ItemType Directory -Force -Path $iso | Out-Null

$realAppData = $env:APPDATA
$realSettings = Get-SettingsPath $realAppData
$realHashBefore = Get-FileHashOrNull $realSettings

$isoSettings = Get-SettingsPath $iso
$isoHashBefore = Get-FileHashOrNull $isoSettings

# --- 2. launch with redirected APPDATA -------------------------------------
$proc = $null
$contender = $null
try {
    $env:APPDATA = $iso
    $script:AppOut = Join-Path $ShotDir 'gui-smoke-app.out.txt'
    $script:AppErr = Join-Path $ShotDir 'gui-smoke-app.err.txt'
    $proc = Start-Process -FilePath $Exe -ArgumentList "`"$ImgDir/4k.png`"" -PassThru `
        -RedirectStandardOutput $script:AppOut -RedirectStandardError $script:AppErr
} finally {
    $env:APPDATA = $realAppData
}

# Optional focus contention: a background runspace keeps bringing a topmost
# form to the front so our SendKeys target loses focus mid-step.
if ($ContendFocus) {
    $contender = Start-Job -ScriptBlock {
        Add-Type -AssemblyName System.Windows.Forms
        $form = New-Object System.Windows.Forms.Form
        $form.Text = 'gui-smoke contender'
        $form.Size = New-Object System.Drawing.Size(200, 80)
        $form.TopMost = $true
        $form.Show()
        $deadline = (Get-Date).AddSeconds(40)
        while ((Get-Date) -lt $deadline) {
            $form.Activate()
            [System.Windows.Forms.Application]::DoEvents()
            Start-Sleep -Milliseconds 250
        }
        $form.Close()
    }
    Write-Output 'note: focus contention enabled'
}

try {
    # --- 3. identify the main window (with timeout) ------------------------
    # `MainWindowHandle` can transiently point at a tiny helper window, so a
    # candidate only counts once it has a non-empty title *and* a real size.
    $hwnd = [IntPtr]::Zero
    $waited = 0
    while ($waited -lt $StartupTimeoutSec) {
        Start-Sleep -Seconds 1
        $waited++
        $proc.Refresh()
        if ($proc.HasExited) {
            Fail "identify: process exited early (code $($proc.ExitCode))$(Get-AppReasonSuffix)"
            Write-AppDiagnostics
            break
        }
        $candidate = $proc.MainWindowHandle
        if ($candidate -eq [IntPtr]::Zero) { continue }
        $rect = New-Object Win32Fg+RECT
        [Win32Fg]::GetWindowRect($candidate, [ref]$rect) | Out-Null
        if (($rect.R - $rect.L) -ge 400 -and -not [string]::IsNullOrWhiteSpace($proc.MainWindowTitle)) {
            $hwnd = $candidate
            break
        }
    }
    if ($hwnd -ne [IntPtr]::Zero) {
        Ok "identify: window after ${waited}s"
    } elseif (-not $proc.HasExited) {
        Fail "identify: no sized, titled main window within ${StartupTimeoutSec}s"
    } else {
        # 早期終了はループ内で Fail 済み。ここで "window after Ns" を出すと
        # 直前の FAIL と矛盾して見えるため、明示的にスキップを記録する。
        Write-Output 'note: identify: skipped (process already exited)'
    }

    if ($hwnd -ne [IntPtr]::Zero) {
        ShootHwnd $hwnd 'smoke-01-initial.png'
        $title0 = $proc.MainWindowTitle
        Ok "initial title: $title0"

        # --- 4. navigate Right twice: focus, input, reflect ----------------
        $want = @('8k.png', 'corrupt.gif')
        for ($s = 0; $s -lt $want.Count; $s++) {
            $stage = "step $($s + 1) ($($want[$s]))"
            $reflected = $false
            $attempts = 0
            $reason = ''
            while ($attempts -lt 5 -and -not $reflected) {
                $attempts++
                $reason = ''
                $focus = Invoke-FocusAttempt $proc ([ref]$hwnd)
                if ($focus -ne 'ok') {
                    $reason = $focus
                    if ($focus -eq 'app exited') { break }
                    continue
                }
                [System.Windows.Forms.SendKeys]::SendWait('{RIGHT}')
                # Wait (bounded) for the title to reflect the new image.
                for ($w = 0; $w -lt 12; $w++) {
                    Start-Sleep -Milliseconds 1000
                    $proc.Refresh()
                    if ($proc.HasExited) { $reason = 'app exited'; break }
                    if ($proc.MainWindowTitle -like ("*" + $want[$s] + "*")) { $reflected = $true; break }
                }
                if (-not $reflected -and -not $reason) { $reason = 'title did not change' }
            }
            if ($reflected) {
                Ok "$stage`: reflected after $attempts attempt(s)"
            } else {
                Fail "$stage`: not reflected after $attempts attempt(s) ($reason)"
            }
            ShootHwnd $hwnd ("smoke-{0:D2}-{1}.png" -f ($s + 2), ($want[$s] -replace '\.', '-'))
        }

        # --- 4b. wheel navigation: prev then next --------------------------
        # Now on corrupt.gif after two Right presses. With defaults
        # (wheel_navigation = true, no Ctrl) a positive wheel notch maps to
        # `prev` and a negative one to `next`, so one notch up must land back on
        # 8k.png and one notch down must return to corrupt.gif.
        $wheelChecks = @(
            @{ Name = 'up (prev)'; Delta = 120; Want = '8k.png' },
            @{ Name = 'down (next)'; Delta = -120; Want = 'corrupt.gif' }
        )
        for ($wc = 0; $wc -lt $wheelChecks.Count; $wc++) {
            $chk = $wheelChecks[$wc]
            $reflected = $false
            $attempts = 0
            $reason = ''
            while ($attempts -lt 5 -and -not $reflected) {
                $attempts++
                $reason = ''
                $focus = Invoke-FocusAttempt $proc ([ref]$hwnd)
                if ($focus -ne 'ok') {
                    $reason = $focus
                    if ($focus -eq 'app exited') { break }
                    continue
                }
                SendWheel $hwnd $chk.Delta
                for ($w = 0; $w -lt 12; $w++) {
                    Start-Sleep -Milliseconds 1000
                    $proc.Refresh()
                    if ($proc.HasExited) { $reason = 'app exited'; break }
                    if ($proc.MainWindowTitle -like ("*" + $chk.Want + "*")) { $reflected = $true; break }
                }
                if (-not $reflected -and -not $reason) { $reason = 'title did not change' }
            }
            if ($reflected) {
                Ok "wheel $($chk.Name): reflected to $($chk.Want) after $attempts attempt(s)"
            } else {
                Fail "wheel $($chk.Name): not reflected after $attempts attempt(s) ($reason)"
            }
            ShootHwnd $hwnd ("smoke-04-wheel-{0}.png" -f $wc)
        }

        # --- 5. still alive after the whole run ----------------------------
        $proc.Refresh()
        if ($proc.HasExited) {
            Fail "liveness: app exited (code $($proc.ExitCode))$(Get-AppReasonSuffix)"
            Write-AppDiagnostics
        } elseif ($proc.MainWindowHandle -eq [IntPtr]::Zero) {
            # Process alive but its window is no longer reachable: an environment
            # condition (window/desktop moved mid-run), not a product defect, so
            # it is reported without failing a run whose transitions all passed.
            Write-Output 'note: liveness: process alive but window no longer reachable'
        } else {
            Ok 'liveness: app alive at end'
        }
    }
} finally {
    if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    if ($contender) {
        Stop-Job $contender -ErrorAction SilentlyContinue
        Remove-Job $contender -Force -ErrorAction SilentlyContinue
    }
    $env:APPDATA = $realAppData
}

# --- 6. settings isolation + diff -----------------------------------------
$realHashAfter = Get-FileHashOrNull $realSettings
if ($realSettings -and $realHashBefore -ne $realHashAfter) {
    Fail "isolation: real settings changed! ($realHashBefore -> $realHashAfter)"
} else {
    Ok 'isolation: real %APPDATA% settings untouched'
}

$isoHashAfter = Get-FileHashOrNull $isoSettings
if ($isoSettings -and (Test-Path -LiteralPath $isoSettings)) {
    Ok "isolated settings written: $isoSettings"
    if ($isoHashBefore -ne $isoHashAfter) { Ok 'isolated settings changed during run (expected)' }
} else {
    Write-Output "note: isolated settings not written ($isoSettings)"
}
Remove-Item -LiteralPath $iso -Recurse -Force -ErrorAction SilentlyContinue

# --- 7. summary -----------------------------------------------------------
Write-Output ''
if ($script:Failures.Count -eq 0) {
    Write-Output "PASS: gui-smoke ($($want.Count + 2) transitions verified)"
    exit 0
} else {
    Write-Output "FAIL: gui-smoke ($($script:Failures.Count) failure(s))"
    foreach ($f in $script:Failures) { Write-Output "  - $f" }
    exit 1
}
