#Requires -Version 5.1
<#
.SYNOPSIS
  associate.ps1 で行った関連付けをバックアップから戻します。
.DESCRIPTION
  - 各拡張子の既定値をバックアップ時の値に戻す（元が無ければ自作キーを削除）
  - ProgID / Applications / Capabilities / RegisteredApplications の自作エントリを削除
  - 注意: ExplorerのUserChoiceハッシュは復元できないため、
    ダブルクリックの既定が期待と違う場合はUIから選び直してください
#>

$ErrorActionPreference = "Stop"

$progId = "ImageViewer.App"
$appName = "画像ビューワー"
$backupFile = Join-Path $env:LOCALAPPDATA "ImageViewer\assoc-backup\backup.xml"
if (-not (Test-Path -LiteralPath $backupFile -PathType Leaf)) {
    throw "バックアップが見つかりません: $backupFile"
}

$backup = Import-Clixml -LiteralPath $backupFile
foreach ($row in $backup) {
    # row.Ext は "jpg" 形式で保存されている
    $cls = "HKCU:\Software\Classes\.$($row.Ext)"
    if ([string]::IsNullOrEmpty($row.ClassesDefault)) {
        if ((Test-Path -LiteralPath $cls) -and
            ((Get-ItemProperty -LiteralPath $cls -Name "(default)" -ErrorAction SilentlyContinue)."(default)" -eq $progId)) {
            Remove-Item -LiteralPath $cls -Recurse -Force
        }
    } else {
        New-Item -Path $cls -Force | Out-Null
        Set-ItemProperty -LiteralPath $cls -Name "(default)" -Value $row.ClassesDefault
    }
    Write-Output ".$($row.Ext) -> $($row.ClassesDefault) (UserChoiceはUIから選び直してください)"
}

Remove-Item -LiteralPath "HKCU:\Software\Classes\$progId" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath "HKCU:\Software\Classes\Applications\image-viewer.exe" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath "HKCU:\Software\ImageViewer" -Recurse -Force -ErrorAction SilentlyContinue
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

Write-Output "関連付けを戻しました"


