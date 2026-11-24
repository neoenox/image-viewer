#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Native GUI installer. The application exe is embedded at build time; no
//! PowerShell or other shell is involved.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use eframe::egui;
use image_viewer::assoc::{register, APP_NAME, CAPS_BASE, PROG_ID};
use image_viewer::SUPPORTED_EXTS;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const PAYLOAD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/target/installer-payload.exe"
));

enum Progress {
    Step(String),
    Done(Result<PathBuf, String>),
}

struct InstallerApp {
    rx: mpsc::Receiver<Progress>,
    status: Vec<String>,
    result: Option<Result<PathBuf, String>>,
}

impl eframe::App for InstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(progress) = self.rx.try_recv() {
            match progress {
                Progress::Step(text) => self.status.push(text),
                Progress::Done(result) => self.result = Some(result),
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!(
                "画像ビューワー v{} セットアップ",
                env!("CARGO_PKG_VERSION")
            ));
            ui.separator();
            ui.label("このユーザー用にインストールします（管理者権限不要）。");
            ui.add_space(8.0);
            for line in &self.status {
                ui.label(format!("• {line}"));
            }
            if let Some(result) = &self.result {
                ui.add_space(8.0);
                match result {
                    Ok(exe) => {
                        ui.colored_label(
                            egui::Color32::from_rgb(120, 220, 120),
                            "インストールが完了しました。",
                        );
                        ui.label(exe.to_string_lossy());
                        ui.horizontal(|ui| {
                            if ui.button("画像ビューワーを開く").clicked() {
                                let _ = std::process::Command::new(exe).spawn();
                            }
                            if ui.button("閉じる").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    }
                    Err(error) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("失敗: {error}"));
                        if ui.button("閉じる").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let step = |text: &str| {
            let _ = tx.send(Progress::Step(text.to_owned()));
        };
        let result = install(&step);
        let _ = tx.send(Progress::Done(result));
    });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([520.0, 320.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "画像ビューワー セットアップ",
        options,
        Box::new(move |_| {
            Ok(Box::new(InstallerApp {
                rx,
                status: Vec::new(),
                result: None,
            }))
        }),
    )
}

fn install(step: &impl Fn(&str)) -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA が設定されていません".to_owned())?;
    let dir = local.join("Programs").join("ImageViewer");
    let exe = dir.join("image-viewer.exe");

    step("インストール先を作成しています");
    fs::create_dir_all(&dir).map_err(|e| format!("install dir: {e}"))?;
    step("本体を配置しています");
    fs::write(&exe, PAYLOAD).map_err(|e| format!("write exe: {e}"))?;
    step("ファイル関連付けを登録しています");
    register(PROG_ID, APP_NAME, CAPS_BASE, &exe, SUPPORTED_EXTS)
        .map_err(|e| format!("associations: {e}"))?;
    step("アンインストール情報を登録しています");
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
    step("スタートメニューショートカットを作成しています");
    create_shortcut(&dir, &exe)?;
    step("更新を確認しています");
    let _ = std::process::Command::new(&exe)
        .arg("--check-update")
        .status();
    Ok(exe)
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
        let exe_wide = wide(exe);
        link.SetPath(PCWSTR(exe_wide.as_ptr()))
            .map_err(|e| format!("SetPath: {e}"))?;
        let dir_wide = wide(dir);
        link.SetWorkingDirectory(PCWSTR(dir_wide.as_ptr()))
            .map_err(|e| format!("SetWorkingDirectory: {e}"))?;
        let desc_wide: Vec<u16> = APP_NAME.encode_utf16().chain(std::iter::once(0)).collect();
        link.SetDescription(PCWSTR(desc_wide.as_ptr()))
            .map_err(|e| format!("SetDescription: {e}"))?;
        let persist: IPersistFile = link.cast().map_err(|e| format!("IPersistFile: {e}"))?;
        let lnk = dir.join("画像ビューワー.lnk");
        let lnk_wide = wide(&lnk);
        persist
            .Save(PCWSTR(lnk_wide.as_ptr()), true)
            .map_err(|e| format!("Save shortcut: {e}"))?;
        CoUninitialize();
    }
    Ok(())
}

fn wide(path: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
