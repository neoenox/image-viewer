use crate::app::*;
use crate::{assoc, SUPPORTED_EXTS};
use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Width of the left thumbnail-list side panel, in points. Shared with the input
/// layer so the wheel-navigation dead zone matches the visible panel exactly.
pub(crate) const THUMBNAIL_PANEL_WIDTH: f32 = 156.0;

/// Pointer distance from the top edge that reveals the auto-hidden toolbar.
const TOOLBAR_REVEAL_ZONE: f32 = 48.0;

/// ポインタが上端の表示ゾーン内にあるか（自動非表示ツールバーの再表示条件）。
/// 横位置は関係なく、上端からの距離だけで決まる。
fn pointer_reveals_toolbar(hover: Option<egui::Pos2>) -> bool {
    hover.is_some_and(|p| p.y < TOOLBAR_REVEAL_ZONE)
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // 計測は監視中（F12）だけ。無効時は時刻取得すらしない。
        if self.show_stats {
            self.stats.begin_frame(Instant::now());
        }
        crate::win_icon::apply(frame);
        // 現在のウィンドウ状態（サイズ・位置・最大化）を追跡し、次回起動時に
        // 復元できるようにする。終了時フラッシュもここで行う。
        self.track_window_size(ctx);
        // GPU上限を最新化（先読みの縮小サイズに使う）
        let cap = ctx.input(|i| i.max_texture_side).max(512);
        if cap != self.tex_cap {
            self.rebuild_texture(ctx);
        }
        self.poll_loading(ctx);
        self.poll_open_dialog(ctx);
        self.apply_dropped(ctx);
        self.handle_keys(ctx);
        self.handle_wheel_nav(ctx);
        self.tick_slideshow(ctx);

        self.sync_window_title(ctx);

        // ---- ツールバー ----
        let near_top = ctx.input(|i| pointer_reveals_toolbar(i.pointer.hover_pos()));
        if self.toolbar_visible(near_top) {
            egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(ICON_OPEN.to_string())
                        .on_hover_text("開く (Ctrl+O)")
                        .clicked()
                    {
                        self.open_dialog(ctx);
                    }
                    ui.separator();
                    let has = !self.files.is_empty();
                    ui.add_enabled_ui(has, |ui| {
                        if ui
                            .button(ICON_PREV.to_string())
                            .on_hover_text("前へ (←)")
                            .clicked()
                        {
                            self.prev(ctx);
                        }
                        if ui
                            .button(ICON_NEXT.to_string())
                            .on_hover_text("次へ (→)")
                            .clicked()
                        {
                            self.next(ctx);
                        }
                    });
                    ui.separator();
                    ui.add_enabled_ui(self.texture.is_some(), |ui| {
                        if ui.button("-").on_hover_text("縮小 (-)").clicked() {
                            self.zoom_centered(1.0 / 1.25);
                        }
                        if ui.button("+").on_hover_text("拡大 (+)").clicked() {
                            self.zoom_centered(1.25);
                        }
                        if ui
                            .button(ICON_FIT.to_string())
                            .on_hover_text("フィット (0)")
                            .clicked()
                        {
                            self.fit = true;
                            self.pan_offset = egui::Vec2::ZERO;
                        }
                        if ui.button("1:1").clicked() {
                            self.fit = false;
                            self.zoom = 1.0;
                        }
                        if ui
                            .button(ICON_ROT_L.to_string())
                            .on_hover_text("左回転 (Shift+R)")
                            .clicked()
                        {
                            self.rotate_ccw(ctx);
                        }
                        if ui
                            .button(ICON_ROT_R.to_string())
                            .on_hover_text("右回転 (R)")
                            .clicked()
                        {
                            self.rotate_cw(ctx);
                        }
                    });
                    ui.separator();
                    let (label, tip) = if self.slideshow {
                        (ICON_STOP.to_string(), "停止 (Space)")
                    } else {
                        (ICON_PLAY.to_string(), "再生 (Space)")
                    };
                    if ui.button(label).on_hover_text(tip).clicked() {
                        self.slideshow = !self.slideshow;
                        self.last_advance = Some(Instant::now());
                    }
                    if self.slideshow {
                        ui.add(
                            egui::Slider::new(&mut self.slideshow_secs, 0.5..=10.0)
                                .suffix("s")
                                .show_value(true),
                        );
                    }
                    let fs_tip = if self.fullscreen {
                        "全画面解除 (Esc)"
                    } else {
                        "全画面 (F)"
                    };
                    if ui
                        .button(ICON_FULL.to_string())
                        .on_hover_text(fs_tip)
                        .clicked()
                    {
                        self.fullscreen = !self.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                    }
                    if ui
                        .button("一覧")
                        .on_hover_text("サムネイル一覧 (T)")
                        .clicked()
                    {
                        self.show_thumbnails = !self.show_thumbnails;
                    }
                    if ui.button("設定").clicked() {
                        self.show_settings = true;
                    }
                    if ui
                        .button(ICON_ASSOC.to_string())
                        .on_hover_text("ファイル関連付け")
                        .clicked()
                    {
                        self.refresh_assoc_status();
                        self.show_assoc = true;
                    }
                });
            });
        }

        self.thumbnail_panel(ctx);
        self.settings_window(ctx);

        // ---- ステータスバー ----
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.files.is_empty() {
                    ui.label("画像を開いてください（ドラッグ＆ドロップ / Ctrl+O）");
                } else if let Some(path) = self.current_path() {
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ui.label(format!(
                        "{}  ({}/{})",
                        name,
                        self.index + 1,
                        self.files.len()
                    ));
                    if let Some((ow, oh)) = self.orig_dims {
                        let (w, h) = if self.rotation % 2 == 1 {
                            (oh, ow)
                        } else {
                            (ow, oh)
                        };
                        ui.separator();
                        ui.label(format!("{}×{}", w, h));
                    }
                    ui.separator();
                    if self.fit {
                        ui.label("フィット");
                    } else {
                        ui.label(format!("{:.0}%", self.zoom * 100.0));
                    }
                    if self.rotation != 0 {
                        ui.separator();
                        ui.label(format!("{}°", self.rotation as u16 * 90));
                    }
                    if self.slideshow {
                        ui.separator();
                        ui.label("再生中");
                    }
                    if self.is_animated() {
                        ui.separator();
                        ui.label(format!("アニメ #{}", self.anim_index() + 1));
                    }
                }
                if self.loading {
                    ui.spinner();
                    ui.label("読み込み中");
                }
                if !self.status_msg.is_empty() {
                    ui.label(&self.status_msg);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small("上端:操作 / Tab:固定 / T:一覧");
                });
            });
        });

        // ---- メイン ----
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = self.load_error.clone() {
                ui.centered_and_justified(|ui| {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                });
                return;
            }
            let Some(handle) = self.texture.clone() else {
                if self.loading {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                        ui.label("読み込み中…");
                    });
                    return;
                }
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.heading("画像がありません");
                        ui.add_space(8.0);
                        ui.label("画像ファイルをドラッグ＆ドロップするか、");
                        ui.label("「開く」ボタン / Ctrl+O で開いてください。");
                        ui.add_space(16.0);
                        if ui.button("画像を開く").clicked() {
                            self.open_dialog(ctx);
                        }
                        ui.add_space(16.0);
                        ui.small("対応: jpg / png / gif / bmp / webp / tiff 他");
                    });
                });
                return;
            };

            let view_rect = ui.available_rect_before_wrap();
            self.view_rect = view_rect;
            let avail = view_rect.size();
            let size = self.display_image_size().unwrap();
            let (iw, ih) = (size.x, size.y);
            self.view_avail = avail;
            self.view_img = egui::vec2(iw, ih);

            let hover = ctx.input(|i| i.pointer.hover_pos());
            let hover_in_view = hover.map(|p| view_rect.contains(p)).unwrap_or(false);
            let mid_down = ctx.input(|i| i.pointer.middle_down());

            // 中ボタン押下中だけ拡大（ルーペ、押した時点の PEEK_MAGNIFICATION 倍）。離したら元の表示に戻る。
            // ホバー連動で覗き見位置がカーソルに追従する。
            if self.settings.hold_to_peek && mid_down && hover_in_view {
                self.begin_peek(avail, iw, ih);
            } else if !mid_down {
                self.end_peek();
            }

            // 描画サイズ
            let scale = self.current_scale(avail, iw, ih);
            let disp = egui::vec2((iw * scale).max(1.0), (ih * scale).max(1.0));

            // マウス位置連動で視点移動（ドラッグ不要）。
            // 右端に寄せると画像右側、左端で左側が見える。覗き見中も追従する。
            // ドラッグ中はドラッグを優先し、離したらホバー連動に戻る。
            let mut dragging = false;
            if !self.fit {
                let drag = ui.interact(view_rect, egui::Id::new("image-pan"), egui::Sense::drag());
                if drag.dragged() {
                    dragging = true;
                    self.pan_offset += ctx.input(|i| i.pointer.delta());
                }
            }
            if self.fit {
                self.pan_offset = egui::Vec2::ZERO;
            } else if !dragging
                && self.settings.hover_pan
                && hover_in_view
                && avail.x > 0.0
                && avail.y > 0.0
            {
                let rel = hover.unwrap() - view_rect.min;
                let tx = (rel.x / avail.x).clamp(0.0, 1.0);
                let ty = (rel.y / avail.y).clamp(0.0, 1.0);
                self.pan_offset = Self::pan_for_hover(avail, disp, tx, ty);
            }
            self.clamp_pan(avail, disp);
            let center = view_rect.center() + self.pan_offset;
            let img_rect = egui::Rect::from_center_size(center, disp);
            // Always draw preview underneath so uncached detail never leaves blank areas.
            ui.painter().image(
                handle.id(),
                img_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            // Load original pixels as soon as the (at most 2048 px) preview would be
            // stretched on screen, not only from 100% up, so magnified views stay sharp.
            let preview_stretched = disp.x > handle.size()[0] as f32 + 0.5;
            if !self.fit && preview_stretched && !self.is_animated() {
                if let Some(base) = &self.base {
                    let (ow, oh) = self.orig_dims.unwrap();
                    if ow > base.width() || oh > base.height() {
                        let visible = view_rect.intersect(img_rect);
                        let lo = ((visible.min - img_rect.min) / scale).max(egui::Vec2::ZERO);
                        let hi = ((visible.max - img_rect.min) / scale).min(size);
                        // Quantize to 128px regions to avoid re-decoding for every mouse pixel.
                        let x = (lo.x as u32 / 128) * 128;
                        let y = (lo.y as u32 / 128) * 128;
                        let right = ((hi.x.ceil() as u32).div_ceil(128) * 128).min(iw as u32);
                        let bottom = ((hi.y.ceil() as u32).div_ceil(128) * 128).min(ih as u32);
                        // At low magnification the visible region can cover most of a
                        // huge image; keep the preview rather than exceed the budget.
                        let bytes = u64::from(right.saturating_sub(x))
                            * u64::from(bottom.saturating_sub(y))
                            * 4;
                        if right > x && bottom > y && bytes <= crate::loader::CACHE_BUDGET as u64 {
                            self.request_detail([x, y, right - x, bottom - y]);
                        }
                    }
                }
                for (tile, rect) in &self.detail_tiles {
                    let dest = egui::Rect::from_min_max(
                        img_rect.min + rect.min.to_vec2() * scale,
                        img_rect.min + rect.max.to_vec2() * scale,
                    );
                    ui.painter().image(
                        tile.id(),
                        dest,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
            // 高品質拡大: 準備できていれば見えている範囲に重ねて描く
            if let Some((texture, rect)) = self.update_upscale(ctx, view_rect, img_rect, scale) {
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            // アニメ再生中は再描画を続ける
            if self.is_animated() {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        });

        // ---- 関連付けダイアログ ----
        if self.show_assoc {
            let mut open = true;
            egui::Window::new("ファイル関連付け")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    let done = self.assoc_status.iter().filter(|(_, d)| *d).count();
                    let total = self.assoc_status.len();
                    ui.heading(format!("関連付け: {done}/{total} 済み"));
                    ui.add_space(4.0);
                    if ui.button("関連付けを設定する").clicked() {
                        match assoc::exe_path() {
                            Ok(exe) => {
                                if let Err(error) = assoc::register(
                                    assoc::PROG_ID,
                                    assoc::APP_NAME,
                                    assoc::CAPS_BASE,
                                    &exe,
                                    SUPPORTED_EXTS,
                                ) {
                                    self.status_msg =
                                        format!("関連付け登録に失敗しました: {error}");
                                }
                            }
                            Err(error) => {
                                self.status_msg = format!("関連付け登録に失敗しました: {error}");
                            }
                        }
                        if let Err(error) = assoc::open_default_apps() {
                            self.status_msg = format!("設定画面を開けませんでした: {error}");
                        }
                        self.refresh_assoc_status();
                    }
                    ui.small("登録後に設定画面が開くので、各形式をクリックして確定。");
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.small_button("状態を更新").clicked() {
                            self.refresh_assoc_status();
                        }
                        if ui.small_button("登録を解除").clicked() {
                            match assoc::exe_path() {
                                Ok(exe) => {
                                    if let Err(error) = assoc::unregister(
                                        assoc::PROG_ID,
                                        assoc::APP_NAME,
                                        assoc::CAPS_BASE,
                                        &exe,
                                    ) {
                                        self.status_msg =
                                            format!("関連付け解除に失敗しました: {error}");
                                    }
                                }
                                Err(error) => {
                                    self.status_msg =
                                        format!("関連付け解除に失敗しました: {error}");
                                }
                            }
                            self.refresh_assoc_status();
                        }
                    });
                    ui.collapsing(format!("形式ごとの状態（全{total}件）"), |ui| {
                        egui::Grid::new("assoc-grid").num_columns(6).show(ui, |ui| {
                            for chunk in self.assoc_status.chunks(3) {
                                for (ext, done) in chunk {
                                    if *done {
                                        ui.colored_label(egui::Color32::GREEN, "済");
                                    } else {
                                        ui.colored_label(egui::Color32::GRAY, "未");
                                    }
                                    ui.label(format!(".{ext}"));
                                }
                                ui.end_row();
                            }
                        });
                    });
                });
            self.show_assoc = open;
        }

        self.stats_window(ctx);
        if self.show_stats {
            self.stats.end_frame(Instant::now());
        }
    }
}

