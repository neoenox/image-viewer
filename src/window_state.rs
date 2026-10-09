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
pub(crate) fn monitor_fit(
    outer_pos: egui::Pos2,
    inner_size: egui::Vec2,
    monitor: egui::Vec2,
) -> (Option<egui::Pos2>, Option<egui::Vec2>) {
    if !monitor.x.is_finite() || !monitor.y.is_finite() || monitor.x <= 0.0 || monitor.y <= 0.0 {
        return (None, None);
    }
    let size = egui::vec2(inner_size.x.min(monitor.x), inner_size.y.min(monitor.y));
    let size_cmd = (size != inner_size).then_some(size);
    let visible = outer_pos.x < monitor.x
        && outer_pos.y < monitor.y
        && outer_pos.x + size.x > 0.0
        && outer_pos.y + size.y > 0.0;
    let pos_cmd = if visible {
        None
    } else {
        Some(egui::pos2(
            ((monitor.x - size.x) / 2.0).max(0.0),
            ((monitor.y - size.y) / 2.0).max(0.0),
        ))
    };
    (pos_cmd, size_cmd)
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
        if self.settings != before
            && (snapshot.close_requested
                || self.last_window_save.elapsed() >= Duration::from_secs(2))
        {
            self.flush_settings();
        }
    }

    /// 現在のウィンドウ形状をメモリ上の設定に反映する（保存は
    /// [`Self::flush_settings`] が行う）。
    ///
    /// 最大化・全画面中はモニター形状を報告するだけで、通常ウィンドウとして
    /// 復元したい値ではないため、最大化フラグのみ保持して形状は温存する。
    pub(crate) fn update_window_state(&mut self, snapshot: WindowSnapshot) {
        use crate::settings::{MAX_WINDOW_POS, MAX_WINDOW_SIZE, MIN_WINDOW_POS, MIN_WINDOW_SIZE};
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
        // 不正値は範囲検査で弾く。
        let (w, h) = (rect.width(), rect.height());
        if w.is_finite()
            && h.is_finite()
            && (MIN_WINDOW_SIZE..=MAX_WINDOW_SIZE).contains(&(w.round() as u32))
            && (MIN_WINDOW_SIZE..=MAX_WINDOW_SIZE).contains(&(h.round() as u32))
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
        let (Some(monitor), Some(inner)) = (snapshot.monitor, snapshot.inner) else {
            return; // 情報が揃うまで次フレーム以降に再試行
        };
        let outer_pos = snapshot.outer.map(|rect| rect.min).unwrap_or(inner.min);
        let (pos_cmd, size_cmd) = monitor_fit(outer_pos, inner.size(), monitor);
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ViewerApp;

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
        // 保存内容から復元できる。
        assert_eq!(
            crate::settings::ViewerSettings::load(&path).window_height,
            800
        );
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
}
