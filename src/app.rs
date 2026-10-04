use crate::{assoc, files::*, imaging::*, loader::ImageLoader};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
/// ツールバーアイコン。Noto Sans JP と Segoe UI Symbol の両方にあるグリフのみ使う。
/// （Segoe UI Symbolは setup_jp_font でフォールバック登録される）
pub(crate) const ICON_OPEN: char = '\u{1F4C1}';
pub(crate) const ICON_PREV: char = '\u{23EE}';
pub(crate) const ICON_NEXT: char = '\u{23ED}';
pub(crate) const ICON_FIT: char = '\u{26F6}';
pub(crate) const ICON_ROT_L: char = '\u{27F2}';
pub(crate) const ICON_ROT_R: char = '\u{27F3}';
pub(crate) const ICON_PLAY: char = '\u{25B6}';
pub(crate) const ICON_STOP: char = '\u{25A0}';
pub(crate) const ICON_FULL: char = '\u{2922}';
pub(crate) const ICON_ASSOC: char = '\u{1F517}';

/// ツールバーで使う全アイコン。tofu防止の回帰テスト用。
pub const TOOLBAR_ICONS: &[char] = &[
    ICON_OPEN, ICON_PREV, ICON_NEXT, ICON_FIT, ICON_ROT_L, ICON_ROT_R, ICON_PLAY, ICON_STOP,
    ICON_FULL, ICON_ASSOC,
];

