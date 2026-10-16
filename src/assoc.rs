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

// Explorerへの変更通知はshell32直呼び（新規依存なし）。
#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn SHChangeNotify(
        w_event_id: i32,
        u_flags: u32,
        dw_item1: *const std::ffi::c_void,
        dw_item2: *const std::ffi::c_void,
    );
}

/// 関連付け変更をExplorerへ通知する。APIに戻り値はなく失敗不能。
/// 非Windowsでは何もしない。
fn notify_shell_assoc_changed() {
    #[cfg(windows)]
    {
        // SHCNE_ASSOCCHANGED = 0x08000000, SHCNF_IDLIST = 0.
        unsafe { SHChangeNotify(0x0800_0000, 0, std::ptr::null(), std::ptr::null()) };
    }
}

/// 拡張子を正規化する。前のドット・前後空白・大文字を吸収し、
/// 英数字のみ・非空のものだけ受け付ける。レジストリ値名の安全のため。
pub fn sanitize_ext(ext: &str) -> Option<String> {
    let clean = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if !clean.is_empty() && clean.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Some(clean)
    } else {
        None
    }
}

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

/// 実行中exeのパス。
pub fn exe_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// その拡張子のダブルクリック既定が指定ProgIDかどうか（UserChoice参照・読み取りのみ）。
pub fn is_default(ext: &str, prog_id: &str) -> bool {
    let Some(ext) = sanitize_ext(ext) else {
        return false;
    };
    let path =
        format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.{ext}\UserChoice");
    hkcu()
        .open_subkey(path)
        .and_then(|k| k.get_value::<String, _>("ProgId"))
        .map(|p| p == prog_id)
        .unwrap_or(false)
}

/// 候補アプリとして登録する。既定値とUserChoiceは変更しない。
/// 不正な拡張子が1つでもあれば何も書かずに失敗する。
pub fn register(
    prog_id: &str,
    app_name: &str,
    caps_base: &str,
    exe: &Path,
    exts: &[&str],
) -> io::Result<()> {
    let hkcu = hkcu();
    let mut clean = Vec::with_capacity(exts.len());
    for ext in exts {
        match sanitize_ext(ext) {
            Some(ext) => clean.push(ext),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid extension: {ext}"),
                ));
            }
        }
    }
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
    for e in &clean {
        k.set_value(format!(".{e}"), &String::new())?;
    }

    // Default Programs (設定画面) 用Capabilities
    let (k, _) = hkcu.create_subkey(caps_base)?;
    k.set_value("ApplicationName", &app_name.to_owned())?;
    k.set_value("ApplicationDescription", &app_name.to_owned())?;
    let (k, _) = hkcu.create_subkey(format!(r"{caps_base}\FileAssociations"))?;
    for e in &clean {
        k.set_value(format!(".{e}"), &prog_id.to_owned())?;
    }
    let (k, _) = hkcu.create_subkey(r"Software\RegisteredApplications")?;
    k.set_value(app_name, &caps_base.to_owned())?;

    notify_shell_assoc_changed();
    Ok(())
}

/// 関連付け登録を外す。自作キーのみ削除し、他アプリの値は触らない。
/// UserChoiceハッシュは復元できないため、既定が期待と違う場合はUIから選び直す。
/// exe名は `register` と同じ導出（実行ファイル名）で統一する。
pub fn unregister(prog_id: &str, app_name: &str, caps_base: &str, exe: &Path) -> io::Result<()> {
    let hkcu = hkcu();
    let exe_name = exe
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image-viewer.exe".to_owned());
    hkcu.delete_subkey_all(format!(r"Software\Classes\{prog_id}"))
        .ok();
    hkcu.delete_subkey_all(format!(r"Software\Classes\Applications\{exe_name}"))
        .ok();
    hkcu.delete_subkey_all(caps_base).ok();
    if let Ok((k, _)) = hkcu.create_subkey(r"Software\RegisteredApplications") {
        k.delete_value(app_name).ok();
    }
    notify_shell_assoc_changed();
    Ok(())
}

/// 設定→既定のアプリ画面を開く。
pub fn open_default_apps() -> io::Result<std::process::Child> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", "ms-settings:defaultapps"])
        .spawn()
}

