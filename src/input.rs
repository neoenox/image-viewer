use crate::app::*;
use eframe::egui;
use std::time::{Duration, Instant};
/// Carry the unused fraction into the next frame instead of dropping fast
/// wheel events. Cap work per frame without losing the remaining movement.
fn wheel_navigation_steps(accum: &mut f32, delta: f32) -> i32 {
    const THRESHOLD: f32 = 20.0;
    const MAX_STEPS: i32 = 4;
    if !delta.is_finite() {
        *accum = 0.0;
        return 0;
    }
    *accum += delta;
    let steps = (*accum / THRESHOLD).trunc().clamp(-(MAX_STEPS as f32), MAX_STEPS as f32) as i32;
    *accum -= steps as f32 * THRESHOLD;
    steps
}

impl ViewerApp {
    pub(crate) fn handle_keys(&mut self, ctx: &egui::Context) {
        // Escape works while a modal is open; other shortcuts must not leak
        // into the viewer during modal or text input.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.show_settings {
                self.show_settings = false;
                return;
            }
            if self.show_assoc {
                self.show_assoc = false;
                return;
            }
            if self.fullscreen {
                self.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                return;
            }
            if self.slideshow {
                self.slideshow = false;
                return;
            }
        }
        if self.show_settings || self.show_assoc || ctx.wants_keyboard_input() {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            self.toolbar_pinned = !self.toolbar_pinned;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::T)) {
            self.show_thumbnails = !self.show_thumbnails;
        }
        let open_key = ctx.input(|i| {
            i.key_pressed(egui::Key::O) && (i.modifiers.ctrl || i.modifiers.command)
        });
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::D)) {
            self.next(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::A)) {
            self.prev(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Home)) && !self.files.is_empty() {
            self.goto(0, ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::End)) && !self.files.is_empty() {
            let n = self.files.len() - 1;
            self.goto(n, ctx);
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
            self.slideshow = self.files.len() >= 2 && !self.slideshow;
            self.last_advance = self.slideshow.then(Instant::now);
        }
        if open_key {
            self.open_dialog(ctx);
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
        let steps = wheel_navigation_steps(&mut self.wheel_accum, y);
        if steps < 0 {
            for _ in 0..steps.unsigned_abs() {
                self.next(ctx);
            }
        } else {
            for _ in 0..steps as u32 {
                self.prev(ctx);
            }
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

#[cfg(test)]
mod tests {
    use super::wheel_navigation_steps;

    #[test]
    fn wheel_delta_keeps_remainder_and_drains_burst() {
        let mut carry = 0.0;
        assert_eq!(wheel_navigation_steps(&mut carry, 45.0), 2);
        assert_eq!(carry, 5.0);
        assert_eq!(wheel_navigation_steps(&mut carry, 15.0), 1);
        assert_eq!(carry, 0.0);
        assert_eq!(wheel_navigation_steps(&mut carry, -180.0), -4);
        assert_eq!(carry, -100.0);
        assert_eq!(wheel_navigation_steps(&mut carry, 0.0), -4);
        assert_eq!(carry, -20.0);
        assert_eq!(wheel_navigation_steps(&mut carry, 0.0), -1);
        assert_eq!(carry, 0.0);
    }

    #[test]
    fn wheel_discard_non_finite_delta() {
        let mut carry = 5.0;
        assert_eq!(wheel_navigation_steps(&mut carry, f32::INFINITY), 0);
        assert_eq!(carry, 0.0);
    }
}