pub fn setup_jp_font(ctx: &egui::Context) {
    let candidates = [
        "NotoSansJP-VF.ttf",
        "yumin.ttf",
        "yumindb.ttf",
        "yuminl.ttf",
        "NotoSerifJP-VF.ttf",
    ];
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    let fonts_dir = std::path::Path::new(&windir).join("Fonts");
    let mut fonts = egui::FontDefinitions::default();
    let mut order: Vec<String> = Vec::new();
    for name in candidates {
        if let Ok(bytes) = std::fs::read(fonts_dir.join(name)) {
            fonts.font_data.insert(
                "japanese".to_owned(),
                egui::FontData::from_owned(bytes).into(),
            );
            order.push("japanese".to_owned());
            break;
        }
    }
    // 記号（ツールバーのアイコン用）。日本語フォントの後ろに置く。
    if let Ok(bytes) = std::fs::read(fonts_dir.join("seguisym.ttf")) {
        fonts.font_data.insert(
            "symbols".to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        order.push("symbols".to_owned());
    }
    if order.is_empty() {
        return;
    }
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            for (i, name) in order.iter().enumerate() {
                list.insert(i, name.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
}

pub struct ViewerApp {
    pub(crate) files: Vec<PathBuf>,
    pub(crate) index: usize,
    pub(crate) base: Option<std::sync::Arc<image::RgbaImage>>,
    pub(crate) texture: Option<egui::TextureHandle>,
    pub(crate) detail_tiles: Vec<(egui::TextureHandle, egui::Rect)>,
    pub(crate) load_error: Option<String>,
    pub(crate) zoom: f32,
    pub(crate) fit: bool,
    pub(crate) rotation: u8, // 0..3 : 時計回り90度 × n
    pub(crate) fullscreen: bool,
    pub(crate) slideshow: bool,
    pub(crate) slideshow_secs: f32,
    pub(crate) last_advance: Option<Instant>,
    pub(crate) status_msg: String,
    pub(crate) pan_offset: egui::Vec2,
    pub(crate) wheel_accum: f32,
    pub(crate) view_avail: egui::Vec2,
    pub(crate) view_img: egui::Vec2,
    pub(crate) peek_saved: Option<PeekState>,
    pub(crate) loader: ImageLoader,
    pub(crate) tex_cap: usize,
    pub(crate) base_cap: usize,
    pub(crate) orig_dims: Option<(u32, u32)>,
    pub(crate) animation: Option<crate::animation::Animation>,
    pub(crate) anim_due: Option<Instant>,
    pub(crate) anim_frame: usize,
    pub(crate) anim_total: usize,
    pub(crate) generation: u64,
    pub(crate) loading: bool,
    pub(crate) detail_pending: bool,
    pub(crate) view_rect: egui::Rect,
    pub(crate) detail_key: Option<crate::loader::DetailKey>,
    pub settings: crate::settings::ViewerSettings,
    /// Where settings are saved when changed; `None` keeps them in memory only.
    pub settings_path: Option<PathBuf>,
    pub(crate) show_settings: bool,
    pub(crate) show_thumbnails: bool,
    pub(crate) toolbar_pinned: bool,
    pub(crate) thumbnails: std::collections::HashMap<PathBuf, egui::TextureHandle>,
    pub(crate) thumbnail_loader: ImageLoader,
    pub(crate) show_assoc: bool,
    pub(crate) assoc_status: Vec<(String, bool)>,
}

/// 中ボタン押下中の等倍覗き見（ルーペ）用に退避する表示状態。
#[derive(Clone, Copy)]
pub(crate) struct PeekState {
    pub(crate) fit: bool,
    pub(crate) zoom: f32,
    pub(crate) pan: egui::Vec2,
}

impl ViewerApp {
    pub fn new(initial: Option<PathBuf>) -> Self {
        let mut app = Self {
            files: Vec::new(),
            index: 0,
            base: None,
            texture: None,
            detail_tiles: Vec::new(),
            load_error: None,
            zoom: 1.0,
            fit: true,
            rotation: 0,
            fullscreen: false,
            slideshow: false,
            slideshow_secs: 3.0,
            last_advance: None,
            status_msg: String::new(),
            pan_offset: egui::Vec2::ZERO,
            wheel_accum: 0.0,
            view_avail: egui::vec2(1000.0, 700.0),
            view_img: egui::vec2(800.0, 600.0),
            peek_saved: None,
            loader: ImageLoader::new(),
            tex_cap: 2048,
            base_cap: 2048,
            orig_dims: None,
            animation: None,
            anim_due: None,
            anim_frame: 0,
            anim_total: 0,
            generation: 0,
            loading: false,
            detail_pending: false,
            view_rect: egui::Rect::NOTHING,
            detail_key: None,
            settings: crate::settings::ViewerSettings::default(),
            settings_path: None,
            show_settings: false,
            show_thumbnails: false,
            toolbar_pinned: false,
            thumbnails: std::collections::HashMap::new(),
            thumbnail_loader: ImageLoader::for_thumbnails(),
            show_assoc: false,
            assoc_status: Vec::new(),
        };
        if let Some(p) = initial {
            if p.is_dir() {
                if let Some(first) = first_image_in_dir(&p) {
                    app.open_path(first, None);
                }
            } else {
                app.open_path(p, None);
            }
        }
        app
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.files.get(self.index).map(|p| p.as_path())
    }

    pub(crate) fn oriented_image(&self) -> Option<image::RgbaImage> {
        self.base
            .as_ref()
            .map(|base| rotate_rgba(base, self.rotation))
    }

    pub(crate) fn rebuild_texture(&mut self, ctx: &egui::Context) {
        // GPU上限は環境で変わり得るため都度取得
        self.tex_cap = ctx.input(|i| i.max_texture_side).max(512);
        if self.base.is_some() && self.base_cap != self.tex_cap {
            self.load_current(None);
            return;
        }
        let name = self
            .current_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".to_owned());
        if let Some(rgba) = self.oriented_image() {
            // 通常は先読み/読込時に縮小済み。念のため上限ガード。
            let rgba = downscale_to_cap(rgba, self.tex_cap);
            let (w, h) = (rgba.width() as usize, rgba.height() as usize);
            let pixels = rgba.into_raw();
            let color = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
            self.texture =
                Some(ctx.load_texture(name.clone(), color, egui::TextureOptions::LINEAR));
        } else {
            self.texture = None;
        }
    }

    pub fn open_path(&mut self, path: PathBuf, ctx: Option<&egui::Context>) {
        self.files = collect_siblings(&path);
        self.index = self.files.iter().position(|p| p == &path).unwrap_or(0);
        self.rotation = 0;
        self.fit = true;
        self.zoom = 1.0;
        self.load_current(ctx);
    }

    pub(crate) fn load_current(&mut self, ctx: Option<&egui::Context>) {
        if let Some(ctx) = ctx {
            self.tex_cap = ctx.input(|i| i.max_texture_side).max(512);
        }
        self.generation = self.generation.wrapping_add(1);
        self.base_cap = self.tex_cap;
        self.detail_tiles.clear();
        self.detail_key = None;
        self.detail_pending = false;
        self.status_msg.clear();
        self.load_error = None;
        self.texture = None;
        self.base = None;
        self.orig_dims = None;
        self.animation = None;
        self.anim_due = None;
        self.anim_frame = 0;
        self.anim_total = 0;
        self.peek_saved = None;
        self.pan_offset = egui::Vec2::ZERO;
        self.wheel_accum = 0.0;
        self.loading = false;
        let Some(path) = self.current_path().map(Path::to_path_buf) else {
            return;
        };
        self.loading = true;
        self.loader.select(self.generation, path, self.tex_cap);
        self.request_preloads();
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }

    /// Apply completed worker results only for the current selection.
    pub fn poll_loading(&mut self, ctx: &egui::Context) {
        if let Some((generation, result)) = self.loader.poll() {
            if generation == self.generation {
                self.loading = false;
                match result {
                    Ok(image) => {
                        self.base = Some(image.rgba);
                        self.orig_dims = Some(image.orig);
                        self.status_msg.clear();
                        if image.gif {
                            self.animation = Some(crate::animation::Animation::new(
                                self.current_path().unwrap().to_owned(),
                                self.tex_cap,
                            ));
                            self.anim_due = Some(Instant::now());
                        }
                        self.rebuild_texture(ctx);
                    }
                    Err(error) => self.load_error = Some(format!("開けませんでした: {error}")),
                }
            }
        }
        if self.anim_due.is_some_and(|due| Instant::now() >= due) {
            if let Some(frame) = self.animation.as_ref().and_then(|a| a.poll()) {
                match frame {
                    Ok(frame) => {
                        self.anim_frame = frame.index;
                        self.anim_total = frame.total.max(frame.index + 1);
                        self.base = Some(std::sync::Arc::new(frame.rgba));
                        self.anim_due = Some(Instant::now() + frame.delay);
                        self.rebuild_texture(ctx);
                    }
                    Err(error) => {
                        self.status_msg = format!("GIF再生エラー: {error}");
                        self.animation = None;
                        self.anim_due = None;
                    }
                }
            }
        }
        if let Some(result) = self.loader.poll_detail() {
            match result {
                Ok(detail) if self.detail_key.as_ref() == Some(&detail.key) => {
                    self.detail_pending = false;
                    self.detail_tiles.clear();
                    let [x, y, _, _] = detail.key.region;
                    let cap = self.tex_cap as u32;
                    for ty in (0..detail.rgba.height()).step_by(cap as usize) {
                        for tx in (0..detail.rgba.width()).step_by(cap as usize) {
                            let w = cap.min(detail.rgba.width() - tx);
                            let h = cap.min(detail.rgba.height() - ty);
                            let tile =
                                image::imageops::crop_imm(&detail.rgba, tx, ty, w, h).to_image();
                            let color = egui::ColorImage::from_rgba_unmultiplied(
                                [w as usize, h as usize],
                                tile.as_raw(),
                            );
                            let texture = ctx.load_texture(
                                format!("detail-{tx}-{ty}"),
                                color,
                                egui::TextureOptions::LINEAR,
                            );
                            self.detail_tiles.push((
                                texture,
                                egui::Rect::from_min_size(
                                    egui::pos2((x + tx) as f32, (y + ty) as f32),
                                    egui::vec2(w as f32, h as f32),
                                ),
                            ));
                        }
                    }
                }
                Err((key, error)) if self.detail_key.as_ref() == Some(&key) => {
                    self.detail_pending = false;
                    self.status_msg = format!("原寸表示エラー: {error}")
                }
                _ => {}
            }
        }
        if self.loading || self.animation.is_some() || self.detail_pending {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// 前後画像の先読みを要求し、遠い画像をキャッシュから捨てる。
    /// 前後2枚ずつ（最大5枚保持）でホイール連打にも追従する。
    pub(crate) fn request_preloads(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len();
        let at = |i: usize| self.files[i % n].clone();
        let next = at(self.index + 1);
        let prev = at(self.index + n - 1);
        let next2 = at(self.index + 2);
        let prev2 = at((self.index + n - (2 % n)) % n);
        let current = self.files[self.index].clone();
        self.loader.prune(
            &[
                current,
                next.clone(),
                prev.clone(),
                next2.clone(),
                prev2.clone(),
            ],
            self.tex_cap,
        );
        for p in [next, prev, next2, prev2] {
            self.loader.request(p, self.tex_cap);
        }
    }

    pub fn goto(&mut self, new_index: usize, ctx: &egui::Context) {
        if self.files.is_empty() {
            return;
        }
        self.index = new_index % self.files.len();
        self.rotation = 0;
        self.fit = true;
        self.zoom = 1.0;
        self.pan_offset = egui::Vec2::ZERO;
        self.wheel_accum = 0.0;
        self.load_current(Some(ctx));
        self.last_advance = Some(Instant::now());
    }

    pub fn next(&mut self, ctx: &egui::Context) {
        if self.files.is_empty() {
            return;
        }
        let n = (self.index + 1) % self.files.len();
        self.goto(n, ctx);
    }

    pub fn prev(&mut self, ctx: &egui::Context) {
        if self.files.is_empty() {
            return;
        }
        let n = if self.index == 0 {
            self.files.len() - 1
        } else {
            self.index - 1
        };
        self.goto(n, ctx);
    }

    pub fn rotate_cw(&mut self, ctx: &egui::Context) {
        if self.base.is_none() {
            return;
        }
        self.rotation = (self.rotation + 1) % 4;
        self.detail_tiles.clear();
        self.detail_key = None;
        self.detail_pending = false;
        self.rebuild_texture(ctx);
    }

    pub fn rotate_ccw(&mut self, ctx: &egui::Context) {
        if self.base.is_none() {
            return;
        }
        self.rotation = (self.rotation + 3) % 4;
        self.detail_tiles.clear();
        self.detail_key = None;
        self.detail_pending = false;
        self.rebuild_texture(ctx);
    }

    /// 現在の表示倍率。fit中は領域に合わせた倍率を返す。
    pub fn current_scale(&self, avail: egui::Vec2, iw: f32, ih: f32) -> f32 {
        if self.fit {
            (avail.x / iw).min(avail.y / ih).clamp(0.01, 8.0)
        } else {
            self.zoom
        }
    }

    /// パンを「画像が表示領域を常に覆う」範囲に収める。
    /// 画像が領域より大きい軸は端合わせまで動け、小さい軸は中央固定。
    /// 背景が見えるほど画像が逃げない。
    pub fn clamp_pan(&mut self, avail: egui::Vec2, disp: egui::Vec2) {
        let rx = (disp.x - avail.x) / 2.0;
        let ry = (disp.y - avail.y) / 2.0;
        self.pan_offset.x = if rx > 0.0 {
            self.pan_offset.x.clamp(-rx, rx)
        } else {
            0.0
        };
        self.pan_offset.y = if ry > 0.0 {
            self.pan_offset.y.clamp(-ry, ry)
        } else {
            0.0
        };
    }

    /// anchor（画面座標）を基準にズーム。fit中はその倍率を起点に手動モードへ移行する。
    #[allow(clippy::too_many_arguments)]
    pub fn zoom_at(
        &mut self,
        factor: f32,
        cursor: egui::Pos2,
        center: egui::Pos2,
        old_scale: f32,
        avail: egui::Vec2,
        iw: f32,
        ih: f32,
    ) {
        let new_scale = (old_scale * factor).clamp(0.05, 32.0);
        if new_scale == old_scale {
            return;
        }
        self.fit = false;
        self.zoom = new_scale;
        // カーソル下の点が動かないようパンを補正
        let k = 1.0 - new_scale / old_scale;
        self.pan_offset += (cursor - center) * k;
        let disp = egui::vec2((iw * new_scale).max(1.0), (ih * new_scale).max(1.0));
        self.clamp_pan(avail, disp);
    }

    /// 画面中央基準のズーム（ツールバー・キー用）。パンは変えない。
    pub fn zoom_centered(&mut self, factor: f32) {
        let old = self.current_scale(self.view_avail, self.view_img.x, self.view_img.y);
        let new = (old * factor).clamp(0.05, 32.0);
        if new == old {
            return;
        }
        self.fit = false;
        self.zoom = new;
    }

    /// 中ボタン押下中の一時ズーム。dy<0（上移動）で拡大、dy>0（下移動）で縮小。
    /// カーソル位置基準。離した時点で倍率確定。dy==0では何もしない。
    pub fn momentary_zoom(
        &mut self,
        dy: f32,
        cursor: egui::Pos2,
        center: egui::Pos2,
        avail: egui::Vec2,
        iw: f32,
        ih: f32,
    ) {
        if dy == 0.0 {
            return;
        }
        let old = self.current_scale(avail, iw, ih);
        let factor = (-dy * 0.004).exp().clamp(0.9, 1.11);
        self.zoom_at(factor, cursor, center, old, avail, iw, ih);
    }

    /// 中ボタン押下中の等倍覗き見を開始。すでに等倍以上なら何もしない。
    /// 戻り値は覗き見に入ったかどうか。
    pub fn begin_peek(&mut self, avail: egui::Vec2, iw: f32, ih: f32) -> bool {
        if self.peek_saved.is_some() || self.current_scale(avail, iw, ih) >= 1.0 {
            return false;
        }
        self.peek_saved = Some(PeekState {
            fit: self.fit,
            zoom: self.zoom,
            pan: self.pan_offset,
        });
        self.fit = false;
        self.zoom = 1.0;
        true
    }

    /// 等倍覗き見を終了し、押下前の表示に戻す。戻り値は復元したかどうか。
    pub fn end_peek(&mut self) -> bool {
        if let Some(s) = self.peek_saved.take() {
            self.fit = s.fit;
            self.zoom = s.zoom;
            self.pan_offset = s.pan;
            true
        } else {
            false
        }
    }

    /// 中クリック相当：フィット⇔1:1切替。1:1化はカーソル位置基準。
    pub fn toggle_fit_zoom(
        &mut self,
        cursor: egui::Pos2,
        center: egui::Pos2,
        avail: egui::Vec2,
        iw: f32,
        ih: f32,
    ) {
        if self.fit {
            let old = self.current_scale(avail, iw, ih);
            self.zoom_at(1.0 / old, cursor, center, old, avail, iw, ih);
        } else {
            self.fit = true;
            self.zoom = 1.0;
            self.pan_offset = egui::Vec2::ZERO;
        }
    }

    /// ホバー位置連動の視点移動量。tは領域内の正規化位置(0.0..1.0)。
    /// 左端で画像左側、右端で右側が見える。
    /// GAINでマウス移動に対する追従を鋭くする（中央からの半分の移動で端に届く）。
    /// はみ出しは呼び出し側のclamp_panで抑える。
    pub fn pan_for_hover(avail: egui::Vec2, disp: egui::Vec2, tx: f32, ty: f32) -> egui::Vec2 {
        const GAIN: f32 = 2.0;
        let ox = (disp.x - avail.x).max(0.0);
        let oy = (disp.y - avail.y).max(0.0);
        egui::vec2(ox * (0.5 - tx) * GAIN, oy * (0.5 - ty) * GAIN)
    }

    // ---- 参照用ゲッター ----
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
    pub fn zoom_level(&self) -> f32 {
        self.zoom
    }
    pub fn is_fit(&self) -> bool {
        self.fit
    }
    pub fn rotation_steps(&self) -> u8 {
        self.rotation
    }
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }
    pub fn has_texture(&self) -> bool {
        self.texture.is_some()
    }
    pub fn texture_size(&self) -> Option<[usize; 2]> {
        self.texture.as_ref().map(|t| t.size())
    }
    pub fn is_animated(&self) -> bool {
        self.animation.is_some()
    }
    /// Total is learned when the first loop completes; until then it is the observed count.
    pub fn frame_count(&self) -> usize {
        self.anim_total
    }
    pub fn anim_index(&self) -> usize {
        self.anim_frame
    }
    pub fn image_dims(&self) -> Option<(u32, u32)> {
        self.orig_dims
    }

    /// 表示倍率の基準は縮小テクスチャではなく元画像の寸法。
    pub fn display_image_size(&self) -> Option<egui::Vec2> {
        let (w, h) = self.orig_dims?;
        let (w, h) = if self.rotation % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        };
        Some(egui::vec2(w as f32, h as f32))
    }

    pub(crate) fn request_detail(&mut self, region: [u32; 4]) {
        if self.is_animated() {
            return;
        }
        let key = crate::loader::DetailKey {
            generation: self.generation,
            rotation: self.rotation,
            region,
        };
        if self.detail_key.as_ref() == Some(&key) {
            return;
        }
        if let Some(path) = self.current_path() {
            self.loader.detail(path.to_owned(), key.clone());
            self.detail_pending = true;
            self.detail_key = Some(key);
        }
    }
    pub fn pan(&self) -> egui::Vec2 {
        self.pan_offset
    }
    pub fn is_preloaded(&self, path: &Path) -> bool {
        self.loader.is_cached(path, self.tex_cap)
    }

    /// 関連付けダイアログ用に各拡張子の既定状態を読み直す。
    pub fn refresh_assoc_status(&mut self) {
        self.assoc_status = SUPPORTED_EXTS
            .iter()
            .map(|e| (e.to_string(), assoc::is_default(e, assoc::PROG_ID)))
            .collect();
    }

    pub(crate) fn open_dialog(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new().set_title("画像を開く");
        dlg = dlg.add_filter("画像", SUPPORTED_EXTS);
        if let Some(path) = dlg.pick_file() {
            self.open_path(path, Some(ctx));
        }
    }

    pub(crate) fn apply_dropped(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            if path.is_dir() {
                if let Some(first) = first_image_in_dir(&path) {
                    self.open_path(first, Some(ctx));
                } else {
                    self.load_error = Some("フォルダ内に画像がありません".to_owned());
                }
            } else if is_supported(&path) {
                self.open_path(path, Some(ctx));
            } else {
                self.load_error =
                    Some("未対応の形式です（jpg/png/gif/bmp/webp/tiff等に対応）".to_owned());
            }
        }
    }
}

/// Title bar / taskbar icon. The exe icon itself is embedded by build.rs.
/// Uses the PNG decoder directly: `image::load_from_memory` would instantiate every
/// format decoder again for an in-memory reader and added about 2.8 MB to the exe.
fn window_icon() -> egui::IconData {
    let bytes: &[u8] = include_bytes!("../assets/icon-256.png");
    let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes))
        .expect("bundled icon is a valid PNG");
    let image = image::DynamicImage::from_decoder(decoder)
        .expect("bundled icon is a valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

pub fn run() -> eframe::Result<()> {
    let initial: Option<PathBuf> = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir() || is_supported(p) || p.exists());

    let mut app = ViewerApp::new(initial);
    app.settings_path = crate::settings::ViewerSettings::default_path();
    if let Some(path) = &app.settings_path {
        app.settings = crate::settings::ViewerSettings::load(path);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([640.0, 480.0])
            .with_drag_and_drop(true)
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "画像ビューワー",
        options,
        Box::new(|cc| {
            setup_jp_font(&cc.egui_ctx);
            // 初回テクスチャ生成（base があれば）
            app.rebuild_texture(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    pub(crate) fn fit_scale_uses_min_ratio() {
        let app = ViewerApp::new(None);
        // 2000x1000 を 1000x700 にフィット → min(0.5, 0.7) = 0.5
        let s = app.current_scale(egui::vec2(1000.0, 700.0), 2000.0, 1000.0);
        assert!(approx(s, 0.5), "got {s}");
        // 縦長画像 500x2000 → min(2.0, 0.35) = 0.35
        let s = app.current_scale(egui::vec2(1000.0, 700.0), 500.0, 2000.0);
        assert!(approx(s, 0.35), "got {s}");
    }

    #[test]
    pub(crate) fn zoom_at_keeps_cursor_point_stationary() {
        let mut app = ViewerApp::new(None);
        app.fit = false;
        app.zoom = 1.0;
        app.pan_offset = egui::Vec2::ZERO;
        let avail = egui::vec2(1000.0, 700.0);
        let (iw, ih) = (2000.0, 1400.0);
        let center = egui::pos2(500.0, 350.0);
        let cursor = egui::pos2(700.0, 400.0);
        // ズーム前のカーソル下の画像内座標
        let before = (cursor - center) / 1.0;
        app.zoom_at(2.0, cursor, center, 1.0, avail, iw, ih);
        assert!(approx(app.zoom_level(), 2.0));
        assert!(!app.is_fit());
        let center_after = center + app.pan();
        let after = (cursor - center_after) / app.zoom_level();
        assert!(approx(after.x, before.x), "x: {after:?} vs {before:?}");
        assert!(approx(after.y, before.y), "y: {after:?} vs {before:?}");
    }

    #[test]
    pub(crate) fn zoom_clamps_to_range() {
        let mut app = ViewerApp::new(None);
        app.fit = false;
        app.zoom = 1.0;
        let avail = egui::vec2(1000.0, 700.0);
        app.zoom_at(
            1e9,
            egui::pos2(500.0, 350.0),
            egui::pos2(500.0, 350.0),
            1.0,
            avail,
            100.0,
            100.0,
        );
        assert!(approx(app.zoom_level(), 32.0));
        app.zoom_at(
            1e-9,
            egui::pos2(500.0, 350.0),
            egui::pos2(500.0, 350.0),
            32.0,
            avail,
            100.0,
            100.0,
        );
        assert!(approx(app.zoom_level(), 0.05));
    }

    #[test]
    pub(crate) fn hover_pan_endpoints() {
        let avail = egui::vec2(1000.0, 700.0);
        let disp = egui::vec2(2000.0, 1400.0);
        // GAIN=2.0のため端ではオーバーフロー全量（呼び出し側でclampされる）
        let p = ViewerApp::pan_for_hover(avail, disp, 0.0, 0.0);
        assert!(approx(p.x, 1000.0) && approx(p.y, 700.0), "got {p:?}");
        let p = ViewerApp::pan_for_hover(avail, disp, 1.0, 1.0);
        assert!(approx(p.x, -1000.0) && approx(p.y, -700.0), "got {p:?}");
        // 中央 → オフセット0
        let p = ViewerApp::pan_for_hover(avail, disp, 0.5, 0.5);
        assert!(approx(p.x, 0.0) && approx(p.y, 0.0), "got {p:?}");
        // 1/4位置で既に半分（=クランプ後の端相当）まで動く
        let p = ViewerApp::pan_for_hover(avail, disp, 0.25, 0.25);
        assert!(approx(p.x, 500.0) && approx(p.y, 350.0), "got {p:?}");
        // 画像が領域より小さい軸は中央固定
        let p = ViewerApp::pan_for_hover(avail, egui::vec2(400.0, 300.0), 0.0, 1.0);
        assert!(approx(p.x, 0.0) && approx(p.y, 0.0), "got {p:?}");
    }

    #[test]
    pub(crate) fn clamp_pan_bounds() {
        let mut app = ViewerApp::new(None);
        // 画像が覆う範囲（端合わせ）までに制限される
        app.pan_offset = egui::vec2(99999.0, -99999.0);
        app.clamp_pan(egui::vec2(1000.0, 700.0), egui::vec2(2000.0, 1400.0));
        assert!(
            approx(app.pan().x, 500.0) && approx(app.pan().y, -350.0),
            "got {:?}",
            app.pan()
        );
        // 画像が領域より小さい軸は中央固定
        app.pan_offset = egui::vec2(99999.0, -99999.0);
        app.clamp_pan(egui::vec2(1000.0, 700.0), egui::vec2(400.0, 300.0));
        assert!(
            approx(app.pan().x, 0.0) && approx(app.pan().y, 0.0),
            "got {:?}",
            app.pan()
        );
    }

    #[test]
    pub(crate) fn natural_order_numeric_names() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("1.png", "2.png"), Less);
        assert_eq!(natural_cmp("2.png", "10.png"), Less);
        assert_eq!(natural_cmp("10.png", "9.png"), Greater);
        assert_eq!(natural_cmp("a.png", "a.png"), Equal);
        // 数字ブロックが途中でも数値比較
        assert_eq!(natural_cmp("img2x.png", "img10x.png"), Less);
        // 先頭ゼロは少ない方を先に
        assert_eq!(natural_cmp("001.png", "01.png"), Less);
        assert_eq!(natural_cmp("01.png", "1.png"), Less);
        // 数字なしは辞書順
        assert_eq!(natural_cmp("b.png", "a.png"), Greater);
    }

    #[test]
    pub(crate) fn sort_image_files_numeric_order() {
        let mut files: Vec<PathBuf> = ["10.png", "2.png", "1.png", "9.png", "a.png", "11.jpg"]
            .iter()
            .map(PathBuf::from)
            .collect();
        sort_image_files(&mut files);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["1.png", "2.png", "9.png", "10.png", "11.jpg", "a.png"]
        );
    }

    #[test]
    pub(crate) fn toolbar_icons_covered_by_fonts() {
        // ツールバーアイコンが読み込むフォントのどれかに存在すること（tofu防止）。
        // フォントファイルが無い環境ではスキップ。
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
        let fonts_dir = std::path::Path::new(&windir).join("Fonts");
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        for name in ["NotoSansJP-VF.ttf", "seguisym.ttf"] {
            if let Ok(bytes) = std::fs::read(fonts_dir.join(name)) {
                blobs.push(bytes);
            }
        }
        if blobs.is_empty() {
            eprintln!("skip: no system fonts found");
            return;
        }
        let faces: Vec<ttf_parser::Face<'_>> = blobs
            .iter()
            .filter_map(|b| ttf_parser::Face::parse(b, 0).ok())
            .collect();
        assert!(!faces.is_empty(), "no parsable fonts");
        for cp in TOOLBAR_ICONS {
            let covered = faces.iter().any(|f| f.glyph_index(*cp).is_some());
            assert!(covered, "glyph U+{:04X} missing in all fonts", *cp as u32);
        }
    }

    #[test]
    fn bundled_window_icon_decodes() {
        let icon = window_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }

    #[test]
    pub(crate) fn supported_extensions() {
        use std::path::Path;
        assert!(is_supported(Path::new("a.JPG")));
        assert!(is_supported(Path::new("a.webp")));
        assert!(!is_supported(Path::new("a.txt")));
        assert!(!is_supported(Path::new("a")));
    }

    #[test]
    pub(crate) fn downscale_respects_cap() {
        let big = image::RgbaImage::from_pixel(3000, 2000, image::Rgba([1, 2, 3, 255]));
        let small = downscale_to_cap(big, 2048);
        assert_eq!((small.width(), small.height()), (2048, 1365));
        let tiny = image::RgbaImage::from_pixel(100, 80, image::Rgba([1, 2, 3, 255]));
        let same = downscale_to_cap(tiny, 2048);
        assert_eq!((same.width(), same.height()), (100, 80));
    }

    fn settle(app: &mut ViewerApp, ctx: &egui::Context) {
        let start = Instant::now();
        while app.is_loading() {
            app.poll_loading(ctx);
            assert!(start.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn settle_detail(app: &mut ViewerApp, ctx: &egui::Context) {
        let start = Instant::now();
        while app.detail_tiles.is_empty() {
            app.poll_loading(ctx);
            assert!(start.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn visible_detail_has_original_pixels_after_rotation() {
        let dir = std::env::temp_dir().join(format!("image-viewer-detail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("original.png");
        image::RgbaImage::from_fn(1400, 1200, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 99, 255])
        })
        .save(&path)
        .unwrap();
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                max_texture_side: Some(1024),
                ..Default::default()
            },
            |_| {},
        );
        let mut app = ViewerApp::new(None);
        app.open_path(path, Some(&ctx));
        settle(&mut app, &ctx);
        app.request_detail([1024, 1024, 376, 176]);
        settle_detail(&mut app, &ctx);
        assert_eq!(app.detail_tiles.len(), 1);
        assert_eq!(app.detail_tiles[0].0.size(), [376, 176]);
        assert_eq!(app.detail_tiles[0].1.min, egui::pos2(1024.0, 1024.0));
        let output = ctx.run(egui::RawInput::default(), |_| {});
        let (_, delta) = output
            .textures_delta
            .set
            .iter()
            .find(|(id, _)| *id == app.detail_tiles[0].0.id())
            .unwrap();
        let egui::ImageData::Color(color) = &delta.image;
        assert_eq!(
            color.pixels[0],
            egui::Color32::from_rgba_unmultiplied(0, 0, 99, 255)
        );
        app.rotate_cw(&ctx);
        app.request_detail([0, 0, 256, 256]);
        settle_detail(&mut app, &ctx);
        let output = ctx.run(egui::RawInput::default(), |_| {});
        let (_, delta) = output
            .textures_delta
            .set
            .iter()
            .find(|(id, _)| *id == app.detail_tiles[0].0.id())
            .unwrap();
        let egui::ImageData::Color(color) = &delta.image;
        assert_eq!(
            color.pixels[0],
            egui::Color32::from_rgba_unmultiplied(0, (1199 % 256) as u8, 99, 255)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rotating_with_pending_detail_clears_pending_flag() {
        let dir = std::env::temp_dir().join(format!("image-viewer-rot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.png");
        image::RgbaImage::new(1400, 1200).save(&path).unwrap();
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                max_texture_side: Some(1024),
                ..Default::default()
            },
            |_| {},
        );
        let mut app = ViewerApp::new(None);
        app.open_path(path, Some(&ctx));
        settle(&mut app, &ctx);
        app.request_detail([0, 0, 256, 256]);
        assert!(app.detail_pending);
        app.rotate_cw(&ctx);
        assert!(!app.detail_pending);
        app.request_detail([0, 0, 256, 256]);
        app.rotate_ccw(&ctx);
        assert!(!app.detail_pending);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn gpu_cap_changes_reload_asynchronously() {
        let dir = std::env::temp_dir().join(format!("image-viewer-gpu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large.png");
        image::RgbaImage::new(3000, 2100).save(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = ViewerApp::new(Some(path));
        let _ = ctx.run(
            egui::RawInput {
                max_texture_side: Some(4096),
                ..Default::default()
            },
            |_| {},
        );
        app.rebuild_texture(&ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.texture_size(), Some([2048, 1434]));
        let _ = ctx.run(
            egui::RawInput {
                max_texture_side: Some(1024),
                ..Default::default()
            },
            |_| {},
        );
        app.rebuild_texture(&ctx);
        assert!(app.is_loading());
        settle(&mut app, &ctx);
        assert_eq!(app.texture_size(), Some([1024, 717]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    pub(crate) fn preload_cache_separates_gpu_caps_even_when_requests_overlap() {
        let dir = std::env::temp_dir().join(format!("image-viewer-caps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large.png");
        image::RgbaImage::new(1400, 1200).save(&path).unwrap();
        let loader = ImageLoader::new();
        loader.request(path.clone(), 1024);
        loader.request(path.clone(), 2048);
        let started = Instant::now();
        while !loader.is_cached(&path, 1024) || !loader.is_cached(&path, 2048) {
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "preload timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(loader.take(&path, 4096).is_none());
        assert_eq!(
            loader.take(&path, 1024).unwrap().rgba.dimensions(),
            (1024, 878)
        );
        assert_eq!(
            loader.take(&path, 2048).unwrap().rgba.dimensions(),
            (1400, 1200)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
