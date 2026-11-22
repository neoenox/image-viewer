#Requires -Version 5.1
<#
.SYNOPSIS
  ビューワーの登録だけを解除します。
.DESCRIPTION
  - 削除範囲は assoc.rs の unregister と同一（ProgID / Applications/<exe名> /
    Capabilities配下 / RegisteredApplications値）。他アプリの値は触りません。
  - 拡張子の既定値とUserChoiceは変更しません。
  - exeが見つからない場合（アップグレード・アンインストール時）は、既定値または
    明示指定した ProgId/AppName/CapsBase/AppExe で削除を続行します。
  - 注意: ExplorerのUserChoiceハッシュは復元できないため、
    ダブルクリックの既定が期待と違う場合はUIから選び直してください
.PARAMETER ExePath
  設定を読み出すexe。省略時は既定の定数を使用します。
#>
param(
    [string]$ExePath = "",
    [string]$ProgId = "",
    [string]$AppName = "",
    [string]$CapsBase = "",
    [string]$AppExe = ""
)

$ErrorActionPreference = "Stop"

function Get-AssocConfig([string]$Exe) {
    $previousEncoding = [Console]::OutputEncoding
    try {
        [Console]::OutputEncoding = [Text.Encoding]::UTF8
        $config = @{}
        foreach ($line in (& $Exe --print-assoc-config)) {
            if ($line -match '^([^=]+)=(.*)$') { $config[$Matches[1]] = $Matches[2] }
        }
    } finally {
        [Console]::OutputEncoding = $previousEncoding
    }
    foreach ($key in @('format', 'prog_id', 'app_name', 'caps_base', 'app_exe')) {
        if ([string]::IsNullOrWhiteSpace($config[$key])) {
            throw "関連付け設定を取得できませんでした ($key)"
        }
    }
    if ($config['format'] -ne '1') { throw "関連付け設定の形式が不明です" }
    if ($config['app_exe'] -match '[\\/]') { throw "不正なexe名です: $($config['app_exe'])" }
    return $config
}

# exe が読めるときは exe を正本にする。読めないときは assoc.rs と同じ定数を
# 既定値として使い、アンインストール時に登録が残らないようにする。
$config = $null
if (-not [string]::IsNullOrWhiteSpace($ExePath)) {
    if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
        throw "exeが見つかりません: $ExePath（定数を直接指定するか、exeを復元してください）"
    }
    $config = Get-AssocConfig -Exe (Resolve-Path -LiteralPath $ExePath).Path
}
$progId = if ($null -ne $config) { $config['prog_id'] } elseif ($ProgId) { $ProgId } else { 'ImageViewer.App' }
$appName = if ($null -ne $config) { $config['app_name'] } elseif ($AppName) { $AppName } else { '画像ビューワー' }
$capsBase = if ($null -ne $config) { $config['caps_base'] } elseif ($CapsBase) { $CapsBase } else { 'Software\ImageViewer\Capabilities' }
$appExe = if ($null -ne $config) { $config['app_exe'] } elseif ($AppExe) { $AppExe } else { 'image-viewer.exe' }
foreach ($name in @('progId', 'appExe')) {
    if ((Get-Variable -Name $name).Value -match '[\\/]') { throw "不正な値です: $name" }
}

Remove-Item -LiteralPath "HKCU:\Software\Classes\$progId" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath "HKCU:\Software\Classes\Applications\$appExe" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath "HKCU:\$capsBase" -Recurse -Force -ErrorAction SilentlyContinue
Remove-ItemProperty -LiteralPath "HKCU:\Software\RegisteredApplications" -Name $appName -ErrorAction SilentlyContinue

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ShellNotify2 {
    [DllImport("shell32.dll")]
    public static extern void SHChangeNotify(int wEventId, uint uFlags, IntPtr dwItem1, IntPtr dwItem2);
}
"@
[ShellNotify2]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)

Write-Output "ビューワーの登録を解除しました（既定アプリ設定は変更していません）"
