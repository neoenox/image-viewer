use crate::app::ViewerApp;
use crate::settings::{ViewerSettings, DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH};
use eframe::egui;
use std::time::{Duration, Instant};

/// 追跡に必要な viewport 情報だけを抜き出した `Copy` スナップショット。
/// `ViewportInfo` 全体の clone（`events`・`title` のヒープ確保を伴う）を避ける。
#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowSnapshot {
    pub inner: Option<egui::Rect>,
    pub outer: Option<egui::Rect>,
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
    pub monitor: Option<egui::Vec2>,
    pub close_requested: bool,
}

impl WindowSnapshot {
    pub(crate) fn capture(viewport: &egui::ViewportInfo) -> Self {
        Self {
            inner: viewport.inner_rect,
            outer: viewport.outer_rect,
            maximized: viewport.maximized,
            fullscreen: viewport.fullscreen,
            monitor: viewport.monitor_size,
            close_requested: viewport.close_requested(),
        }
    }
}

/// 起動時のウィンドウ構築。復元OFF・初回・不正値は既定ジオメトリ。
pub(crate) fn build_viewport(settings: &ViewerSettings) -> egui::ViewportBuilder {
    let mut viewport = egui::ViewportBuilder::default()
        .with_min_inner_size([640.0, 480.0])
        .with_drag_and_drop(true);
    if settings.restore_window {
        viewport =
            viewport.with_inner_size([settings.window_width as f32, settings.window_height as f32]);
        if let (Some(x), Some(y)) = (settings.window_x, settings.window_y) {
            viewport = viewport.with_position([x as f32, y as f32]);
        }
        if settings.window_maximized {
            viewport = viewport.with_maximized(true);
        }
    } else {
        viewport =
            viewport.with_inner_size([DEFAULT_WINDOW_WIDTH as f32, DEFAULT_WINDOW_HEIGHT as f32]);
    }
    viewport
}

/// 保存ジオメトリが現モニターに収まるか判定し、収まらない場合の
/// (OuterPosition, InnerSize) 補正値を返す。不要なら (None, None)。
/// 部分的に重なっている配置はユーザーの意図として尊重し、補正しない。
#[cfg(test)]
fn monitor_fit(
    outer_pos: egui::Pos2,
    inner_size: egui::Vec2,
    monitor: egui::Vec2,
) -> (Option<egui::Pos2>, Option<egui::Vec2>) {
    fit_monitor_bounds(
        outer_pos,
        inner_size,
        egui::Rect::from_min_size(egui::Pos2::ZERO, monitor),
    )
}

fn fit_monitor_bounds(
    outer_pos: egui::Pos2,
    inner_size: egui::Vec2,
    bounds: egui::Rect,
) -> (Option<egui::Pos2>, Option<egui::Vec2>) {
    if !bounds.is_finite() || !bounds.is_positive() {
        return (None, None);
    }
    let size = inner_size.min(bounds.size());
    let size_cmd = (size != inner_size).then_some(size);
    let visible = outer_pos.x < bounds.max.x
        && outer_pos.y < bounds.max.y
        && outer_pos.x + size.x > bounds.min.x
        && outer_pos.y + size.y > bounds.min.y;
    let pos_cmd = if visible {
        None
    } else {
        Some(bounds.min + (bounds.size() - size) / 2.0)
    };
    (pos_cmd, size_cmd)
}

