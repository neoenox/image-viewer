use crate::app::*;
use crate::{assoc, SUPPORTED_EXTS};
use eframe::egui;
use std::time::{Duration, Instant};
impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        crate::win_icon::apply(frame);
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

        let title = match self.current_path() {
            Some(p) => format!(
                "{} - 画像ビューワー",
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            None => "画像ビューワー".to_owned(),
        };
        // Only on change: every viewport command requests another frame, so sending
        // it unconditionally kept the UI repainting (one CPU core busy while idle).
        if self.window_title != title {
            self.window_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // ---- ツールバー ----
        let near_top = ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y < 48.0));
        if !self.settings.auto_hide_toolbar
            || self.toolbar_pinned
            || near_top
            || self.files.is_empty()
            || self.show_settings
            || self.show_assoc
        {
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
            if self.fit {
                self.pan_offset = egui::Vec2::ZERO;
            } else if self.settings.hover_pan && hover_in_view && avail.x > 0.0 && avail.y > 0.0 {
                let rel = hover.unwrap() - view_rect.min;
                let tx = (rel.x / avail.x).clamp(0.0, 1.0);
                let ty = (rel.y / avail.y).clamp(0.0, 1.0);
                self.pan_offset = Self::pan_for_hover(avail, disp, tx, ty);
            }
            if !self.settings.hover_pan && !self.fit {
                let drag = ui.interact(view_rect, egui::Id::new("image-pan"), egui::Sense::drag());
                if drag.dragged() {
                    self.pan_offset += ctx.input(|i| i.pointer.delta());
                }
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
                        if let Ok(exe) = assoc::exe_path() {
                            let _ = assoc::register(
                                assoc::PROG_ID,
                                assoc::APP_NAME,
                                assoc::CAPS_BASE,
                                &exe,
                                SUPPORTED_EXTS,
                            );
                        }
                        let _ = assoc::open_default_apps();
                        self.refresh_assoc_status();
                    }
                    ui.small("登録後に設定画面が開くので、各形式をクリックして確定。");
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.small_button("状態を更新").clicked() {
                            self.refresh_assoc_status();
                        }
                        if ui.small_button("登録を解除").clicked() {
                            if let Ok(exe) = assoc::exe_path() {
                                let name = exe
                                    .file_name()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "image-viewer.exe".to_owned());
                                let _ = assoc::unregister(
                                    assoc::PROG_ID,
                                    assoc::APP_NAME,
                                    assoc::CAPS_BASE,
                                    &name,
                                );
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
    }
}

impl ViewerApp {
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
                ui.small("Ctrl+ホイール:拡大縮小 / ドラッグ:視点移動");
                ui.small("← →:前後 / R:回転 / 0:フィット / F:全画面");
                if self.settings_path.is_some() {
                    ui.small("設定は自動で保存され、次回の起動時にも使われます。");
                } else {
                    ui.small("設定はこの起動中に適用されます。");
                }
            });
        self.show_settings = open;
        if self.settings != before {
            if let Some(path) = &self.settings_path {
                if let Err(error) = self.settings.save(path) {
                    self.status_msg = format!("設定を保存できませんでした: {error}");
                }
            }
        }
        if !self.settings.hold_to_peek {
            self.end_peek();
        }
    }
    fn thumbnail_panel(&mut self, ctx: &egui::Context) {
        if !self.show_thumbnails {
            self.thumbnails.clear();
            return;
        }
        let mut selected = None;
        let mut keep = Vec::new();
        egui::SidePanel::left("thumbnails")
            .exact_width(156.0)
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
                        keep.push(path.clone());
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
        self.thumbnails.retain(|path, _| keep.contains(path));
        self.thumbnail_loader.prune(&keep, 128);
        if let Some(index) = selected {
            self.goto(index, ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
