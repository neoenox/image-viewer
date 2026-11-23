#Requires -Version 5.1
<#
.SYNOPSIS
  リリース用のインストーラーパッケージZIPを作ります。
.DESCRIPTION
  - exeとインストール/関連付けスクリプトを1つのZIPにまとめます
  - ZIPのSHA256をSHA256SUMS-installer.txtに書きます
  - workflowファイルを変更せずにリリースへ添付できます
.PARAMETER ExePath
  パッケージするexe。省略時は target\release\image-viewer.exe
.PARAMETER Version
  ファイル名に使うバージョン。省略時はexeのProductVersion
.PARAMETER OutDir
  出力先。省略時は target\package
#>
param(
    [string]$ExePath = "",
    [string]$Version = "",
    [string]$OutDir = "target\package"
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\image-viewer.exe"
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "exeが見つかりません: $ExePath"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = (Get-Item -LiteralPath $ExePath).VersionInfo.ProductVersion
}
if ([string]::IsNullOrWhiteSpace($Version)) {
    throw "バージョンを取得できませんでした。-Version を指定してください"
}

$outFull = [IO.Path]::GetFullPath($OutDir)
New-Item -ItemType Directory -Force -Path $outFull | Out-Null
$stage = Join-Path $outFull "stage-$Version"
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $stage | Out-Null

foreach ($name in @('install.ps1', 'uninstall.ps1', 'associate.ps1', 'unassociate.ps1', 'open-default-apps.ps1')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $stage $name) -Force
}
Copy-Item -LiteralPath $ExePath -Destination (Join-Path $stage 'image-viewer.exe') -Force

$instructions = @"
画像ビューワー v$Version インストール手順

1. このZIPを展開します
2. PowerShellで install.ps1 を実行します
   powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
3. 画像と動画の関連付けが登録されます。ダブルクリックの既定は
   Windowsが保護しているため、必要なら設定 > 既定のアプリで確定してください

アンインストール: uninstall.ps1 を同じ方法で実行します
"@
Set-Content -LiteralPath (Join-Path $stage 'INSTALL.txt') -Value $instructions -Encoding UTF8

$zip = Join-Path $outFull "image-viewer-installer-$Version-windows-x86_64.zip"
Remove-Item -LiteralPath $zip -Force -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
$checksum = Join-Path $outFull "SHA256SUMS-installer.txt"
Set-Content -LiteralPath $checksum -Value ("{0}  {1}" -f $hash, [IO.Path]::GetFileName($zip)) -Encoding ASCII

Write-Output "package: $zip"
Write-Output "checksum: $checksum"
