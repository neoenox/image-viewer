use crate::app::*;
use eframe::egui;
use std::time::{Duration, Instant};
impl ViewerApp {
    pub(crate) fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            self.toolbar_pinned = !self.toolbar_pinned;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::T)) && !self.show_settings && !self.show_assoc {
            self.show_thumbnails = !self.show_thumbnails;
        }
        if self.show_settings || self.show_assoc {
            return;
        }
        let (open_key, quit_esc) = ctx.input(|i| {
            let open = i.key_pressed(egui::Key::O) && i.modifiers.ctrl
                || i.key_pressed(egui::Key::O) && i.modifiers.command;
            let esc = i.key_pressed(egui::Key::Escape);
            (open, esc)
        });
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::D)) {
            self.next(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::A)) {
            self.prev(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Home)) && !self.files.is_empty() {
            let ctx2 = ctx.clone();
            self.goto(0, &ctx2);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::End)) && !self.files.is_empty() {
            let n = self.files.len() - 1;
            let ctx2 = ctx.clone();
            self.goto(n, &ctx2);
        }
        if self.texture.is_some() {
            if ctx.input(|i| {
                i.key_pressed(egui::Key::Plus)
                    || i.key_pressed(egui::Key::Equals)
                    || i.key_pressed(egui::Key::E)
            }) {
                self.zoom_centered(1.25);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Minus) || i.key_pressed(egui::Key::Q)) {
                self.zoom_centered(1.0 / 1.25);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Num0)) {
            self.fit = true;
            self.zoom = 1.0;
            self.pan_offset = egui::Vec2::ZERO;
        }
        let shift = ctx.input(|i| i.modifiers.shift);
        if ctx.input(|i| i.key_pressed(egui::Key::R)) {
            if shift {
                self.rotate_ccw(ctx);
            } else {
                self.rotate_cw(ctx);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F) || i.key_pressed(egui::Key::F11)) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.slideshow = !self.slideshow;
            self.last_advance = Some(Instant::now());
        }
        if open_key {
            self.open_dialog(ctx);
        }
        if quit_esc {
            if self.fullscreen {
                self.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            } else if self.slideshow {
                self.slideshow = false;
            }
        }
    }

    /// ホイールで前後画像へ移動。生デルタを使い1ノッチ=1枚に即応する。
    /// 中ボタン押下中の誤回転では移動しない。
    pub fn handle_wheel_nav(&mut self, ctx: &egui::Context) {
        if self.show_settings || self.show_assoc {
            return;
        }
        let in_list = self.show_thumbnails
            && ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| p.x < 156.0));
        if in_list {
            return;
        }
        if ctx.input(|i| i.modifiers.ctrl) || !self.settings.wheel_navigation {
            let delta = ctx.input(|i| i.raw_scroll_delta.y);
            if delta != 0.0 && self.texture.is_some() {
                let cursor = ctx
                    .input(|i| i.pointer.hover_pos())
                    .unwrap_or(self.view_avail.to_pos2() / 2.0);
                let center = if self.view_rect.is_finite() {
                    self.view_rect.center()
                } else {
                    self.view_avail.to_pos2() / 2.0
                } + self.pan_offset;
                let size = self.view_img;
                let scale = self.current_scale(self.view_avail, size.x, size.y);
                self.zoom_at(
                    (delta / 240.0).exp(),
                    cursor,
                    center,
                    scale,
                    self.view_avail,
                    size.x,
                    size.y,
                );
            }
            return;
        }
        if self.files.len() < 2 {
            self.wheel_accum = 0.0;
            return;
        }
        if ctx.input(|i| i.pointer.middle_down()) {
            self.wheel_accum = 0.0;
            return;
        }
        // smoothは複数フレームに分散して遅く感じるためrawを使う。
        // 1ノッチ≒±40なので、しきい値20で確実に1枚進む。
        let y = ctx.input(|i| i.raw_scroll_delta.y);
        if y == 0.0 {
            return;
        }
        self.wheel_accum += y;
        if self.wheel_accum <= -20.0 {
            // 手前（下）回し = 次へ
            self.next(ctx);
            self.wheel_accum = 0.0;
        } else if self.wheel_accum >= 20.0 {
            // 奥（上）回し = 前へ
            self.prev(ctx);
            self.wheel_accum = 0.0;
        }
    }

    pub(crate) fn tick_slideshow(&mut self, ctx: &egui::Context) {
        if !self.slideshow || self.files.len() < 2 {
            return;
        }
        let interval = Duration::from_secs_f32(self.slideshow_secs.max(0.5));
        let now = Instant::now();
        match self.last_advance {
            Some(t) if now.duration_since(t) >= interval => {
                self.next(ctx);
            }
            Some(t) => {
                ctx.request_repaint_after(interval - now.duration_since(t));
            }
            None => {
                self.last_advance = Some(now);
                ctx.request_repaint_after(interval);
            }
        }
    }
}
