#Requires -Version 5.1
<#
.SYNOPSIS
  画像ビューワーを現在のユーザーの画像関連付けに登録します（管理者権限不要）。
.DESCRIPTION
  - 関連付けの正本（ProgID・キー名・拡張子リスト）は exe 側 (assoc.rs) が持ち、
    本スクリプトは --print-assoc-config で取得して登録するだけです。
    HKCU: リテラルを保持しているため tests/association-scripts.ps1 による
    ハイブ差し替え検証が可能です。
  - ProgID (ImageViewer.App) を HKCU\Software\Classes に登録
  - Capabilitiesを登録し、既定アプリ選択はWindowsの設定画面で行います。
  - 拡張子の既定値とUserChoiceは変更しません。
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

function Initialize-RegistryKey([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -Path $Path -Force | Out-Null
    }
}

function Get-AssocConfig([string]$Exe) {
    $previousEncoding = [Console]::OutputEncoding
    try {
        # Rust側はUTF-8で出力する（app_nameに日本語を含むため）。
        [Console]::OutputEncoding = [Text.Encoding]::UTF8
        $config = @{}
        foreach ($line in (& $Exe --print-assoc-config)) {
            if ($line -match '^([^=]+)=(.*)$') { $config[$Matches[1]] = $Matches[2] }
        }
    } finally {
        [Console]::OutputEncoding = $previousEncoding
    }
    foreach ($key in @('format', 'prog_id', 'app_name', 'caps_base', 'app_exe', 'exts')) {
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
$exts = @($cfg['exts'] -split ',' | Where-Object { $_ -ne '' })
if ($exts.Count -eq 0) { throw "拡張子リストが空です" }

# --- ProgID登録 ---
Initialize-RegistryKey -Path "HKCU:\Software\Classes\$progId" | Out-Null
Set-ItemProperty -LiteralPath "HKCU:\Software\Classes\$progId" -Name "(default)" -Value $appName
Initialize-RegistryKey -Path "HKCU:\Software\Classes\$progId\DefaultIcon" | Out-Null
Set-ItemProperty -LiteralPath "HKCU:\Software\Classes\$progId\DefaultIcon" -Name "(default)" -Value "`"$ExePath`",0"
Initialize-RegistryKey -Path "HKCU:\Software\Classes\$progId\shell\open\command" | Out-Null
Set-ItemProperty -LiteralPath "HKCU:\Software\Classes\$progId\shell\open\command" -Name "(default)" -Value "`"$ExePath`" `"%1`""

# --- 「プログラムから開く」用エントリ（exe名は実行ファイルから導出） ---
$appKey = "HKCU:\Software\Classes\Applications\$($cfg['app_exe'])"
Initialize-RegistryKey -Path $appKey | Out-Null
Set-ItemProperty -LiteralPath $appKey -Name "FriendlyAppName" -Value $appName
Initialize-RegistryKey -Path "$appKey\shell\open\command" | Out-Null
Set-ItemProperty -LiteralPath "$appKey\shell\open\command" -Name "(default)" -Value "`"$ExePath`" `"%1`""
Initialize-RegistryKey -Path "$appKey\SupportedTypes" | Out-Null
foreach ($e in $exts) {
    New-ItemProperty -LiteralPath "$appKey\SupportedTypes" -Name ".$e" -Value "" -Force | Out-Null
}

# --- Default Programs (設定画面) 用Capabilities ---
$cap = "HKCU:\$($cfg['caps_base'])"
Initialize-RegistryKey -Path "$cap\FileAssociations" | Out-Null
Set-ItemProperty -LiteralPath $cap -Name "ApplicationName" -Value $appName
Set-ItemProperty -LiteralPath $cap -Name "ApplicationDescription" -Value $appName
foreach ($e in $exts) {
    Set-ItemProperty -LiteralPath "$cap\FileAssociations" -Name ".$e" -Value $progId
}
New-ItemProperty -LiteralPath "HKCU:\Software\RegisteredApplications" -Name $appName -Value $cfg['caps_base'] -Force | Out-Null

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
