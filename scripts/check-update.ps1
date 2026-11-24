#Requires -Version 5.1
<#
.SYNOPSIS
  古いビューワーでも最新バージョンを確認できるスタンドアロンスクリプト。
.DESCRIPTION
  GitHub ReleaseのSHA256SUMS.txtを読み、インストール済みexeのバージョンと
  比較します。ダウンロードやインストールは行いません。
.PARAMETER ExePath
  確認するexe。省略時はインストール先、次に target\release を探します。
.PARAMETER Open
  新しいバージョンがあればリリースページをブラウザで開きます。
#>
param(
    [string]$ExePath = "",
    [switch]$Open
)

$ErrorActionPreference = 'Stop'
$releasePage = 'https://github.com/neoenox/image-viewer/releases/latest'
$checksumsUrl = "$releasePage/download/SHA256SUMS.txt"

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\ImageViewer\image-viewer.exe'),
        (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\image-viewer.exe')
    )
    $ExePath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    throw "確認するexeが見つかりません。-ExePath を指定してください"
}

$current = (Get-Item -LiteralPath $ExePath).VersionInfo.ProductVersion
if ([string]::IsNullOrWhiteSpace($current)) {
    throw "exeからバージョンを取得できませんでした: $ExePath"
}

$curl = Join-Path ([Environment]::SystemDirectory) 'curl.exe'
$text = (& $curl -fsSL --max-time 20 $checksumsUrl | Out-String)
if ([string]::IsNullOrWhiteSpace($text)) {
    throw "最新のチェックサムを取得できませんでした"
}

$latest = $null
foreach ($line in ($text -split "`n")) {
    if ($line -match '^\s*[0-9a-f]{64}\s+image-viewer-v([0-9]+(?:\.[0-9]+)+)-windows-x86_64\.exe\s*$') {
        $candidate = [version]$Matches[1]
        if (-not $latest -or $candidate -gt $latest) { $latest = $candidate }
    }
}
if (-not $latest) {
    throw "チェックサムからversionを読み取れませんでした"
}

Write-Output "installed: v$current"
Write-Output "latest:    v$latest"
if ($latest -le [version]$current) {
    Write-Output '最新バージョンです。'
    exit 0
}

Write-Output "更新があります: $releasePage"
if ($Open) {
    Start-Process 'rundll32' -ArgumentList @('url.dll,FileProtocolHandler', $releasePage)
}
