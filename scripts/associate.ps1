#Requires -Version 5.1
<#
.SYNOPSIS
  画像ビューワーを現在のユーザーの画像関連付けに登録します（管理者権限不要）。
.DESCRIPTION
  - ProgID (ImageViewer.App) を HKCU\Software\Classes に登録
  - 対応17拡張子の既定値を ProgID に向け、ExplorerのUserChoiceを削除
    （UserChoiceのハッシュは書き換えられないため削除でフォールバックさせる方式）
  - 変更前の状態は %LOCALAPPDATA%\ImageViewer\assoc-backup\backup.xml に保存
  - 戻す場合は unassociate.ps1 を実行
.PARAMETER ExePath
  関連付けるexeのパス。省略時は ..\target\release\image-viewer.exe
.EXAMPLE
  .\associate.ps1
#>
param(
    [string]$ExePath = ""
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot "..\target\release\image-viewer.exe"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "exeが見つかりません: $ExePath（先に cargo build --release してください）"
}

$exts = @("jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "ico",
          "dds", "hdr", "exr", "qoi", "pbm", "pgm", "ppm", "pam")
$progId = "ImageViewer.App"
$appName = "画像ビューワー"
$backupDir = Join-Path $env:LOCALAPPDATA "ImageViewer\assoc-backup"
New-Item -ItemType Directory -Path $backupDir -Force | Out-Null

# --- バックアップ ---
$backup = foreach ($e in $exts) {
    $cls = "HKCU:\Software\Classes\.$e"
    $uc = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.$e\UserChoice"
    $clsDef = $null
    $ucProg = $null
    if (Test-Path -LiteralPath $cls) {
        $clsDef = (Get-ItemProperty -LiteralPath $cls -Name "(default)" -ErrorAction SilentlyContinue)."(default)"
    }
    if (Test-Path -LiteralPath $uc) {
        $ucProg = (Get-ItemProperty -LiteralPath $uc -Name "ProgId" -ErrorAction SilentlyContinue).ProgId
    }
    [pscustomobject]@{ Ext = $e; ClassesDefault = $clsDef; UserChoiceProgId = $ucProg }
}
$backup | Export-Clixml -LiteralPath (Join-Path $backupDir "backup.xml")
Write-Output "backup -> $backupDir\backup.xml"

# --- ProgID登録 ---
New-Item -Path "HKCU:\Software\Classes\$progId" -Force | Out-Null
Set-ItemProperty -LiteralPath "HKCU:\Software\Classes\$progId" -Name "(default)" -Value $appName
New-Item -Path "HKCU:\Software\Classes\$progId\DefaultIcon" -Force | Out-Null
Set-ItemProperty -LiteralPath "HKCU:\Software\Classes\$progId\DefaultIcon" -Name "(default)" -Value "`"$ExePath`",0"
New-Item -Path "HKCU:\Software\Classes\$progId\shell\open\command" -Force | Out-Null
Set-ItemProperty -LiteralPath "HKCU:\Software\Classes\$progId\shell\open\command" -Name "(default)" -Value "`"$ExePath`" `"%1`""

# --- 「プログラムから開く」用エントリ ---
$appKey = "HKCU:\Software\Classes\Applications\image-viewer.exe"
New-Item -Path $appKey -Force | Out-Null
Set-ItemProperty -LiteralPath $appKey -Name "FriendlyAppName" -Value $appName
New-Item -Path "$appKey\shell\open\command" -Force | Out-Null
Set-ItemProperty -LiteralPath "$appKey\shell\open\command" -Name "(default)" -Value "`"$ExePath`" `"%1`""
New-Item -Path "$appKey\SupportedTypes" -Force | Out-Null
foreach ($e in $exts) {
    New-ItemProperty -LiteralPath "$appKey\SupportedTypes" -Name ".$e" -Value "" -Force | Out-Null
}

# --- Default Programs (設定画面) 用Capabilities ---
$cap = "HKCU:\Software\ImageViewer\Capabilities"
New-Item -Path "$cap\FileAssociations" -Force | Out-Null
Set-ItemProperty -LiteralPath $cap -Name "ApplicationName" -Value $appName
Set-ItemProperty -LiteralPath $cap -Name "ApplicationDescription" -Value $appName
foreach ($e in $exts) {
    Set-ItemProperty -LiteralPath "$cap\FileAssociations" -Name ".$e" -Value $progId
}
New-ItemProperty -LiteralPath "HKCU:\Software\RegisteredApplications" -Name $appName -Value "Software\ImageViewer\Capabilities" -Force | Out-Null

# --- 各拡張子の既定を向けてUserChoiceを外す ---
foreach ($e in $exts) {
    New-Item -Path "HKCU:\Software\Classes\.$e" -Force | Out-Null
    Set-ItemProperty -LiteralPath "HKCU:\Software\Classes\.$e" -Name "(default)" -Value $progId
    Remove-Item -LiteralPath "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.$e\UserChoice" -Force -ErrorAction SilentlyContinue
}

# --- Explorerに変更を通知 ---
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ShellNotify {
    [DllImport("shell32.dll")]
    public static extern void SHChangeNotify(int wEventId, uint uFlags, IntPtr dwItem1, IntPtr dwItem2);
}
"@
[ShellNotify]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)

Write-Output "関連付け完了: $($exts.Count)形式を $appName に登録しました"
Write-Output "ダブルクリックの既定にするには .\scripts\open-default-apps.ps1 で設定画面を開き、各形式を割り当ててください"


