# インストーラー／アンインストーラーを隔離されたレジストリと一時ディレクトリで検証する。
$ErrorActionPreference = 'Stop'
$id = [guid]::NewGuid().ToString('N')
$registryRoot = "HKCU:\Software\ImageViewerInstallerReview-$id"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) "ImageViewerInstallerReview-$id"
$installDir = Join-Path $tempRoot 'Installed'
$savedLocalAppData = $env:LOCALAPPDATA
$realKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageViewer'
$realKeyExisted = Test-Path -LiteralPath $realKey
function Assert-True($condition, $message) {
    if (-not $condition) { throw $message }
}
try {
    New-Item -Path $registryRoot | Out-Null
    New-PSDrive -Name IVReview -PSProvider Registry -Root $registryRoot | Out-Null
    New-Item -Path "$registryRoot\Software\Microsoft\Windows\CurrentVersion\Uninstall" -Force | Out-Null

    $installText = Get-Content (Join-Path $PSScriptRoot '..\scripts\install.ps1') -Raw
    $uninstallText = Get-Content (Join-Path $PSScriptRoot '..\scripts\uninstall.ps1') -Raw
    $install = [scriptblock]::Create($installText.Replace('HKCU:', 'IVReview:'))
    $uninstall = [scriptblock]::Create($uninstallText.Replace('HKCU:', 'IVReview:'))
    $exe = Join-Path $PSScriptRoot '..\target\debug\image-viewer.exe'
    Assert-True (Test-Path -LiteralPath $exe -PathType Leaf) "debug exeが見つかりません: $exe"

    $scriptsDir = Join-Path $PSScriptRoot '..\scripts'
    $target = Join-Path $installDir 'image-viewer.exe'
    & $install -ExePath $exe -InstallDir $installDir -SkipAssoc -SkipShortcut -SkipUpdateCheck -ScriptDir $scriptsDir
    Assert-True (Test-Path -LiteralPath $target -PathType Leaf) 'exe was not installed'
    $keyPath = 'IVReview:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageViewer'
    Assert-True (Test-Path -LiteralPath $keyPath) 'uninstall entry was not registered'
    Assert-True ((Get-ItemProperty -LiteralPath $keyPath).DisplayName -eq '画像ビューワー') 'DisplayName is wrong'
    Assert-True ((Test-Path -LiteralPath $realKey) -eq $realKeyExisted) 'real HKCU uninstall entry was touched'

    & $uninstall -InstallDir $installDir -SkipAssoc -ScriptDir $scriptsDir
    Assert-True (-not (Test-Path -LiteralPath $target)) 'installed exe was not removed'
    Assert-True (-not (Test-Path -LiteralPath $keyPath)) 'uninstall entry was not removed'
    Assert-True ((Test-Path -LiteralPath $realKey) -eq $realKeyExisted) 'real HKCU uninstall entry was touched'
    Write-Output 'PASS: per-user install, silent-update hook and clean uninstall'
} finally {
    $env:LOCALAPPDATA = $savedLocalAppData
    Remove-PSDrive -Name IVReview -ErrorAction SilentlyContinue
    if ($registryRoot -match '^HKCU:\\Software\\ImageViewerInstallerReview-[a-f0-9]{32}$') {
        Remove-Item -LiteralPath $registryRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    $resolvedTemp = [IO.Path]::GetFullPath($tempRoot)
    if ($resolvedTemp.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($resolvedTemp) -eq "ImageViewerInstallerReview-$id") {
        Remove-Item -LiteralPath $resolvedTemp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
