<#
.SYNOPSIS
  Generates test images for the #48 (memory bounds) user acceptance procedure.
.DESCRIPTION
  - 8k.png (7680x4320 gradient, 33.2Mpx): opens normally; detail holds ~133 MiB
    transiently (just under the 128 MiB bound); 8K upscale output is refused.
  - over8k.png (8000x6000 gradient, 48Mpx): detail decode must refuse safely
    and keep a preview (w*h*4 = 192 MiB > 128 MiB bound).
  - 4k.png (3840x2160 gradient): must display and upscale normally (regression check).
  - corrupt.gif: random bytes; must show an error, never hang or crash.
  Fast GDI+ gradient fills are used (no per-pixel loops).
  Output defaults to target\acceptance (git-ignored build output area).
#>
param(
    [string]$OutDir = (Join-Path $PSScriptRoot '..\target\acceptance')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

function New-GradientPng([string]$Path, [int]$Width, [int]$Height) {
    $bitmap = New-Object System.Drawing.Bitmap($Width, $Height)
    try {
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $rect = New-Object System.Drawing.Rectangle(0, 0, $Width, $Height)
            $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
                $rect,
                [System.Drawing.Color]::FromArgb(255, 200, 30, 30),
                [System.Drawing.Color]::FromArgb(255, 30, 60, 200),
                [System.Drawing.Drawing2D.LinearGradientMode]::Horizontal)
            try {
                $graphics.FillRectangle($brush, $rect)
            } finally {
                $brush.Dispose()
            }
        } finally {
            $graphics.Dispose()
        }
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $bitmap.Dispose()
    }
    $sizeMiB = ((Get-Item -LiteralPath $Path).Length / 1MB).ToString('0.0')
    Write-Output "wrote $Path (${sizeMiB} MiB file)"
}

New-GradientPng -Path (Join-Path $OutDir '8k.png') -Width 7680 -Height 4320
New-GradientPng -Path (Join-Path $OutDir 'over8k.png') -Width 8000 -Height 6000
New-GradientPng -Path (Join-Path $OutDir '4k.png') -Width 3840 -Height 2160

$random = New-Object byte[] 4096
(New-Object System.Random(42)).NextBytes($random)
[System.IO.File]::WriteAllBytes((Join-Path $OutDir 'corrupt.gif'), $random)
Write-Output "wrote corrupt.gif (4096 random bytes)"

Write-Output 'done. See PR #48 acceptance comment for the procedure.'