/// Windows reports desktop-global physical coordinates. Convert both the
/// window and the selected monitor using the viewport's current scale, just
/// as egui-winit does for inner_rect/outer_rect. MonitorFromRect selects a
/// connected monitor (nearest when the saved monitor was disconnected).
#[cfg(windows)]
fn native_monitor_bounds(outer: egui::Rect, pixels_per_point: f32) -> Option<egui::Rect> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct MonitorInfo {
        size: u32,
        monitor: Rect,
        work: Rect,
        flags: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn MonitorFromRect(rect: *const Rect, flags: u32) -> *mut c_void;
        fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
    }
    if !outer.is_finite()
        || !outer.is_positive()
        || !pixels_per_point.is_finite()
        || pixels_per_point <= 0.0
    {
        return None;
    }
    let physical = outer * pixels_per_point;
    let rect = Rect {
        left: physical.min.x.round() as i32,
        top: physical.min.y.round() as i32,
        right: physical.max.x.round() as i32,
        bottom: physical.max.y.round() as i32,
    };
    let mut info = MonitorInfo {
        size: std::mem::size_of::<MonitorInfo>() as u32,
        monitor: Rect::default(),
        work: Rect::default(),
        flags: 0,
    };
    // SAFETY: both pointers reference live, correctly sized Win32 structs.
    // MONITOR_DEFAULTTONEAREST (2) also handles disconnected saved screens.
    unsafe {
        let monitor = MonitorFromRect(&rect, 2);
        if monitor.is_null() || GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
    }
    let work = info.work;
    Some(
        egui::Rect::from_min_max(
            egui::pos2(work.left as f32, work.top as f32),
            egui::pos2(work.right as f32, work.bottom as f32),
        ) / pixels_per_point,
    )
}

#[cfg(not(windows))]
fn native_monitor_bounds(_outer: egui::Rect, _pixels_per_point: f32) -> Option<egui::Rect> {
    None
}

impl ViewerApp {
    /// 現在のウィンドウ状態を追跡し、次回起動時に同じ状態で開けるようにする。
    /// 毎フレーム `update` から呼ぶ。
    ///
    /// 書き込みは間引きし、最終状態は終了要求フレームで確定保存するため、
    /// 強制終了でも数秒分しか失われない。`eframe::App::on_exit` は使わない
    ///（シグネチャが glow フィーチャに依存し、どのレンダラー構成でも
    /// コンパイルできるようにするため）。
    pub(crate) fn track_window_size(&mut self, ctx: &egui::Context) {
        let snapshot = ctx.input(|i| WindowSnapshot::capture(i.viewport()));
        let before = self.settings.clone();
        self.update_window_state(snapshot);
        self.fit_to_monitor_once(ctx, snapshot);
        self.window_state_dirty |= self.settings != before;
        if self.window_state_dirty
            && (snapshot.close_requested
                || self.last_window_save.elapsed() >= Duration::from_secs(2))
        {
            self.flush_settings();
        }
        if self.window_state_dirty {
            ctx.request_repaint_after(
                Duration::from_secs(2).saturating_sub(self.last_window_save.elapsed()),
            );
        }
    }

    /// 現在のウィンドウ形状をメモリ上の設定に反映する（保存は
    /// [`Self::flush_settings`] が行う）。
    ///
    /// 最大化・全画面中はモニター形状を報告するだけで、通常ウィンドウとして
    /// 復元したい値ではないため、最大化フラグのみ保持して形状は温存する。
    pub(crate) fn update_window_state(&mut self, snapshot: WindowSnapshot) {
        use crate::settings::{
            MAX_WINDOW_POS, MAX_WINDOW_SIZE, MIN_WINDOW_HEIGHT, MIN_WINDOW_POS, MIN_WINDOW_SIZE,
        };
        if self.fullscreen || snapshot.fullscreen == Some(true) {
            return;
        }
        if snapshot.maximized == Some(true) {
            self.settings.window_maximized = true;
            return;
        }
        let Some(rect) = snapshot.inner else {
            return;
        };
        // 不正値は範囲検査で弾く。高さの下限は強制最小 (640x480) に合わせる。
        let (w, h) = (rect.width(), rect.height());
        if w.is_finite()
            && h.is_finite()
            && (MIN_WINDOW_SIZE..=MAX_WINDOW_SIZE).contains(&(w.round() as u32))
            && (MIN_WINDOW_HEIGHT..=MAX_WINDOW_SIZE).contains(&(h.round() as u32))
        {
            self.settings.window_width = w.round() as u32;
            self.settings.window_height = h.round() as u32;
            self.settings.window_maximized = false;
        }
        // `with_position` は外枠（フレーム・装飾）の位置を復元するため、
        // 外枠 rect を保存する。内枠はフォールバック。
        let pos = snapshot.outer.or(snapshot.inner).map(|rect| rect.min);
        if let Some(pos) = pos {
            if pos.x.is_finite() {
                let x = pos.x.round() as i32;
                if (MIN_WINDOW_POS..=MAX_WINDOW_POS).contains(&x) {
                    self.settings.window_x = Some(x);
                }
            }
            if pos.y.is_finite() {
                let y = pos.y.round() as i32;
                if (MIN_WINDOW_POS..=MAX_WINDOW_POS).contains(&y) {
                    self.settings.window_y = Some(y);
                }
            }
        }
    }

