# Windows build preflight diagnostic.
#
# Fails fast, with a clear reason, when the machine cannot build the app:
# missing Rust toolchain, missing MSVC linker / Windows SDK, or a cargo
# invocation that cannot find the linker. Run this before a build or in CI
# to turn a cryptic link error into an actionable message.
#
# Usage:
#   pwsh -File scripts/preflight.ps1
# Exit code 0 = ready to build, 1 = something is missing.

$ErrorActionPreference = 'Continue'

$problems = New-Object System.Collections.Generic.List[string]
$notes = New-Object System.Collections.Generic.List[string]

# Resolve an executable from PATH first, then from extra fallback dirs
# (the Rust tools are commonly installed under the user profile without
# being added to PATH for a fresh shell).
function Resolve-Tool([string]$name, [string[]]$extraDirs) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    foreach ($dir in $extraDirs) {
        if (-not $dir) { continue }
        $p = Join-Path $dir ($name + '.exe')
        if (Test-Path -LiteralPath $p) { return $p }
    }
    return $null
}

$cargoDir = Join-Path $env:USERPROFILE '.cargo\bin'
$extraDirs = @($cargoDir)

function Check([string]$name, [string]$path, [string]$version) {
    if ($path) {
        Write-Output ("ok:   {0}: {1}" -f $name, (($version | Select-Object -First 1) -join ' ').Trim())
        return
    }
    Write-Output ("MISSING: {0}" -f $name)
    $problems.Add($name)
}

Write-Output '== Rust toolchain =='
$rustc = Resolve-Tool 'rustc' $extraDirs
$cargo = Resolve-Tool 'cargo' $extraDirs
$rustup = Resolve-Tool 'rustup' $extraDirs
Check 'rustc' $rustc (& $rustc --version 2>&1) 
Check 'cargo' $cargo (& $cargo --version 2>&1) 
Check 'rustup' $rustup (& $rustup --version 2>&1) 

if ($cargo -and -not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $notes.Add("cargo is at $cargo but not on PATH; add $cargoDir to PATH for plain 'cargo' to work")
}

Write-Output ''
Write-Output '== MSVC target =='
if ($rustup) {
    $triples = (& $rustup target list --installed 2>&1) -join ' '
    if ($triples -match 'x86_64-pc-windows-msvc') {
        Write-Output 'ok:   target x86_64-pc-windows-msvc installed'
    } else {
        Write-Output 'MISSING: target x86_64-pc-windows-msvc (run: rustup target add x86_64-pc-windows-msvc)'
        $problems.Add('target x86_64-pc-windows-msvc')
    }
}

# link.exe is normally NOT on PATH; cargo locates the MSVC linker through
# vswhere / the cc crate. Probe both PATH and a Visual Studio installation.
Write-Output ''
Write-Output '== Linker (MSVC) =='
$link = Resolve-Tool 'link' @()
if ($link) {
    Write-Output ("ok:   link.exe on PATH: {0}" -f $link)
} else {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vsPath = $null
    if (Test-Path -LiteralPath $vswhere) {
        $vsPath = (& $vswhere -latest -products * `
            -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
            -property installationPath 2>&1) | Select-Object -First 1
    }
    if ($vsPath) {
        Write-Output ("ok:   MSVC build tools via vswhere: {0}" -f $vsPath.Trim())
    } else {
        Write-Output 'MISSING: MSVC linker (install Visual Studio Build Tools: "Desktop development with C++")'
        $problems.Add('MSVC linker (link.exe)')
    }
}

Write-Output ''
Write-Output '== Windows SDK =='
$sdkRoot = @(
    (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'),
    (Join-Path $env:ProgramFiles 'Windows Kits\10')
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if ($sdkRoot) {
    $libs = Join-Path $sdkRoot 'Lib'
    $versions = if (Test-Path -LiteralPath $libs) {
        (Get-ChildItem -LiteralPath $libs -Directory | Sort-Object Name -Descending |
            Select-Object -First 5 -ExpandProperty Name) -join ', '
    } else { 'no Lib dir' }
    Write-Output ("ok:   {0} (Lib: {1})" -f $sdkRoot, $versions)
} else {
    Write-Output 'MISSING: Windows 10/11 SDK (Windows Kits\10)'
    $problems.Add('Windows SDK')
}

Write-Output ''
if ($notes.Count -gt 0) {
    Write-Output 'notes:'
    foreach ($n in $notes) { Write-Output ("  - {0}" -f $n) }
    Write-Output ''
}
if ($problems.Count -eq 0) {
    Write-Output 'PASS: preflight - ready to build'
    exit 0
} else {
    Write-Output ("FAIL: preflight - {0} problem(s)" -f $problems.Count)
    foreach ($p in $problems) { Write-Output ("  - {0}" -f $p) }
    exit 1
}

