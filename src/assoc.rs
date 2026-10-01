//! ファイル関連付け（Windows per-user, 管理者権限不要）。
//!
//! できること：ProgID・起動コマンド・アイコン・Capabilities の登録、
//! 現在の既定状態の参照、設定画面の起動。拡張子の既定値は変更しない。
//! できないこと：UserChoiceハッシュの直接書き込み（OSが保護）。
//! ダブルクリック既定の確定は設定画面でのユーザー操作が必要。

use std::io;
use std::path::{Path, PathBuf};

use winreg::{enums::HKEY_CURRENT_USER, RegKey};

/// 本番用ProgID・アプリ名・Capabilities配置。
pub const PROG_ID: &str = "ImageViewer.App";
pub const APP_NAME: &str = "画像ビューワー";
pub const CAPS_BASE: &str = r"Software\ImageViewer\Capabilities";

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

/// 実行中exeのパス。
pub fn exe_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// その拡張子のダブルクリック既定が指定ProgIDかどうか（UserChoice参照・読み取りのみ）。
pub fn is_default(ext: &str, prog_id: &str) -> bool {
    let path =
        format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.{ext}\UserChoice");
    hkcu()
        .open_subkey(path)
        .and_then(|k| k.get_value::<String, _>("ProgId"))
        .map(|p| p == prog_id)
        .unwrap_or(false)
}

/// 候補アプリとして登録する。既定値とUserChoiceは変更しない。
pub fn register(
    prog_id: &str,
    app_name: &str,
    caps_base: &str,
    exe: &Path,
    exts: &[&str],
) -> io::Result<()> {
    let hkcu = hkcu();
    let exe_str = exe.to_string_lossy().into_owned();
    let app_exe = exe
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image-viewer.exe".to_owned());

    // ProgID本体
    let (k, _) = hkcu.create_subkey(format!(r"Software\Classes\{prog_id}"))?;
    k.set_value("", &app_name.to_owned())?;
    let (k, _) = hkcu.create_subkey(format!(r"Software\Classes\{prog_id}\DefaultIcon"))?;
    k.set_value("", &format!(r#""{exe_str}",0"#))?;
    let (k, _) = hkcu.create_subkey(format!(r"Software\Classes\{prog_id}\shell\open\command"))?;
    k.set_value("", &format!(r#""{exe_str}" "%1""#))?;

    // 「プログラムから開く」用
    let (k, _) = hkcu.create_subkey(format!(r"Software\Classes\Applications\{app_exe}"))?;
    k.set_value("FriendlyAppName", &app_name.to_owned())?;
    let (k, _) = hkcu.create_subkey(format!(
        r"Software\Classes\Applications\{app_exe}\shell\open\command"
    ))?;
    k.set_value("", &format!(r#""{exe_str}" "%1""#))?;
    let (k, _) = hkcu.create_subkey(format!(
        r"Software\Classes\Applications\{app_exe}\SupportedTypes"
    ))?;
    for e in exts {
        k.set_value(format!(".{e}"), &String::new())?;
    }

    // Default Programs (設定画面) 用Capabilities
    let (k, _) = hkcu.create_subkey(caps_base)?;
    k.set_value("ApplicationName", &app_name.to_owned())?;
    k.set_value("ApplicationDescription", &app_name.to_owned())?;
    let (k, _) = hkcu.create_subkey(format!(r"{caps_base}\FileAssociations"))?;
    for e in exts {
        k.set_value(format!(".{e}"), &prog_id.to_owned())?;
    }
    let (k, _) = hkcu.create_subkey(r"Software\RegisteredApplications")?;
    k.set_value(app_name, &caps_base.to_owned())?;

    Ok(())
}

/// 関連付け登録を外す。自作キーのみ削除し、他アプリの値は触らない。
/// UserChoiceハッシュは復元できないため、既定が期待と違う場合はUIから選び直す。
pub fn unregister(
    prog_id: &str,
    app_name: &str,
    caps_base: &str,
    exe_name: &str,
) -> io::Result<()> {
    let hkcu = hkcu();
    hkcu.delete_subkey_all(format!(r"Software\Classes\{prog_id}"))
        .ok();
    hkcu.delete_subkey_all(format!(r"Software\Classes\Applications\{exe_name}"))
        .ok();
    hkcu.delete_subkey_all(caps_base).ok();
    if let Ok((k, _)) = hkcu.create_subkey(r"Software\RegisteredApplications") {
        k.delete_value(app_name).ok();
    }
    Ok(())
}

/// 設定→既定のアプリ画面を開く。
pub fn open_default_apps() -> io::Result<std::process::Child> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", "ms-settings:defaultapps"])
        .spawn()
}
