#Requires -Version 5.1
<#
.SYNOPSIS
  画像ビューワーを現在のユーザーにインストールします（管理者権限不要）。
.DESCRIPTION
  - exeを %LOCALAPPDATA%\Programs\ImageViewer へコピーします
  - コピー直後に --check-update を実行し、更新をサイレントに確認・ステージします
    適用は次回起動時に行われ、インストール中の exe は置き換わりません
  - スタートメニューショートカット、ファイル関連付け、アンインストール情報を登録します
.PARAMETER ExePath
  インストール元のexe。省略時は ..\target\release\image-viewer.exe
.PARAMETER InstallDir
  インストール先。省略時は %LOCALAPPDATA%\Programs\ImageViewer
.PARAMETER SkipAssoc, SkipShortcut, SkipUpdateCheck
  テスト用のスキップ指定
#>
param(
    [string]$ExePath = "",
    [string]$InstallDir = "",
    [switch]$SkipAssoc,
    [switch]$SkipShortcut,
    [switch]$SkipUpdateCheck,
    [string]$ScriptDir = ""
)
$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($ScriptDir)) {
    $ScriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
}

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot "..\target\release\image-viewer.exe"
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "exeが見つかりません: $ExePath（先に cargo build --release してください）"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $local = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $env:USERPROFILE "AppData\Local" }
    $InstallDir = Join-Path $local "Programs\ImageViewer"
}
$InstallDir = [IO.Path]::GetFullPath($InstallDir)
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$target = Join-Path $InstallDir "image-viewer.exe"
Copy-Item -LiteralPath $ExePath -Destination $target -Force

# サイレント更新チェック。失敗してもインストール自体は成功とみなす。
if (-not $SkipUpdateCheck) {
    & $target --check-update | Out-Null
}

if (-not $SkipShortcut) {
    $programs = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
    if ($programs) {
        $lnk = Join-Path $programs "画像ビューワー.lnk"
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($lnk)
        $shortcut.TargetPath = $target
        $shortcut.WorkingDirectory = $InstallDir
        $shortcut.Description = "画像ビューワー"
        $shortcut.Save()
    }
}

if (-not $SkipAssoc) {
    & (Join-Path $ScriptDir "associate.ps1") -ExePath $target
}

$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageViewer"
New-Item -Path $uninstallKey -Force | Out-Null
Set-ItemProperty -LiteralPath $uninstallKey -Name "DisplayName" -Value "画像ビューワー"
Set-ItemProperty -LiteralPath $uninstallKey -Name "DisplayVersion" -Value (Get-Item -LiteralPath $target).VersionInfo.ProductVersion
Set-ItemProperty -LiteralPath $uninstallKey -Name "InstallLocation" -Value $InstallDir
Set-ItemProperty -LiteralPath $uninstallKey -Name "UninstallString" -Value ('powershell.exe -NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $ScriptDir "uninstall.ps1") + '"')

Write-Output "インストール完了: $target"
