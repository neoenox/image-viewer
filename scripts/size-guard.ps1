# Binary size guard.
#
# Fails when the release executable grows beyond the committed baseline by
# more than the allowed tolerance. A silent size regression (an accidental
# debug/symbol bloat, a dependency pulling in a large unoptimized chunk) is
# easy to miss until users notice a heavier download, so gate it.
#
# Usage:
#   pwsh -File scripts/size-guard.ps1 -Exe target/release/image-viewer.exe
#
# Update the baseline deliberately when growth is intended:
#   pwsh -File scripts/size-guard.ps1 -Exe target/release/image-viewer.exe -ShowBaseline
# then edit $BaselineBytes below.

param(
    [string]$Exe = 'target/release/image-viewer.exe',
    # Measured from a clean `cargo build --release --locked` on main (2026-10).
    [long]$BaselineBytes = 12601856,
    [double]$TolerancePercent = 5,
    [switch]$ShowBaseline
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $Exe)) {
    Write-Output "FAIL: size-guard - executable not found: $Exe"
    exit 1
}

$size = (Get-Item -LiteralPath $Exe).Length
$limit = [long]([math]::Floor($BaselineBytes * (1 + $TolerancePercent / 100)))

if ($ShowBaseline) {
    Write-Output ("size bytes={0} MiB={1}" -f $size, [math]::Round($size / 1MB, 2))
    Write-Output ("baseline bytes={0} limit bytes={1}" -f $BaselineBytes, $limit)
    exit 0
}

$sizeMiB = [math]::Round($size / 1MB, 2)
$baseMiB = [math]::Round($BaselineBytes / 1MB, 2)
$limitMiB = [math]::Round($limit / 1MB, 2)
$deltaPct = if ($BaselineBytes -gt 0) {
    [math]::Round((($size - $BaselineBytes) / $BaselineBytes) * 100, 1)
} else { 0 }

if ($size -le $limit) {
    Write-Output ("ok:   size {0} MiB (baseline {1} MiB, {2:+0.0;-0.0;0}%, limit {3} MiB)" -f `
        $sizeMiB, $baseMiB, $deltaPct, $limitMiB)
    Write-Output 'PASS: size-guard'
    exit 0
} else {
    Write-Output ("FAIL: size-guard - {0} MiB exceeds baseline {1} MiB by {2}% (limit {3}% -> {4} MiB)" -f `
        $sizeMiB, $baseMiB, $deltaPct, $TolerancePercent, $limitMiB)
    Write-Output 'Update $BaselineBytes only if the growth is intended.'
    exit 1
}
