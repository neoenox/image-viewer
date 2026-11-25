#Requires -Version 5.1
param(
    [string]$Version,
    [string]$ExecutablePath,
    [string]$CompilerPath,
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot

if (-not $Version) {
    $manifest = Get-Content -LiteralPath (Join-Path $projectRoot 'Cargo.toml') -Raw
    $match = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $match.Success) { throw 'Could not read version from Cargo.toml.' }
    $Version = $match.Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must be X.Y.Z, got: $Version"
}
if (-not $ExecutablePath) {
    $ExecutablePath = Join-Path $projectRoot 'target\release\image-viewer.exe'
}
$executable = Get-Item -LiteralPath $ExecutablePath -ErrorAction Stop
if ($executable.PSIsContainer -or $executable.Length -eq 0) {
    throw 'Build image-viewer.exe before creating the installer.'
}
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $projectRoot 'dist'
}
if (-not $CompilerPath) {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) {
        $CompilerPath = $command.Source
    } else {
        $candidates = @(
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
        )
        $CompilerPath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    }
}
if (-not $CompilerPath -or -not (Test-Path -LiteralPath $CompilerPath -PathType Leaf)) {
    throw 'Inno Setup 6 is required. Set -CompilerPath to ISCC.exe.'
}

$script = Join-Path $projectRoot 'installer\image-viewer.iss'
& $CompilerPath "/DAppVersion=$Version" "/DSourceExe=$($executable.FullName)" "/DOutputDirectory=$OutputDirectory" $script
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compiler failed: $LASTEXITCODE" }
$installer = Get-Item -LiteralPath (Join-Path $OutputDirectory "image-viewer-installer-$Version.exe") -ErrorAction Stop
Write-Output "Installer: $($installer.FullName)"
