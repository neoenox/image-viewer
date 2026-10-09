#Requires -Version 5.1
<#
.SYNOPSIS
  ビューワーの登録だけを解除します。
.DESCRIPTION
  - 削除範囲は assoc.rs の unregister と同一（ProgID / Applications/<exe名> /
    Capabilities配下 / RegisteredApplications値）。他アプリの値は触りません。
  - 拡張子の既定値とUserChoiceは変更しません。
  - 注意: ExplorerのUserChoiceハッシュは復元できないため、
    ダブルクリックの既定が期待と違う場合はUIから選び直してください
#>
param(
    [string]$ExePath = ""
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

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot "..\target\release\image-viewer.exe"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "exeが見つかりません: $ExePath（先に cargo build してください）"
}

$cfg = Get-AssocConfig -Exe $ExePath
$progId = $cfg['prog_id']
$appName = $cfg['app_name']
Remove-Item -LiteralPath "HKCU:\Software\Classes\$progId" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath "HKCU:\Software\Classes\Applications\$($cfg['app_exe'])" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath "HKCU:\$($cfg['caps_base'])" -Recurse -Force -ErrorAction SilentlyContinue
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