/// 関連付け定数を `KEY=VALUE` 行（UTF-8）で標準出力する。
/// PSラッパーがここだけを正として読むため、ProgID・キー名・拡張子リストの
/// 二重管理がない。ウィンドウは開かない。
pub fn print_assoc_config() -> io::Result<()> {
    let exe = exe_path()?;
    let app_exe = exe
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image-viewer.exe".to_owned());
    println!("format=1");
    println!("prog_id={PROG_ID}");
    println!("app_name={APP_NAME}");
    println!("caps_base={CAPS_BASE}");
    println!("app_exe={app_exe}");
    println!("exts={}", crate::SUPPORTED_EXTS.join(","));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_ext_normalizes_and_rejects() {
        assert_eq!(sanitize_ext("jpg"), Some("jpg".to_owned()));
        assert_eq!(sanitize_ext(".JPG"), Some("jpg".to_owned()));
        assert_eq!(sanitize_ext("  tif "), Some("tif".to_owned()));
        assert_eq!(sanitize_ext("jpeg2000"), Some("jpeg2000".to_owned()));
        assert_eq!(sanitize_ext(""), None);
        assert_eq!(sanitize_ext("."), None);
        assert_eq!(sanitize_ext("a/b"), None);
        assert_eq!(sanitize_ext(r"a\b"), None);
        assert_eq!(sanitize_ext("jp g"), None);
        assert_eq!(sanitize_ext("jp-g"), None);
        assert_eq!(sanitize_ext("é"), None);
        assert!(!is_default("a/b", PROG_ID));
    }

    #[test]
    fn register_rejects_bad_exts_without_writing() {
        let tag = format!("ImageViewerAssocTestBad{}", std::process::id());
        let prog = format!("{tag}.App");
        let caps = format!(r"Software\{tag}\Capabilities");
        let exe = std::env::current_exe().unwrap();
        let error = register(&prog, "Test App", &caps, &exe, &["jpg", "bogus/ext"])
            .expect_err("must reject invalid extensions");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        // Validated upfront: nothing is written on failure.
        assert!(hkcu()
            .open_subkey(format!(r"Software\Classes\{prog}"))
            .is_err());
        assert!(hkcu().open_subkey(&caps).is_err());
    }

    #[test]
    fn register_unregister_round_trip_keeps_others() {
        let tag = format!("ImageViewerAssocTest{}", std::process::id());
        let prog = format!("{tag}.App");
        let app = "Assoc Test App";
        let caps = format!(r"Software\{tag}\Capabilities");
        let ext = format!("imgassoc{}", std::process::id() % 100000);
        let hkcu = hkcu();
        // A pre-existing default owned by someone else must survive.
        let classes = format!(r"Software\Classes\.{ext}");
        hkcu.delete_subkey_all(&classes).ok();
        let (key, _) = hkcu.create_subkey(&classes).unwrap();
        key.set_value("", &"Original.App").unwrap();
        let exe = std::env::current_exe().unwrap();
        register(&prog, app, &caps, &exe, &[&ext]).unwrap();
        // ProgID, SupportedTypes, FileAssociations and the app value exist.
        assert!(hkcu
            .open_subkey(format!(r"Software\Classes\{prog}"))
            .is_ok());
        assert_eq!(
            hkcu.open_subkey(format!(
                r"Software\Classes\Applications\{}\SupportedTypes",
                exe.file_name().unwrap().to_string_lossy()
            ))
            .unwrap()
            .get_value::<String, _>(format!(".{ext}"))
            .unwrap(),
            ""
        );
        assert_eq!(
            hkcu.open_subkey(format!(r"{caps}\FileAssociations"))
                .unwrap()
                .get_value::<String, _>(format!(".{ext}"))
                .unwrap(),
            prog
        );
        // Shell notification is fire-and-forget; it must at least not panic.
        notify_shell_assoc_changed();
        unregister(&prog, app, &caps, &exe).unwrap();
        assert!(hkcu
            .open_subkey(format!(r"Software\Classes\{prog}"))
            .is_err());
        assert!(hkcu.open_subkey(&caps).is_err());
        assert_eq!(
            hkcu.open_subkey(&classes)
                .unwrap()
                .get_value::<String, _>("")
                .unwrap(),
            "Original.App"
        );
        hkcu.delete_subkey_all(&classes).ok();
        assert!(!is_default(&ext, &prog));
    }
}
