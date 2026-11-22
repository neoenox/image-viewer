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
    New-Item -Path "$registryRoot\Software\Classes" -Force | Out-Null
    New-Item -Path "$registryRoot\Software\RegisteredApplications" -Force | Out-Null

    $installText = Get-Content (Join-Path $PSScriptRoot '..\scripts\install.ps1') -Raw
    $uninstallText = Get-Content (Join-Path $PSScriptRoot '..\scripts\uninstall.ps1') -Raw
    $install = [scriptblock]::Create($installText.Replace('HKCU:', 'IVReview:'))
    $uninstall = [scriptblock]::Create($uninstallText.Replace('HKCU:', 'IVReview:'))
    $exe = Join-Path $PSScriptRoot '..\target\debug\image-viewer.exe'
    Assert-True (Test-Path -LiteralPath $exe -PathType Leaf) "debug exeが見つかりません: $exe"

    $configText = (& $exe --print-assoc-config) -join "`n"
    $progId = (($configText -split "`n" | Where-Object { $_ -like 'prog_id=*' } | Select-Object -First 1) -replace '^prog_id=', '')
    Assert-True (-not [string]::IsNullOrWhiteSpace($progId)) 'prog_id could not be read from exe'
    $sandboxScripts = Join-Path $tempRoot 'Scripts'
    New-Item -ItemType Directory -Force -Path $sandboxScripts | Out-Null
    foreach ($name in @('associate.ps1', 'unassociate.ps1')) {
        $text = Get-Content (Join-Path $PSScriptRoot "..\scripts\$name") -Raw
        Set-Content -LiteralPath (Join-Path $sandboxScripts $name) -Value $text.Replace('HKCU:', 'IVReview:')
    }
    $target = Join-Path $installDir 'image-viewer.exe'
    & $install -ExePath $exe -InstallDir $installDir -SkipShortcut -SkipUpdateCheck -ScriptDir $sandboxScripts
    Assert-True (Test-Path -LiteralPath $target -PathType Leaf) 'exe was not installed'
    $assocKey = "IVReview:\Software\Classes\$progId"
    Assert-True (Test-Path -LiteralPath $assocKey) 'installer did not register file associations'
    $keyPath = 'IVReview:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageViewer'
    Assert-True (Test-Path -LiteralPath $keyPath) 'uninstall entry was not registered'
    Assert-True ((Get-ItemProperty -LiteralPath $keyPath).DisplayName -eq '画像ビューワー') 'DisplayName is wrong'
    Assert-True ((Test-Path -LiteralPath $realKey) -eq $realKeyExisted) 'real HKCU uninstall entry was touched'

    & $uninstall -InstallDir $installDir -ScriptDir $sandboxScripts
    Assert-True (-not (Test-Path -LiteralPath $target)) 'installed exe was not removed'
    Assert-True ((Test-Path -LiteralPath $realKey) -eq $realKeyExisted) 'real HKCU uninstall entry was touched'
    Assert-True (-not (Test-Path -LiteralPath $assocKey)) 'uninstaller did not remove file associations'
    Write-Output 'PASS: per-user install, associations, silent-update hook and clean uninstall'
} finally {
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