    /// 起動直後の1回だけ、復元形状が現モニターに収まるか確認し、
    /// 収まらなければ収まるよう補正する。通常は何も送らない。
    fn fit_to_monitor_once(&mut self, ctx: &egui::Context, snapshot: WindowSnapshot) {
        if self.startup_fit_done {
            return;
        }
        if self.fullscreen || snapshot.fullscreen == Some(true) || snapshot.maximized == Some(true)
        {
            self.startup_fit_done = true;
            return;
        }
        let (Some(monitor), Some(inner)) = (snapshot.monitor, snapshot.inner) else {
            return; // 情報が揃うまで次フレーム以降に再試行
        };
        let outer = snapshot.outer.unwrap_or(inner);
        let (pos_cmd, size_cmd) = match native_monitor_bounds(outer, ctx.pixels_per_point()) {
            Some(bounds) => fit_monitor_bounds(outer.min, inner.size(), bounds),
            // Size alone gives no monitor origin. Clamp size only; never
            // relocate a window based on an assumed (0, 0) origin.
            None => {
                let size = inner.size().min(monitor);
                (None, (size != inner.size()).then_some(size))
            }
        };
        if let Some(size) = size_cmd {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        if let Some(pos) = pos_cmd {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
        self.startup_fit_done = true;
    }

    /// 現在の設定（追跡中のウィンドウ状態を含む）をディスクに書く。
    pub(crate) fn flush_settings(&mut self) {
        self.last_window_save = Instant::now();
        if let Some(path) = &self.settings_path {
            if let Err(error) = self.settings.save(path) {
                self.status_msg = format!("設定を保存できませんでした: {error}");
                return;
            }
        }
        self.window_state_dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ViewerApp;

    #[cfg(windows)]
    #[test]
    fn native_monitor_bounds_use_viewport_scale() {
        let outer = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(800.0, 600.0));
        let physical = native_monitor_bounds(outer, 1.0).expect("Windows monitor work area");
        let scaled = native_monitor_bounds(outer / 2.0, 2.0).unwrap();
        assert_eq!(scaled, physical / 2.0);
        assert!(physical.is_positive());
        assert_eq!(native_monitor_bounds(outer, 0.0), None);
    }

    #[test]
    fn failed_window_save_retries_without_another_resize() {
        let dir = std::env::temp_dir().join(format!("iv-window-retry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let parent = dir.join("blocked");
        std::fs::write(&parent, b"not a directory").unwrap();
        let path = parent.join("settings.txt");
        let ctx = egui::Context::default();
        let mut app = ViewerApp::new(None);
        app.settings_path = Some(path.clone());
        app.last_window_save = Instant::now() - Duration::from_secs(3);
        let info = window_info([10.0, 20.0], [1280.0, 800.0], false, false);
        track_frame(&mut app, &ctx, info.clone());
        assert!(app.window_state_dirty);
        assert!(!path.exists());
        std::fs::remove_file(parent).unwrap();
        app.last_window_save = Instant::now() - Duration::from_secs(3);
        track_frame(&mut app, &ctx, info);
        assert!(!app.window_state_dirty);
        assert_eq!(ViewerSettings::load(&path), app.settings);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn connected_secondary_monitors_preserve_window_positions() {
        for origin in [
            egui::pos2(1920.0, 0.0),
            egui::pos2(-1920.0, 0.0),
            egui::pos2(0.0, -1080.0),
        ] {
            let bounds = egui::Rect::from_min_size(origin, egui::vec2(1920.0, 1080.0));
            let position = origin + egui::vec2(80.0, 100.0);
            // Coordinates are desktop-global, not relative to this monitor.
            assert_eq!(
                fit_monitor_bounds(position, egui::vec2(1100.0, 750.0), bounds),
                (None, None)
            );
        }
        let bounds =
            egui::Rect::from_min_size(egui::pos2(-1920.0, -1080.0), egui::vec2(1920.0, 1040.0));
        assert_eq!(
            fit_monitor_bounds(egui::pos2(4000.0, 100.0), egui::vec2(1100.0, 750.0), bounds),
            (Some(egui::pos2(-1510.0, -935.0)), None)
        );
    }

    #[test]
    fn unchanged_geometry_flushes_after_throttle_and_on_close() {
        for close in [false, true] {
            let dir = std::env::temp_dir()
                .join(format!("iv-final-window-{}-{close}", std::process::id()));
            let path = dir.join("settings.txt");
            let ctx = egui::Context::default();
            let mut app = ViewerApp::new(None);
            app.settings_path = Some(path.clone());
            let info = window_info([10.0, 20.0], [1280.0, 800.0], false, false);
            track_frame(&mut app, &ctx, info.clone());
            assert!(!path.exists());
            if !close {
                app.last_window_save = Instant::now() - Duration::from_secs(3);
            }
            track_frame(
                &mut app,
                &ctx,
                window_info([10.0, 20.0], [1280.0, 800.0], false, close),
            );
            assert_eq!(ViewerSettings::load(&path), app.settings);
            assert!(path.exists());
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    fn window_info(
        min: [f32; 2],
        size: [f32; 2],
        maximized: bool,
        close: bool,
    ) -> egui::ViewportInfo {
        let rect =
            egui::Rect::from_min_size(egui::pos2(min[0], min[1]), egui::vec2(size[0], size[1]));
        egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            maximized: Some(maximized),
            fullscreen: Some(false),
            events: if close {
                vec![egui::ViewportEvent::Close]
            } else {
                Vec::new()
            },
            ..Default::default()
        }
    }

    /// 1フレーム分の viewport 情報を流し込み、そのフレームで追跡を実行する。
    fn track_frame(app: &mut ViewerApp, ctx: &egui::Context, info: egui::ViewportInfo) {
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(egui::ViewportId::ROOT, info);
        let _ = ctx.run(raw, |ctx| app.track_window_size(ctx));
    }

    #[test]
    fn window_geometry_tracked_and_flushed_on_close() {
        let dir = std::env::temp_dir().join(format!("image-viewer-winsize-{}", std::process::id()));
        let path = dir.join("settings.txt");
        let ctx = egui::Context::default();
        let mut app = ViewerApp::new(None);
        app.settings_path = Some(path.clone());
        // 通常ウィンドウ: サイズ・位置を追跡し、close 要求で即時保存する。
        track_frame(
            &mut app,
            &ctx,
            window_info([100.0, 80.0], [1280.0, 800.0], false, true),
        );
        assert_eq!(
            (
                app.settings.window_width,
                app.settings.window_height,
                app.settings.window_x,
                app.settings.window_y,
                app.settings.window_maximized,
            ),
            (1280, 800, Some(100), Some(80), false)
        );
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("window_width=1280\n"), "{saved}");
        assert!(saved.contains("window_x=100\n"), "{saved}");
        assert!(saved.contains("window_maximized=false\n"), "{saved}");
        // 保存内容から全項目が復元できる（追跡経路の完全往復）。
        assert_eq!(crate::settings::ViewerSettings::load(&path), app.settings);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn window_tracking_skips_maximized_and_fullscreen() {
        let ctx = egui::Context::default();
        let mut app = ViewerApp::new(None);
        app.settings_path = None;
        track_frame(
            &mut app,
            &ctx,
            window_info([0.0, 0.0], [1920.0, 1080.0], true, false),
        );
        // 最大化中はモニターサイズを通常サイズにしない。フラグのみ立つ。
        assert!(app.settings.window_maximized);
        assert_eq!(
            (app.settings.window_width, app.settings.window_height),
            (
                crate::settings::DEFAULT_WINDOW_WIDTH,
                crate::settings::DEFAULT_WINDOW_HEIGHT
            )
        );
        // 最小サイズ (640x480) の高さも追跡される。
        track_frame(
            &mut app,
            &ctx,
            window_info([50.0, 60.0], [640.0, 480.0], false, false),
        );
        assert_eq!(
            (app.settings.window_width, app.settings.window_height),
            (640, 480)
        );
        // 通常に戻すとフラグが降り、サイズ・位置が更新される。
        track_frame(
            &mut app,
            &ctx,
            window_info([50.0, 60.0], [1280.0, 800.0], false, false),
        );
        assert!(!app.settings.window_maximized);
        assert_eq!(
            (app.settings.window_width, app.settings.window_height),
            (1280, 800)
        );
        assert_eq!(
            (app.settings.window_x, app.settings.window_y),
            (Some(50), Some(60))
        );
        // 全画面中は一切更新しない。
        app.fullscreen = true;
        track_frame(
            &mut app,
            &ctx,
            window_info([0.0, 0.0], [1920.0, 1080.0], false, false),
        );
        assert_eq!(
            (app.settings.window_width, app.settings.window_height),
            (1280, 800)
        );
        assert!(!app.settings.window_maximized);
    }

    #[test]
    fn window_tracking_ignores_bogus_rects_and_throttles_writes() {
        let dir = std::env::temp_dir().join(format!(
            "image-viewer-winsize-throttle-{}",
            std::process::id()
        ));
        let path = dir.join("settings.txt");
        let ctx = egui::Context::default();
        let mut app = ViewerApp::new(None);
        app.settings_path = Some(path.clone());
        // 情報なし・ゼロサイズ・範囲外位置・NaN では何も変わらず、
        // ファイルも作られない。
        let mut no_rect = window_info([0.0, 0.0], [1280.0, 800.0], false, false);
        no_rect.inner_rect = None;
        no_rect.outer_rect = None;
        track_frame(&mut app, &ctx, no_rect);
        // ゼロサイズ・範囲外位置（位置 (0,0) 自体は正当な値のため範囲外で検証）
        track_frame(
            &mut app,
            &ctx,
            window_info([50000.0, 50000.0], [0.0, 0.0], false, false),
        );
        track_frame(
            &mut app,
            &ctx,
            window_info([f32::NAN, f32::NAN], [f32::NAN, 800.0], false, false),
        );
        assert_eq!(
            (app.settings.window_width, app.settings.window_height),
            (
                crate::settings::DEFAULT_WINDOW_WIDTH,
                crate::settings::DEFAULT_WINDOW_HEIGHT
            )
        );
        assert_eq!((app.settings.window_x, app.settings.window_y), (None, None));
        assert!(!path.exists());
        // 変更直後は間引きで保存されないが、間隔が空けば保存される。
        track_frame(
            &mut app,
            &ctx,
            window_info([10.0, 20.0], [1280.0, 800.0], false, false),
        );
        assert!(!path.exists());
        app.last_window_save = Instant::now() - Duration::from_secs(10);
        track_frame(
            &mut app,
            &ctx,
            window_info([10.0, 20.0], [1281.0, 800.0], false, false),
        );
        assert!(path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn build_viewport_applies_saved_or_default_geometry() {
        // 全値あり: サイズ・位置・最大化が反映される。
        let settings = ViewerSettings {
            window_width: 1280,
            window_height: 800,
            window_x: Some(100),
            window_y: Some(80),
            window_maximized: true,
            restore_window: true,
            ..ViewerSettings::default()
        };
        let viewport = build_viewport(&settings);
        assert_eq!(viewport.inner_size, Some(egui::vec2(1280.0, 800.0)));
        assert_eq!(viewport.position, Some(egui::pos2(100.0, 80.0)));
        assert_eq!(viewport.maximized, Some(true));
        // 位置片欠け: 位置指定なし（中央）になる。
        let settings = ViewerSettings {
            window_x: Some(100),
            window_y: None,
            window_maximized: false,
            ..ViewerSettings::default()
        };
        let viewport = build_viewport(&settings);
        assert_eq!(viewport.position, None);
        assert_eq!(viewport.maximized, None);
        // 復元OFF: 保存値に関わらず既定ジオメトリ。
        let settings = ViewerSettings {
            restore_window: false,
            window_width: 1280,
            window_x: Some(100),
            window_y: Some(80),
            window_maximized: true,
            ..ViewerSettings::default()
        };
        let viewport = build_viewport(&settings);
        assert_eq!(
            viewport.inner_size,
            Some(egui::vec2(
                DEFAULT_WINDOW_WIDTH as f32,
                DEFAULT_WINDOW_HEIGHT as f32
            ))
        );
        assert_eq!(viewport.position, None);
        assert_eq!(viewport.maximized, None);
    }

    #[test]
    fn monitor_fit_corrects_only_when_needed() {
        let monitor = egui::vec2(1920.0, 1080.0);
        // 収まる: 何もしない。
        assert_eq!(
            monitor_fit(egui::pos2(100.0, 80.0), egui::vec2(1280.0, 800.0), monitor),
            (None, None)
        );
        // 完全画面外（右）: 中央へ。
        assert_eq!(
            monitor_fit(
                egui::pos2(3000.0, 100.0),
                egui::vec2(1280.0, 800.0),
                monitor
            ),
            (Some(egui::pos2(320.0, 140.0)), None)
        );
        // モニター超過: サイズのみ補正。
        assert_eq!(
            monitor_fit(egui::pos2(0.0, 0.0), egui::vec2(2560.0, 1440.0), monitor),
            (None, Some(egui::vec2(1920.0, 1080.0)))
        );
        // 両方: 両方補正（中央は補正後サイズ基準）。
        assert_eq!(
            monitor_fit(egui::pos2(3000.0, 0.0), egui::vec2(2560.0, 1440.0), monitor),
            (Some(egui::pos2(0.0, 0.0)), Some(egui::vec2(1920.0, 1080.0)))
        );
        // 部分重なり: 尊重して何もしない。
        assert_eq!(
            monitor_fit(
                egui::pos2(-200.0, 100.0),
                egui::vec2(1280.0, 800.0),
                monitor
            ),
            (None, None)
        );
        // 不正モニター: 何もしない。
        assert_eq!(
            monitor_fit(
                egui::pos2(3000.0, 0.0),
                egui::vec2(1280.0, 800.0),
                egui::Vec2::ZERO
            ),
            (None, None)
        );
    }

    // Multi-monitor automation matrix (PR #63).
    //
    // Approach comparison (why only viewport-matrix is implemented):
    // - virtual-display (IddSampleDriver): needs admin driver install + real
    //   GPU + reboot; impossible on `windows-latest` CI (single virtual
    //   monitor, no driver). Real multi-monitor rig still needed for E2E.
    // - WinAPI-mock (hook EnumDisplayMonitors/GetDpiForMonitor): Windows-only,
    //   unsafe detours, fragile across SDKs; CI still single-monitor so it
    //   proves little beyond what the pure function proves.
    // - viewport-matrix (this): pure `monitor_fit` geometry + `egui`
    //   `ViewportInfo` mocks. Deterministic, cross-platform, CI-safe. It
    //   covers add/remove, resolution/DPI, negative coords, off-screen,
    //   4K/8K and opt-out combos. RESIDUAL RISK: real OS enumeration
    //   (monitor hot-plug order, per-monitor DPI, taskbar insets) still needs
    //   a real multi-monitor rig.
    //
    // Conventions: `monitor_fit(outer_pos, inner_size, monitor)` returns
    // `(pos_cmd, size_cmd)`; `(None, None)` means "respect user placement".

    #[test]
    fn monitor_add_remove_matrix() {
        // Single 1080p: normal placement is respected.
        let primary = egui::vec2(1920.0, 1080.0);
        assert_eq!(
            monitor_fit(egui::pos2(100.0, 80.0), egui::vec2(1280.0, 800.0), primary),
            (None, None)
        );
        // Second monitor to the right disconnected: a window that lived at
        // x=2000 (fully on the removed secondary) recenters on primary.
        // Center of 1920x1080 for 1280x800 is (320, 140).
        assert_eq!(
            monitor_fit(
                egui::pos2(2000.0, 100.0),
                egui::vec2(1280.0, 800.0),
                primary
            ),
            (Some(egui::pos2(320.0, 140.0)), None)
        );
        // Adding a monitor does not disturb a window already visible.
        assert_eq!(
            monitor_fit(egui::pos2(100.0, 80.0), egui::vec2(1100.0, 750.0), primary),
            (None, None)
        );
        // Secondary to the left disconnected: fully-left window recenters.
        assert!(monitor_fit(
            egui::pos2(-2000.0, 100.0),
            egui::vec2(1280.0, 800.0),
            primary
        )
        .0
        .is_some());
    }

    #[test]
    fn resolution_dpi_change_matrix() {
        // 1080p -> laptop 1366x768: 1280x800 window needs height clamp only;
        // position (100,80) stays visible so no move.
        let laptop = egui::vec2(1366.0, 768.0);
        assert_eq!(
            monitor_fit(egui::pos2(100.0, 80.0), egui::vec2(1280.0, 800.0), laptop),
            (None, Some(egui::vec2(1280.0, 768.0)))
        );
        // DPI scale simulated as smaller effective monitor (150% on 1080p ->
        // 1280x720 points): same window clamps to 1280x720.
        let hidpi = egui::vec2(1280.0, 720.0);
        assert_eq!(
            monitor_fit(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0), hidpi),
            (None, Some(egui::vec2(1280.0, 720.0)))
        );
        // Resolution increase (1080p -> 1440p 2560x1440): existing placement
        // still fits, no correction.
        let larger = egui::vec2(2560.0, 1440.0);
        assert_eq!(
            monitor_fit(egui::pos2(100.0, 80.0), egui::vec2(1280.0, 800.0), larger),
            (None, None)
        );
    }

    #[test]
    fn negative_coords_partial_overlap_matrix() {
        let monitor = egui::vec2(1920.0, 1080.0);
        // Partial overlap from the left/top is user intent: respect it.
        assert_eq!(
            monitor_fit(
                egui::pos2(-500.0, -100.0),
                egui::vec2(1280.0, 800.0),
                monitor
            ),
            (None, None)
        );
        // Partial overlap from the right/bottom is also respected.
        assert_eq!(
            monitor_fit(
                egui::pos2(1500.0, 800.0),
                egui::vec2(1280.0, 800.0),
                monitor
            ),
            (None, None)
        );
        // Fully left (secondary-left layout, x=-1920 col): recenter.
        let (pos, _) = monitor_fit(egui::pos2(-1920.0, 0.0), egui::vec2(1280.0, 800.0), monitor);
        assert!(pos.is_some());
        // Negative Y fully above: recenter.
        let (pos, _) = monitor_fit(
            egui::pos2(100.0, -2000.0),
            egui::vec2(1280.0, 800.0),
            monitor,
        );
        assert!(pos.is_some());
    }

    #[test]
    fn offscreen_restore_matrix() {
        let monitor = egui::vec2(1920.0, 1080.0);
        let size = egui::vec2(1280.0, 800.0);
        // Far off-screen in every direction recenters to (320,140).
        for pos in [
            egui::pos2(5000.0, 5000.0),
            egui::pos2(-5000.0, -5000.0),
            egui::pos2(3000.0, -500.0),
            egui::pos2(-3000.0, 2000.0),
        ] {
            assert_eq!(
                monitor_fit(pos, size, monitor),
                (Some(egui::pos2(320.0, 140.0)), None),
                "pos {pos:?} must recenter"
            );
        }
        // One-pixel visibility still counts as visible (respect).
        assert_eq!(
            monitor_fit(egui::pos2(-1279.0, 100.0), size, monitor),
            (None, None)
        );
        assert_eq!(
            monitor_fit(egui::pos2(1919.0, 100.0), size, monitor),
            (None, None)
        );
    }

    #[test]
    fn uhd_4k_8k_matrix() {
        // 4K: normal window fits, no correction.
        let uhd = egui::vec2(3840.0, 2160.0);
        assert_eq!(
            monitor_fit(egui::pos2(100.0, 80.0), egui::vec2(1280.0, 800.0), uhd),
            (None, None)
        );
        // 4K fullscreen-ish window fits exactly.
        assert_eq!(
            monitor_fit(egui::pos2(0.0, 0.0), egui::vec2(3840.0, 2160.0), uhd),
            (None, None)
        );
        // Oversize on 4K clamps to monitor, position stays (visible).
        assert_eq!(
            monitor_fit(egui::pos2(0.0, 0.0), egui::vec2(5000.0, 3000.0), uhd),
            (None, Some(egui::vec2(3840.0, 2160.0)))
        );
        // 8K: max window (7680x4320 = MAX_WINDOW_SIZE x 4320) fits.
        let eight_k = egui::vec2(7680.0, 4320.0);
        assert_eq!(
            monitor_fit(egui::pos2(0.0, 0.0), egui::vec2(7680.0, 4320.0), eight_k),
            (None, None)
        );
        // Beyond 8K clamps to 8K.
        assert_eq!(
            monitor_fit(egui::pos2(0.0, 0.0), egui::vec2(8000.0, 5000.0), eight_k),
            (None, Some(egui::vec2(7680.0, 4320.0)))
        );
        // 8K window restored on 1080p laptop clamps to laptop.
        let laptop = egui::vec2(1920.0, 1080.0);
        assert_eq!(
            monitor_fit(egui::pos2(0.0, 0.0), egui::vec2(7680.0, 4320.0), laptop),
            (None, Some(egui::vec2(1920.0, 1080.0)))
        );
    }

    #[test]
    fn restore_opt_out_combos_matrix() {
        // Opt-out always yields defaults, even with off-screen/maximized/4K
        // geometry saved.
        for settings in [
            ViewerSettings {
                restore_window: false,
                window_width: 1280,
                window_height: 800,
                window_x: Some(3000),
                window_y: Some(100),
                window_maximized: true,
                ..ViewerSettings::default()
            },
            ViewerSettings {
                restore_window: false,
                window_width: 3840,
                window_height: 2160,
                window_x: Some(-2000),
                window_y: Some(-500),
                window_maximized: false,
                ..ViewerSettings::default()
            },
            ViewerSettings {
                restore_window: false,
                window_width: 7680,
                window_height: 4320,
                window_x: None,
                window_y: None,
                window_maximized: true,
                ..ViewerSettings::default()
            },
        ] {
            let viewport = build_viewport(&settings);
            assert_eq!(
                viewport.inner_size,
                Some(egui::vec2(
                    DEFAULT_WINDOW_WIDTH as f32,
                    DEFAULT_WINDOW_HEIGHT as f32
                )),
                "opt-out must ignore saved size"
            );
            assert_eq!(viewport.position, None);
            assert_eq!(viewport.maximized, None);
        }
        // Opt-in with maximized still carries size+position (maximized flag
        // wins at the OS level; the file keeps geometry for unmaximize).
        let settings = ViewerSettings {
            restore_window: true,
            window_width: 1280,
            window_height: 800,
            window_x: Some(100),
            window_y: Some(80),
            window_maximized: true,
            ..ViewerSettings::default()
        };
        let viewport = build_viewport(&settings);
        assert_eq!(viewport.inner_size, Some(egui::vec2(1280.0, 800.0)));
        assert_eq!(viewport.position, Some(egui::pos2(100.0, 80.0)));
        assert_eq!(viewport.maximized, Some(true));
    }
}
