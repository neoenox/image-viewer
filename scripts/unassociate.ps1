#Requires -Version 5.1
<#
.SYNOPSIS
  ビューワーの登録だけを解除します。
.DESCRIPTION
  - 拡張子の既定値とUserChoiceは変更しません。
  - ProgID / Applications / Capabilities / RegisteredApplications の自作エントリを削除
  - 注意: ExplorerのUserChoiceハッシュは復元できないため、
    ダブルクリックの既定が期待と違う場合はUIから選び直してください
#>

$ErrorActionPreference = "Stop"

$progId = "ImageViewer.App"
$appName = "画像ビューワー"
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

Write-Output "ビューワーの登録を解除しました（既定アプリ設定は変更していません）"
