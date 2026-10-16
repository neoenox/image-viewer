# CI guard: forbid writes to production registry keys in tests.
#
# Production keys (real user settings) live only in:
# - src/assoc.rs constants (PROG_ID/CAPS_BASE/APP_NAME, read by --print-assoc-config)
# - scripts/*.ps1 via --print-assoc-config (redirected to IVReview: in tests)
#
# tests/*.rs and tests/association-scripts.ps1 must use ONLY isolated HKCU
# test keys (ImageViewer*Test*, *Cycle*, *E2E*, ImageViewerReview-<guid>) and
# must always clean up. This script fails CI on any violation.
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$failures = @()

function Add-Failure([string]$msg) {
    $script:failures += $msg
    Write-Output "GUARD-FAIL: $msg"
}

# 1. tests/*.rs must not contain production literals (comments ignored).
$testRs = Get-ChildItem (Join-Path $repoRoot 'tests\*.rs') -ErrorAction SilentlyContinue
foreach ($file in $testRs) {
    $lines = Get-Content -LiteralPath $file.FullName
    # Strip Rust line comments (//, ///, //!) so doc mentions don't trip the guard.
    $codeLines = $lines | Where-Object { $_.TrimStart() -notmatch '^//' }
    $text = $codeLines -join "`n"
    # Real ProgID literal (test ProgIDs always contain Test/Cycle/E2E).
    if ($text -match 'ImageViewer\.App') {
        Add-Failure "$($file.Name) contains production ProgID 'ImageViewer.App'"
    }
    # Real caps literal without a test suffix.
    if ($text -match 'Software\\ImageViewer\\Capabilities') {
        Add-Failure "$($file.Name) contains production caps 'Software\ImageViewer\Capabilities'"
    }
    # Real app name (Japanese production name); tests use ASCII test names.
    if ($text -match '画像ビューワー') {
        Add-Failure "$($file.Name) contains production app name"
    }
    # Writes with production constants (read-only use in print-config test is OK).
    if ($text -match 'register\s*\(\s*(crate::assoc::|assoc::|super::)?(PROG_ID|CAPS_BASE|APP_NAME)') {
        Add-Failure "$($file.Name) calls register with a production constant"
    }
    if ($text -match 'unregister\s*\(\s*(crate::assoc::|assoc::|super::)?(PROG_ID|CAPS_BASE|APP_NAME)') {
        Add-Failure "$($file.Name) calls unregister with a production constant"
    }
}

# 2. tests/association-scripts.ps1 must redirect HKCU: -> IVReview: and always clean up.
$assocScript = Join-Path $repoRoot 'tests\association-scripts.ps1'
if (Test-Path -LiteralPath $assocScript) {
    $text = Get-Content -LiteralPath $assocScript -Raw
    if ($text -notmatch 'IVReview:') {
        Add-Failure 'association-scripts.ps1 lacks IVReview: isolation redirect'
    }
    if ($text -notmatch '\.Replace\(\s*''HKCU:''\s*,\s*''IVReview:''\s*\)') {
        Add-Failure 'association-scripts.ps1 lacks HKCU: -> IVReview: rewrite'
    }
    if ($text -notmatch 'finally') {
        Add-Failure 'association-scripts.ps1 lacks finally cleanup'
    }
    if ($text -notmatch 'ImageViewerReview-\[a-f0-9\]') {
        Add-Failure 'association-scripts.ps1 lacks GUID-scoped cleanup guard'
    }
    # Must not touch the real caps base (only via $capsBase variable from config,
    # which is redirected to IVReview: at runtime).
    if ($text -match 'HKCU:\\Software\\ImageViewer\\Capabilities') {
        Add-Failure 'association-scripts.ps1 contains hardcoded production caps path'
    }
} else {
    Add-Failure 'tests/association-scripts.ps1 missing'
}

# 3. src/assoc.rs must validate exts upfront (no partial writes on bad input).
$assocRs = Join-Path $repoRoot 'src\assoc.rs'
if (Test-Path -LiteralPath $assocRs) {
    $text = Get-Content -LiteralPath $assocRs -Raw
    $sanitizePos = $text.IndexOf('sanitize_ext')
    $createPos = $text.IndexOf('create_subkey')
    if ($sanitizePos -lt 0 -or $createPos -lt 0 -or $sanitizePos -gt $createPos) {
        Add-Failure 'src/assoc.rs must validate (sanitize_ext) before create_subkey'
    }
} else {
    Add-Failure 'src/assoc.rs missing'
}

if ($failures.Count -gt 0) {
    Write-Output "FAIL: $($failures.Count) production-registry guard violation(s)"
    exit 1
}
Write-Output 'PASS: no production registry writes in tests; isolation + cleanup verified'
