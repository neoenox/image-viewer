#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Native, shell-free installer. The application exe is embedded at build time.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use image_viewer::assoc::{register, APP_NAME, CAPS_BASE, PROG_ID};
use image_viewer::SUPPORTED_EXTS;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const PAYLOAD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/target/installer-payload.exe"
));

fn main() {
    if let Err(error) = install() {
        message_box(&format!("インストールに失敗しました:\n{error}"), true);
    } else {
        message_box("画像ビューワーのインストールが完了しました。", false);
    }
}

fn install() -> Result<(), String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA が設定されていません".to_owned())?;
    let dir = local.join("Programs").join("ImageViewer");
    fs::create_dir_all(&dir).map_err(|e| format!("install dir: {e}"))?;
    let exe = dir.join("image-viewer.exe");
    fs::write(&exe, PAYLOAD).map_err(|e| format!("write exe: {e}"))?;

    register(PROG_ID, APP_NAME, CAPS_BASE, &exe, SUPPORTED_EXTS)
        .map_err(|e| format!("associations: {e}"))?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageViewer")
        .map_err(|e| format!("uninstall key: {e}"))?;
    key.set_value("DisplayName", &APP_NAME)
        .map_err(|e| format!("DisplayName: {e}"))?;
    key.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION").to_owned())
        .map_err(|e| format!("DisplayVersion: {e}"))?;
    key.set_value("InstallLocation", &dir.to_string_lossy().into_owned())
        .map_err(|e| format!("InstallLocation: {e}"))?;

    create_shortcut(&dir, &exe)?;
    let _ = Command::new(&exe).arg("--check-update").status();
    Ok(())
}

fn create_shortcut(dir: &std::path::Path, exe: &std::path::Path) -> Result<(), String> {
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("COM init: {e}"))?;
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("ShellLink: {e}"))?;
        let exe_wide: Vec<u16> = wide(exe.as_os_str());
        link.SetPath(PCWSTR(exe_wide.as_ptr()))
            .map_err(|e| format!("SetPath: {e}"))?;
        let dir_wide: Vec<u16> = wide(dir.as_os_str());
        link.SetWorkingDirectory(PCWSTR(dir_wide.as_ptr()))
            .map_err(|e| format!("SetWorkingDirectory: {e}"))?;
        let app_name = OsString::from(APP_NAME);
        let desc_wide: Vec<u16> = wide(app_name.as_os_str());
        link.SetDescription(PCWSTR(desc_wide.as_ptr()))
            .map_err(|e| format!("SetDescription: {e}"))?;
        let persist: IPersistFile = link.cast().map_err(|e| format!("IPersistFile: {e}"))?;
        let lnk = dir.join("画像ビューワー.lnk");
        let lnk_wide: Vec<u16> = wide(lnk.as_os_str());
        persist
            .Save(PCWSTR(lnk_wide.as_ptr()), true)
            .map_err(|e| format!("Save shortcut: {e}"))?;
        CoUninitialize();
    }
    Ok(())
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        value.encode_wide().chain(std::iter::once(0)).collect()
    }
    #[cfg(not(windows))]
    {
        let _ = value;
        Vec::new()
    }
}

fn message_box(text: &str, error: bool) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
    };

    let text: Vec<u16> = wide(OsString::from(text).as_os_str());
    let caption: Vec<u16> = wide(OsString::from("画像ビューワー セットアップ").as_os_str());
    let style = if error {
        MB_ICONERROR | MB_OK
    } else {
        MB_ICONINFORMATION | MB_OK
    };
    unsafe {
        MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(caption.as_ptr()), style);
    }
}
