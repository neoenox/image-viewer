#Requires -Version 5.1
<#
.SYNOPSIS
  画像ビューワーをアンインストールします（管理者権限不要）。
.DESCRIPTION
  - ファイル関連付けを解除し、スタートメニューのショートカットを削除します
  - インストール先とアンインストール情報を削除します
.PARAMETER InstallDir
  インストール先。省略時は %LOCALAPPDATA%\Programs\ImageViewer
.PARAMETER SkipAssoc
  テスト用の関連付け解除スキップ
#>
param(
    [string]$InstallDir = "",
    [switch]$SkipAssoc,
    [string]$ScriptDir = ""
)
$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($ScriptDir)) {
    $ScriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
}

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $local = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $env:USERPROFILE "AppData\Local" }
    $InstallDir = Join-Path $local "Programs\ImageViewer"
}
$InstallDir = [IO.Path]::GetFullPath($InstallDir)
$target = Join-Path $InstallDir "image-viewer.exe"

if (-not $SkipAssoc) {
    $unassociate = Join-Path $ScriptDir "unassociate.ps1"
    if (Test-Path -LiteralPath $target -PathType Leaf) {
        & $unassociate -ExePath $target
    } else {
        # exeが無くても assoc.rs と同じ定数で解除できる。
        & $unassociate
    }
}

$programs = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
if ($programs) {
    $lnk = Join-Path $programs "画像ビューワー.lnk"
    if (Test-Path -LiteralPath $lnk) {
        Remove-Item -LiteralPath $lnk -Force
    }
}

Remove-Item -LiteralPath $InstallDir -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageViewer" -Recurse -Force -ErrorAction SilentlyContinue

Write-Output "アンインストール完了"