impl ViewerApp {
    /// ウィンドウタイトルに出す文字列。ファイルを開いていればファイル名を前置する。
    fn window_title_text(&self) -> String {
        match self.current_path() {
            Some(p) => format!(
                "{} - 画像ビューワー",
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            None => "画像ビューワー".to_owned(),
        }
    }

    /// タイトルをウィンドウへ送る。変わったときだけ送る：viewportコマンドは
    /// どれも次フレームを要求するため、毎フレーム送るとアイドル中も再描画が
    /// 続き、CPUを1コア占有する。
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let title = self.window_title_text();
        if self.window_title != title {
            self.window_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    /// ツールバーを表示するか。自動非表示が有効でも、固定中・ポインタが上端付近・
    /// ファイル未読込・設定／関連付けダイアログの表示中は表示する。
    fn toolbar_visible(&self, pointer_near_top: bool) -> bool {
        !self.settings.auto_hide_toolbar
            || self.toolbar_pinned
            || pointer_near_top
            || self.files.is_empty()
            || self.show_settings
            || self.show_assoc
    }

    /// 表示レイテンシのオーバーレイ（F12）。
    ///
    /// `loader::bench` はワーカーが画像を作るまでしか測らないため、描画と
    /// テクスチャのアップロードを含む「実際に待つ時間」をここで出す。
    fn stats_window(&mut self, ctx: &egui::Context) {
        if !self.show_stats {
            return;
        }
        let mut open = true;
        egui::Window::new("表示レイテンシ (F12)")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("フレーム数: {}", self.stats.frames()));
                match (self.stats.frame_median_ms(), self.stats.frame_p95_ms()) {
                    (Some(median), Some(p95)) => {
                        ui.label(format!("フレーム: 中央値 {median:.1} ms / p95 {p95:.1} ms"));
                    }
                    _ => {
                        ui.label("フレーム: 計測中");
                    }
                }
                match (self.stats.display_last_ms(), self.stats.display_median_ms()) {
                    (Some(last), Some(median)) => {
                        ui.label(format!(
                            "選択→表示: 最新 {last:.0} ms / 中央値 {median:.0} ms ({}件)",
                            self.stats.display_sample_count()
                        ));
                    }
                    _ => {
                        ui.label("選択→表示: 計測中");
                    }
                }
                match (self.stats.upload_last_ms(), self.stats.upload_p95_ms()) {
                    (Some(last), Some(p95)) => {
                        ui.label(format!(
                            "テクスチャ生成: 最新 {last:.1} ms / p95 {p95:.1} ms"
                        ));
                    }
                    _ => {
                        ui.label("テクスチャ生成: 計測中");
                    }
                }
                if let Some(name) = self.current_path().map(|p| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                }) {
                    ui.label(format!("画像: {name}"));
                }
                ui.separator();
                ui.small("描画とテクスチャのアップロードを含む実表示の計測。");
                ui.small("計測中は毎フレーム時刻を取るため、普段は F12 で切っておく。");
            });
        self.show_stats = open;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let before = self.settings.clone();
        egui::Window::new("操作設定")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.checkbox(
                    &mut self.settings.wheel_navigation,
                    "ホイールで前後の画像へ移動",
                );
                ui.checkbox(
                    &mut self.settings.hold_to_peek,
                    "中ボタンを押している間だけ拡大表示（2倍）",
                );
                ui.checkbox(&mut self.settings.hover_pan, "カーソル位置で画像を見渡す");
                ui.checkbox(&mut self.settings.auto_hide_toolbar, "操作バーを自動で隠す");
                ui.checkbox(
                    &mut self.settings.restore_window,
                    "起動時に前回のウィンドウ状態を復元する",
                )
                .on_hover_text("OFF では常に既定サイズ・中央・非最大化で開きます");
                ui.horizontal(|ui| {
                    use crate::settings::ZoomQuality;
                    ui.label("拡大時の画質:");
                    ui.radio_value(
                        &mut self.settings.zoom_quality,
                        ZoomQuality::Standard,
                        "標準",
                    )
                    .on_hover_text("GPU で引き伸ばす（最速）");
                    ui.radio_value(&mut self.settings.zoom_quality, ZoomQuality::High, "高品質")
                        .on_hover_text(
                            "拡大中、見えている範囲を Lanczos で計算し直してくっきり表示",
                        );
                    ui.radio_value(
                        &mut self.settings.zoom_quality,
                        ZoomQuality::Sharp,
                        "シャープ",
                    )
                    .on_hover_text("高品質に軽いシャープ処理を加えて輪郭をさらにくっきり");
                });
                ui.separator();
                if self.settings.wheel_navigation {
                    ui.small("ホイール:前後 / Ctrl+ホイール:拡大縮小 / ドラッグ:視点移動");
                } else {
                    ui.small("ホイール:拡大縮小 / ドラッグ:視点移動");
                }
                ui.small("← →:前後 / R:回転 / 0:フィット / F:全画面 / Space:再生");
                ui.small("+ / - (E/Q):拡大縮小 / T:一覧 / Tab:バー固定");
                if self.settings_path.is_some() {
                    ui.small("設定は自動で保存され、次回の起動時にも使われます。");
                } else {
                    ui.small("設定はこの起動中に適用されます。");
                }
            });
        self.show_settings = open;
        if self.settings != before {
            self.window_state_dirty = true;
            self.flush_settings();
        }
        if !self.settings.hold_to_peek {
            self.end_peek();
        }
    }
    fn thumbnail_panel(&mut self, ctx: &egui::Context) {
        if !self.show_thumbnails {
            self.thumbnails.clear();
            self.thumb_lru.clear();
            self.thumb_retained.clear();
            return;
        }
        let mut selected = None;
        let mut visible = Vec::new();
        egui::SidePanel::left("thumbnails")
            .exact_width(THUMBNAIL_PANEL_WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("画像一覧");
                    if ui.small_button("閉じる").clicked() {
                        self.show_thumbnails = false;
                    }
                });
                egui::ScrollArea::vertical().show_rows(ui, 104.0, self.files.len(), |ui, rows| {
                    for index in rows {
                        let path = self.files[index].clone();
                        visible.push(path.clone());
                        if !self.thumbnails.contains_key(&path) {
                            if let Some(image) = self.thumbnail_loader.take(&path, 128) {
                                let color = egui::ColorImage::from_rgba_unmultiplied(
                                    [image.rgba.width() as usize, image.rgba.height() as usize],
                                    image.rgba.as_raw(),
                                );
                                self.thumbnails.insert(
                                    path.clone(),
                                    ctx.load_texture(
                                        format!("thumb-{}", path.display()),
                                        color,
                                        egui::TextureOptions::LINEAR,
                                    ),
                                );
                            } else {
                                self.thumbnail_loader.request(path.clone(), 128);
                            }
                        }
                        let response = ui.allocate_ui(egui::vec2(140.0, 100.0), |ui| {
                            if let Some(texture) = self.thumbnails.get(&path) {
                                if ui
                                    .add(
                                        egui::Button::image(
                                            egui::Image::new(texture)
                                                .max_size(egui::vec2(128.0, 72.0)),
                                        )
                                        .selected(index == self.index),
                                    )
                                    .clicked()
                                {
                                    selected = Some(index);
                                }
                            } else if self.thumbnail_loader.is_failed(&path, 128) {
                                ui.add_sized(
                                    egui::vec2(128.0, 72.0),
                                    egui::Label::new("表示できません"),
                                );
                            } else {
                                ui.allocate_space(egui::vec2(128.0, 72.0));
                            }
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if ui.selectable_label(index == self.index, name).clicked() {
                                selected = Some(index);
                            }
                        });
                        response.response.on_hover_text(path.to_string_lossy());
                    }
                });
            });
        // 保持は可視と分離する：可視分を新世代として記録し、上限超過分だけ
        // 落とす。スクロールバックはキャッシュに当たる。
        self.retain_visible_thumbs(visible);
        let retained: Vec<PathBuf> = self.thumb_retained.iter().cloned().collect();
        self.thumbnails
            .retain(|path, _| self.thumb_retained.contains(path));
        self.thumbnail_loader.prune(&retained, 128);
        if let Some(index) = selected {
            self.goto(index, ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// 可視サムネイルを新世代として記録し、上限超過の古世代を落とす。
    /// テクスチャ・デコード済み・失敗記録の保持はこの世代で統一する。
    pub(crate) fn retain_visible_thumbs(&mut self, visible: Vec<PathBuf>) {
        for path in visible {
            if self.thumb_lru.back() != Some(&path) {
                self.thumb_lru.retain(|p| p != &path);
                self.thumb_lru.push_back(path.clone());
            }
            self.thumb_retained.insert(path);
        }
        while self.thumb_lru.len() > THUMB_KEEP_MAX {
            if let Some(old) = self.thumb_lru.pop_front() {
                self.thumb_retained.remove(&old);
                self.thumbnails.remove(&old);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumb_path(i: usize) -> PathBuf {
        PathBuf::from(format!("thumb-{i:03}.png"))
    }

    // ---------------------------------------------------------------
    // ヘッドレス駆動のヘルパー。実物の `App::update` をウィンドウ無しで
    // 1フレーム分動かし、コードが実際に作った状態だけを見る。
    // ---------------------------------------------------------------

    /// 決定的なフレーム入力。画面サイズとポインタ位置だけを指定する。
    fn raw_input(size: egui::Vec2, hover: Option<egui::Pos2>) -> egui::RawInput {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        if let Some(pos) = hover {
            raw.events.push(egui::Event::PointerMoved(pos));
        }
        raw
    }

    /// `ViewerApp::update` を1フレーム駆動して出力を返す。
    fn update_frame(
        app: &mut ViewerApp,
        ctx: &egui::Context,
        raw: egui::RawInput,
    ) -> egui::FullOutput {
        let mut frame = eframe::Frame::_new_kittest();
        ctx.run(raw, |ctx| eframe::App::update(app, ctx, &mut frame))
    }

    /// ウィンドウ（`Area`）は初回フレームでは寸法を測るだけで描かれないため、
    /// 2フレーム目を駆動してその出力を返す。
    fn update_frame_settled(
        app: &mut ViewerApp,
        ctx: &egui::Context,
        raw: egui::RawInput,
    ) -> egui::FullOutput {
        let _ = update_frame(app, ctx, raw.clone());
        update_frame(app, ctx, raw)
    }

    /// このフレームでウィンドウへ送られたタイトル（`ViewportCommand::Title`）。
    fn sent_titles(output: &egui::FullOutput) -> Vec<String> {
        output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| {
                viewport
                    .commands
                    .iter()
                    .filter_map(|command| match command {
                        egui::ViewportCommand::Title(title) => Some(title.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 実際に描画されたテキスト（レイアウト結果の galley）を集める。
    /// ピクセル比較ではなく、どのラベルが配置されたかを見る。
    fn rendered_text(output: &egui::FullOutput) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// 描画テキストに `needle` を含むラベルが1つでもあるか。
    fn shows(output: &egui::FullOutput, needle: &str) -> bool {
        rendered_text(output)
            .iter()
            .any(|text| text.contains(needle))
    }

    fn solid(path: &std::path::Path, w: u32, h: u32, rgb: [u8; 3]) {
        image::RgbaImage::from_pixel(w, h, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
            .save(path)
            .unwrap();
    }

    /// a/b/c.png と壊れた d.png を持つ独立フィクスチャ（名前順で決定的）。
    fn fixture_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("iv-view-{}-{tag}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).ok();
        }
        std::fs::create_dir_all(&dir).unwrap();
        solid(&dir.join("a.png"), 100, 80, [200, 30, 30]);
        solid(&dir.join("b.png"), 60, 60, [30, 30, 200]);
        solid(&dir.join("c.png"), 40, 30, [30, 200, 30]);
        std::fs::write(dir.join("d.png"), b"not an image at all").unwrap();
        dir
    }

    /// `img-0000.png`.. の連番画像を `n` 枚持つフォルダ。
    fn numbered_dir(tag: &str, n: usize) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("iv-view-{}-{tag}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).ok();
        }
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..n {
            solid(&dir.join(format!("img-{i:04}.png")), 24, 18, [9, 9, 9]);
        }
        dir
    }

    fn headless_ctx() -> egui::Context {
        egui::Context::default()
    }

    /// バックグラウンド読込が終わるまで待つ（実時間は短いが待ちは有限）。
    fn settle(app: &mut ViewerApp, ctx: &egui::Context) {
        let start = Instant::now();
        while app.is_loading() {
            app.poll_loading(ctx);
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "background loading timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn open(app: &mut ViewerApp, ctx: &egui::Context, path: PathBuf) {
        app.open_path(path, Some(ctx));
        settle(app, ctx);
    }

    // ---------------------------------------------------------------
    // ツールバー表示判定
    // ---------------------------------------------------------------

    #[test]
    fn thumb_recency_evicts_oldest_past_cap() {
        let mut app = ViewerApp::new(None);
        let paths: Vec<PathBuf> = (0..70).map(thumb_path).collect();
        app.retain_visible_thumbs(paths);
        assert_eq!(app.thumb_retained.len(), THUMB_KEEP_MAX);
        assert_eq!(app.thumb_lru.len(), THUMB_KEEP_MAX);
        // Oldest six are gone, newest kept.
        for i in 0..6 {
            assert!(!app.thumb_retained.contains(&thumb_path(i)));
        }
        for i in 6..70 {
            assert!(app.thumb_retained.contains(&thumb_path(i)));
        }
        // Re-touching moves to the back.
        app.retain_visible_thumbs(vec![thumb_path(6)]);
        assert_eq!(app.thumb_lru.back(), Some(&thumb_path(6)));
        // 64 newer entries evict the 64 oldest ({7..69} and 6).
        let more: Vec<PathBuf> = (70..134).map(thumb_path).collect();
        app.retain_visible_thumbs(more);
        assert_eq!(app.thumb_retained.len(), THUMB_KEEP_MAX);
        assert!(!app.thumb_retained.contains(&thumb_path(7)));
        assert!(!app.thumb_retained.contains(&thumb_path(6)));
        assert!(app.thumb_retained.contains(&thumb_path(70)));
        assert!(app.thumb_retained.contains(&thumb_path(133)));
    }

    #[test]
    fn repeated_visible_paths_are_recorded_once() {
        // 同じ可視パスが1フレームに複数回来ても LRU は重複しない（MRU順は最後）。
        let mut app = ViewerApp::new(None);
        app.retain_visible_thumbs(vec![
            thumb_path(0),
            thumb_path(0),
            thumb_path(1),
            thumb_path(0),
        ]);
        assert_eq!(app.thumb_lru.len(), 2);
        assert_eq!(app.thumb_lru.back(), Some(&thumb_path(0)));
        assert_eq!(app.thumb_retained.len(), 2);
    }

    #[test]
    fn toolbar_visible_truth_table() {
        // 自動非表示OFFなら無条件に表示。
        let mut app = ViewerApp::new(None);
        app.settings.auto_hide_toolbar = false;
        assert!(app.toolbar_visible(false));
        assert!(app.toolbar_visible(true));

        // 自動非表示ON・ファイルあり・ポインタは上端から離れている → 隠れる。
        app.settings.auto_hide_toolbar = true;
        app.files = vec![PathBuf::from("a.png")];
        assert!(!app.toolbar_visible(false));

        // 個別条件はそれぞれ単独で表示を強制する。
        assert!(app.toolbar_visible(true), "pointer near the top");
        app.toolbar_pinned = true;
        assert!(app.toolbar_visible(false), "pinned with Tab");
        app.toolbar_pinned = false;
        app.files.clear();
        assert!(app.toolbar_visible(false), "nothing open");
        app.files = vec![PathBuf::from("a.png")];
        app.show_settings = true;
        assert!(app.toolbar_visible(false), "settings window open");
        app.show_settings = false;
        app.show_assoc = true;
        assert!(app.toolbar_visible(false), "association dialog open");
    }

    #[test]
    fn toolbar_reveal_zone_boundary() {
        // 上端から TOOLBAR_REVEAL_ZONE 未満だけが再表示ゾーン。横位置は無関係。
        assert!(!pointer_reveals_toolbar(None));
        assert!(pointer_reveals_toolbar(Some(egui::pos2(
            400.0,
            TOOLBAR_REVEAL_ZONE - 0.5
        ))));
        assert!(!pointer_reveals_toolbar(Some(egui::pos2(
            400.0,
            TOOLBAR_REVEAL_ZONE
        ))));
        assert!(pointer_reveals_toolbar(Some(egui::pos2(2000.0, 0.0))));
    }

    #[test]
    fn toolbar_hides_and_reveals_with_pointer() {
        // update() を丸ごと動かし、中央領域の上端が実際にツールバー分ずれることを見る。
        let dir = fixture_dir("toolbar");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        open(&mut app, &ctx, dir.join("a.png"));
        let size = egui::vec2(800.0, 600.0);

        // ポインタが上端から離れている → ツールバー無し、中央領域は上端から始まる。
        let out = update_frame(
            &mut app,
            &ctx,
            raw_input(size, Some(egui::pos2(400.0, 500.0))),
        );
        let hidden_top = app.view_rect.top();
        assert!(hidden_top < 12.0, "top={hidden_top}");
        assert!(!shows(&out, "設定"), "toolbar buttons must not be drawn");

        // ポインタが再表示ゾーン内 → ツールバーが上端を占有する。
        let out = update_frame(
            &mut app,
            &ctx,
            raw_input(size, Some(egui::pos2(400.0, TOOLBAR_REVEAL_ZONE - 1.0))),
        );
        assert!(
            app.view_rect.top() > hidden_top + 10.0,
            "revealed top={} hidden top={hidden_top}",
            app.view_rect.top()
        );
        assert!(shows(&out, "設定"), "toolbar buttons must be drawn");

        // Tab固定中はポインタが離れても表示されたまま。
        app.toolbar_pinned = true;
        let _ = update_frame(
            &mut app,
            &ctx,
            raw_input(size, Some(egui::pos2(400.0, 500.0))),
        );
        assert!(
            app.view_rect.top() > hidden_top + 10.0,
            "pinned toolbar must stay"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_state_shows_the_toolbar_too() {
        // 画像が無いときは「開く」に到達できるよう常に表示する。
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        assert!(app.files.is_empty());
        let out = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        // 画像が無いフレームでは中央パネルが早期 return して `view_rect` を更新しない
        // ため、ここで領域を主張しても意味がない（常に真になる）。描かれたラベルだけを見る。
        assert!(shows(&out, "設定"));
    }

    // ---------------------------------------------------------------
    // ウィンドウタイトル
    // ---------------------------------------------------------------

    #[test]
    fn window_title_only_sent_on_change() {
        // 再描画ストームの回帰: 同じタイトルを毎フレーム送らない。
        let dir = fixture_dir("title");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        let size = egui::vec2(800.0, 600.0);

        let out = update_frame(&mut app, &ctx, raw_input(size, None));
        assert_eq!(app.window_title, "画像ビューワー");
        assert_eq!(sent_titles(&out), vec!["画像ビューワー".to_owned()]);

        // 2フレーム目は変化なし → コマンドを送らない。
        let out = update_frame(&mut app, &ctx, raw_input(size, None));
        assert!(
            sent_titles(&out).is_empty(),
            "unchanged title must not be re-sent: {:?}",
            sent_titles(&out)
        );

        // ファイルを開くと新しいタイトルが1回だけ送られる。
        app.open_path(dir.join("a.png"), Some(&ctx));
        settle(&mut app, &ctx);
        let out = update_frame(&mut app, &ctx, raw_input(size, None));
        assert_eq!(app.window_title, "a.png - 画像ビューワー");
        assert_eq!(sent_titles(&out), vec!["a.png - 画像ビューワー".to_owned()]);
        let out = update_frame(&mut app, &ctx, raw_input(size, None));
        assert!(sent_titles(&out).is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---------------------------------------------------------------
    // ステータスバー / 中央領域の状態表示
    // ---------------------------------------------------------------

    #[test]
    fn empty_folder_renders_open_hint() {
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        let out = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(shows(&out, "画像がありません"));
        assert!(shows(
            &out,
            "画像を開いてください（ドラッグ＆ドロップ / Ctrl+O）"
        ));
        assert!(shows(&out, "上端:操作 / Tab:固定 / T:一覧"));
        assert!(!shows(&out, "フィット"));
    }

    #[test]
    fn loading_state_renders_progress() {
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        app.files = vec![PathBuf::from("pending.png")];
        app.loading = true;
        let out = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(shows(&out, "読み込み中"));
        assert!(shows(&out, "読み込み中…"));
        assert!(!shows(&out, "画像がありません"));
    }

    #[test]
    fn load_error_renders_message() {
        let dir = fixture_dir("error");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        // 壊れたファイルを実際に開く（d.png は画像ではない）。
        open(&mut app, &ctx, dir.join("d.png"));
        let error = app
            .load_error()
            .expect("corrupt file must report an error")
            .to_owned();
        assert!(error.starts_with("開けませんでした"));
        let out = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(shows(&out, &error));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn status_bar_reflects_view_state() {
        let dir = fixture_dir("status");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        open(&mut app, &ctx, dir.join("a.png")); // 100x80、4枚中1枚目
        let out = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(shows(&out, "a.png  (1/4)"));
        assert!(shows(&out, "100×80"));
        assert!(shows(&out, "フィット"));
        assert!(!shows(&out, "90°"));

        // 回転は寸法の入れ替えと角度表示に反映される。
        app.rotate_cw(&ctx);
        let out = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(shows(&out, "80×100"));
        assert!(shows(&out, "90°"));

        // 手動ズームは倍率表示（フィット表示は消える）。倍率は絶対値なので
        // 状態から期待値を組み立て、ステータスバーが追従しているかを見る。
        app.zoom_centered(1.25);
        assert!(!app.is_fit());
        let expected = format!("{:.0}%", app.zoom_level() * 100.0);
        let out = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(shows(&out, &expected), "expected {expected}");
        assert!(!shows(&out, "フィット"));

        // 再生中は状態が出る。
        app.slideshow = true;
        let out = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(shows(&out, "再生中"));

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---------------------------------------------------------------
    // 設定ウィンドウ / 関連付けダイアログ
    // ---------------------------------------------------------------

    #[test]
    fn settings_window_renders_and_reflects_settings() {
        let dir = fixture_dir("settings");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        app.show_settings = true;
        app.settings.wheel_navigation = true;
        let size = egui::vec2(800.0, 600.0);

        let out = update_frame_settled(&mut app, &ctx, raw_input(size, None));
        assert!(shows(&out, "操作設定"));
        assert!(shows(
            &out,
            "ホイール:前後 / Ctrl+ホイール:拡大縮小 / ドラッグ:視点移動"
        ));
        assert!(shows(&out, "設定はこの起動中に適用されます。"));
        // 開いただけでは設定を書き換えない。
        assert!(!app.window_state_dirty);

        // ホイール移動OFFと保存先ありで、説明文と保存先の案内が切り替わる。
        app.settings.wheel_navigation = false;
        app.settings_path = Some(dir.join("settings.txt"));
        let out = update_frame_settled(&mut app, &ctx, raw_input(size, None));
        assert!(shows(&out, "ホイール:拡大縮小 / ドラッグ:視点移動"));
        assert!(shows(
            &out,
            "設定は自動で保存され、次回の起動時にも使われます。"
        ));
        assert!(!app.window_state_dirty);
        assert!(
            !dir.join("settings.txt").exists(),
            "rendering must not persist settings"
        );

        // ダイアログを閉じた状態では描画されない。
        app.show_settings = false;
        let out = update_frame_settled(&mut app, &ctx, raw_input(size, None));
        assert!(!shows(&out, "操作設定"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn settings_window_releases_the_peek_when_disabled() {
        // 中ボタン拡大中に「中ボタンで拡大」をOFFにしたら元へ戻る。
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        let avail = egui::vec2(1000.0, 700.0);
        assert!(app.begin_peek(avail, 3000.0, 2000.0));
        assert!(!app.is_fit());

        app.show_settings = true;
        app.settings.hold_to_peek = false;
        let _ = update_frame(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(app.is_fit());
        assert!(app.peek_saved.is_none());
    }

    #[test]
    fn assoc_dialog_renders_status_count() {
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        app.refresh_assoc_status();
        let done = app.assoc_status.iter().filter(|(_, ok)| *ok).count();
        let total = app.assoc_status.len();
        assert_eq!(total, SUPPORTED_EXTS.len());

        app.show_assoc = true;
        let out = update_frame_settled(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(shows(&out, "ファイル関連付け"));
        assert!(shows(&out, &format!("関連付け: {done}/{total} 済み")));
        assert!(shows(
            &out,
            "登録後に設定画面が開くので、各形式をクリックして確定。"
        ));

        app.show_assoc = false;
        let out = update_frame_settled(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
        assert!(!shows(&out, "ファイル関連付け"));
    }

    // ---------------------------------------------------------------
    // サムネイルパネル
    // ---------------------------------------------------------------

    #[test]
    fn thumbnail_panel_hidden_releases_caches() {
        let dir = fixture_dir("thumbclose");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        open(&mut app, &ctx, dir.join("a.png"));
        let size = egui::vec2(800.0, 600.0);

        app.show_thumbnails = true;
        let out = update_frame(
            &mut app,
            &ctx,
            raw_input(size, Some(egui::pos2(400.0, 300.0))),
        );
        assert!(shows(&out, "画像一覧"));
        assert!(!app.thumb_retained.is_empty());
        app.thumbnails.insert(
            dir.join("a.png"),
            ctx.load_texture(
                "probe-thumb",
                egui::ColorImage::filled([2, 2], egui::Color32::WHITE),
                egui::TextureOptions::LINEAR,
            ),
        );

        app.show_thumbnails = false;
        let out = update_frame(&mut app, &ctx, raw_input(size, None));
        assert!(app.thumbnails.is_empty());
        assert!(app.thumb_retained.is_empty());
        assert!(app.thumb_lru.is_empty());
        assert!(!shows(&out, "画像一覧"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn thumbnail_panel_decodes_only_visible_rows() {
        // 200枚あっても保持するのは可視行だけ（スクロール分は再デコードで戻す）。
        let dir = numbered_dir("rows", 200);
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        open(&mut app, &ctx, dir.join("img-0000.png"));
        assert_eq!(app.files.len(), 200);

        app.show_thumbnails = true;
        let out = update_frame(
            &mut app,
            &ctx,
            raw_input(egui::vec2(800.0, 400.0), Some(egui::pos2(400.0, 300.0))),
        );
        assert!(shows(&out, "画像一覧"));
        let retained = app.thumb_retained.len();
        assert!(retained > 0, "at least one row must be visible");
        assert!(retained < app.files.len(), "retained={retained}");
        assert!(
            retained <= 12,
            "visible window should stay small: {retained}"
        );
        assert_eq!(app.thumb_lru.len(), retained);
        // 可視分は先頭からの連続したプレフィックス。
        let expected: std::collections::HashSet<PathBuf> =
            app.files.iter().take(retained).cloned().collect();
        assert_eq!(app.thumb_retained, expected);

        // 記録だけでなく実デコードを見る：可視行はキャッシュに載り、画面外は
        // `prune` で落ちる（prune を消すとここが落ちる）。デコードはワーカーで
        // 非同期なので、可視分が載るまで有界にフレームを回す。
        let visible_paths: Vec<PathBuf> = app.thumb_retained.iter().cloned().collect();
        let start = Instant::now();
        while !visible_paths
            .iter()
            .any(|path| app.thumbnail_loader.is_cached(path, 128))
        {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "thumbnail decode timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
            update_frame(
                &mut app,
                &ctx,
                raw_input(egui::vec2(800.0, 400.0), Some(egui::pos2(400.0, 300.0))),
            );
        }
        let off_screen = app.files.last().cloned().unwrap();
        assert!(
            !app.thumbnail_loader.is_cached(&off_screen, 128),
            "an off-screen thumbnail stayed decoded"
        );
        assert!(
            app.thumbnail_loader.retained_bytes() > 0,
            "visible thumbnails should hold decoded bytes"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn thumbnail_panel_width_matches_wheel_dead_zone() {
        // 入力層のホイール移動デッドゾーンは、実際に描かれたパネル幅と一致する。
        let dir = numbered_dir("deadzone", 4);
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        open(&mut app, &ctx, dir.join("img-0000.png"));
        let size = egui::vec2(800.0, 600.0);

        // パネルを閉じた状態の中央領域の左端を基準にする。
        let _ = update_frame(&mut app, &ctx, raw_input(size, None));
        let closed_left = app.view_rect.left();

        app.show_thumbnails = true;
        let _ = update_frame(
            &mut app,
            &ctx,
            raw_input(size, Some(egui::pos2(400.0, 300.0))),
        );
        let open_left = app.view_rect.left();
        // 中央領域はパネルを開いた分だけ共有定数（=入力層のデッドゾーン幅）右へずれる。
        assert!(
            (open_left - closed_left - THUMBNAIL_PANEL_WIDTH).abs() < 1.0,
            "closed={closed_left} open={open_left}"
        );

        // パネル内ではホイール移動しない。
        send_wheel(&ctx, &mut app, THUMBNAIL_PANEL_WIDTH - 1.0, -40.0);
        assert_eq!(app.index(), 0);
        // パネルの外では1ノッチで1枚進む。
        send_wheel(&ctx, &mut app, THUMBNAIL_PANEL_WIDTH + 1.0, -40.0);
        assert_eq!(app.index(), 1);
        // 移動先の読み込みを待つ：読み込み中は中央パネルが早期 return して
        // `view_rect` を更新しないため、待たずに測るとパネルを閉じた幅を見誤る。
        settle(&mut app, &ctx);

        // 閉じれば中央領域は元の位置に戻る。
        app.show_thumbnails = false;
        let _ = update_frame(&mut app, &ctx, raw_input(size, None));
        assert!(
            (app.view_rect.left() - closed_left).abs() < 1.0,
            "closed={closed_left} left={}",
            app.view_rect.left()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    fn send_wheel(ctx: &egui::Context, app: &mut ViewerApp, x: f32, dy: f32) {
        let mut raw = raw_input(egui::vec2(800.0, 600.0), Some(egui::pos2(x, 300.0)));
        raw.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, dy),
            modifiers: egui::Modifiers::default(),
        });
        let _ = ctx.run(raw, |ctx| app.handle_wheel_nav(ctx));
    }

    // ---------------------------------------------------------------
    // 表示レイテンシ計測（F12）
    // ---------------------------------------------------------------

    /// キー押下イベント1つだけを含むフレームを駆動する。
    fn send_key(ctx: &egui::Context, app: &mut ViewerApp, key: egui::Key) {
        let mut raw = raw_input(egui::vec2(800.0, 600.0), None);
        raw.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        let _ = update_frame(app, ctx, raw);
    }

    #[test]
    fn stats_overlay_is_absent_and_unmeasured_by_default() {
        let dir = fixture_dir("stats-off");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        open(&mut app, &ctx, dir.join("a.png"));
        let out = update_frame_settled(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));

        assert!(!app.show_stats, "F12 を押すまでは無効");
        assert!(!shows(&out, "表示レイテンシ"), "オーバーレイは描かない");
        // 無効時は計測そのものを行わない（時刻取得もフレーム計上もしない）。
        assert_eq!(app.stats.frames(), 0);
        assert_eq!(app.stats.display_sample_count(), 0);
        assert_eq!(app.stats.upload_last_ms(), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn f12_toggles_the_stats_overlay() {
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        assert!(!app.show_stats);

        send_key(&ctx, &mut app, egui::Key::F12);
        assert!(app.show_stats, "1回目で表示");

        send_key(&ctx, &mut app, egui::Key::F12);
        assert!(!app.show_stats, "2回目で非表示");
    }

    #[test]
    fn stats_overlay_measures_selection_to_display_and_upload() {
        let dir = fixture_dir("stats-on");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        app.show_stats = true;

        for name in ["a.png", "b.png"] {
            open(&mut app, &ctx, dir.join(name));
            let out =
                update_frame_settled(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));
            assert!(
                shows(&out, "表示レイテンシ"),
                "{name} の表示中はオーバーレイが出る"
            );
        }

        // 画像を選ぶたびに「選択→表示」が1サンプル増える。
        assert_eq!(app.stats.display_sample_count(), 2);
        assert!(app.stats.display_last_ms().is_some());
        assert!(app.stats.display_median_ms().is_some());
        // フレーム時間とテクスチャ生成（アップロード）も測れている。
        assert!(app.stats.frames() >= 2);
        assert!(app.stats.upload_last_ms().is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stats_overlay_renders_the_measured_numbers() {
        let dir = fixture_dir("stats-text");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        app.show_stats = true;
        open(&mut app, &ctx, dir.join("a.png"));
        let out = update_frame_settled(&mut app, &ctx, raw_input(egui::vec2(800.0, 600.0), None));

        // 「計測中」ではなく実測値がラベル化されている（表示まで通っている）。
        let text = rendered_text(&out).join("\n");
        assert!(text.contains("選択→表示: 最新"), "text={text}");
        assert!(text.contains("テクスチャ生成: 最新"), "text={text}");
        assert!(text.contains("画像: a.png"), "text={text}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_load_records_no_waiting_sample() {
        let dir = fixture_dir("stats-fail");
        let ctx = headless_ctx();
        let mut app = ViewerApp::new(None);
        open(&mut app, &ctx, dir.join("a.png"));

        // 監視を始めてから壊れた画像を選ぶ：表示されないので待ち時間は残さない。
        app.show_stats = true;
        open(&mut app, &ctx, dir.join("d.png"));
        assert!(app.load_error.is_some(), "d.png は復号できない");
        assert_eq!(app.stats.display_sample_count(), 0);
        assert_eq!(app.stats.display_last_ms(), None);

        std::fs::remove_dir_all(&dir).ok();
    }
}
