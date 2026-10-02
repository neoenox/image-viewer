# Run in a separate PowerShell process. All registry paths and backups are redirected.
$ErrorActionPreference = 'Stop'
$id = [guid]::NewGuid().ToString('N')
$registryRoot = "HKCU:\Software\ImageViewerReview-$id"
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) "ImageViewerReview-$id"
$savedLocalAppData = $env:LOCALAPPDATA
function Assert-True($condition, $message) {
    if (-not $condition) { throw $message }
}
try {
    New-Item -Path $registryRoot | Out-Null
    New-PSDrive -Name IVReview -PSProvider Registry -Root $registryRoot | Out-Null
    New-Item -Path 'IVReview:\Software\RegisteredApplications' -Force | Out-Null
    New-Item -Path 'IVReview:\Software\Classes\.jpg\OpenWithProgids' -Force | Out-Null
    New-ItemProperty -Path 'IVReview:\Software\Classes\.jpg\OpenWithProgids' -Name OtherApp -Value '' | Out-Null
    New-Item -Path 'IVReview:\Software\Classes\.png' -Force | Out-Null
    Set-ItemProperty -Path 'IVReview:\Software\Classes\.png' -Name '(default)' -Value 'Original.Png'
    $choice = 'IVReview:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.jpg\UserChoice'
    New-Item -Path $choice -Force | Out-Null
    Set-ItemProperty -Path $choice -Name ProgId -Value 'Original.Jpg'
    Set-ItemProperty -Path $choice -Name Hash -Value 'OriginalHash'
    function Assert-AssociationsPreserved {
        $jpgChoice = Get-ItemProperty -LiteralPath $choice
        Assert-True ($jpgChoice.ProgId -eq 'Original.Jpg' -and $jpgChoice.Hash -eq 'OriginalHash') 'UserChoice was modified'
        $png = Get-ItemProperty 'IVReview:\Software\Classes\.png'
        Assert-True ($png.'(default)' -eq 'Original.Png') 'Original Classes default was modified'
        $jpg = Get-ItemProperty 'IVReview:\Software\Classes\.jpg'
        Assert-True ($jpg.PSObject.Properties.Name -notcontains '(default)') 'Absent Classes default was modified'
        Assert-True (-not (Test-Path 'IVReview:\Software\Classes\.gif')) 'An absent extension key was created'
    }
    $env:LOCALAPPDATA = $tempRoot
    $associate = [scriptblock]::Create((Get-Content (Join-Path $PSScriptRoot '..\scripts\associate.ps1') -Raw).Replace('HKCU:', 'IVReview:'))
    $unassociate = [scriptblock]::Create((Get-Content (Join-Path $PSScriptRoot '..\scripts\unassociate.ps1') -Raw).Replace('HKCU:', 'IVReview:'))
    $exe = (Get-Process -Id $PID).Path
    & $associate -ExePath $exe
    Assert-AssociationsPreserved
    & $associate -ExePath $exe
    Assert-AssociationsPreserved
    & $unassociate
    Assert-AssociationsPreserved
    Assert-True (Test-Path 'IVReview:\Software\Classes\.jpg\OpenWithProgids') 'Other-app subkey was deleted'
    $other = Get-ItemProperty 'IVReview:\Software\Classes\.jpg\OpenWithProgids'
    Assert-True ($other.PSObject.Properties.Name -contains 'OtherApp') 'Other-app value was deleted'
    $jpg = Get-ItemProperty 'IVReview:\Software\Classes\.jpg'
    Assert-True ($jpg.PSObject.Properties.Name -notcontains '(default)') 'Viewer default was not removed'
    $png = Get-ItemProperty 'IVReview:\Software\Classes\.png'
    Assert-True ($png.'(default)' -eq 'Original.Png') 'Original default was not restored'
    Write-Output 'PASS: repeated registration, restoration, and preservation of other-app settings'
} finally {
    $env:LOCALAPPDATA = $savedLocalAppData
    Remove-PSDrive -Name IVReview -ErrorAction SilentlyContinue
    if ($registryRoot -match '^HKCU:\\Software\\ImageViewerReview-[a-f0-9]{32}$') {
        Remove-Item -LiteralPath $registryRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    $resolvedTemp = [IO.Path]::GetFullPath($tempRoot)
    if ($resolvedTemp.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($resolvedTemp) -eq "ImageViewerReview-$id") {
        Remove-Item -LiteralPath $resolvedTemp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
