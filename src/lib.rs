use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

const SUPPORTED_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "ico", "dds", "hdr", "exr", "qoi",
    "pbm", "pgm", "ppm", "pam",
];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 画像をデコードする。内容から形式を推測するので、拡張子が間違っている
/// ファイル（例: 中身はJPEGなのに .png）も開ける。
pub fn decode_image(path: &Path) -> image::ImageResult<image::DynamicImage> {
    if let Ok(reader) = image::ImageReader::open(path) {
        if let Ok(guessed) = reader.with_guessed_format() {
            if guessed.format().is_some() {
                if let Ok(img) = guessed.decode() {
                    return Ok(img);
                }
            }
        }
    }
    image::open(path)
}

/// ファイル名の自然順ソート用に、末尾の連番を数値として比較する。
/// "1.png" < "2.png" < "10.png" の順になる（辞書順だと "10" < "2" になる問題の対策）。
/// 数字の先頭ゼロは少ない方を先にする（"001" < "01" < "1"）。
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut ia, mut ib) = (a.char_indices().peekable(), b.char_indices().peekable());
    loop {
        let (ca, cb) = (ia.peek().map(|&(_, c)| c), ib.peek().map(|&(_, c)| c));
        match (ca, cb) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => {
                if x.is_ascii_digit() && y.is_ascii_digit() {
                    // 数字ブロックを切り出して数値比較
                    let mut na = String::new();
                    while let Some(&(_, d)) = ia.peek() {
                        if !d.is_ascii_digit() {
                            break;
                        }
                        na.push(d);
                        ia.next();
                    }
                    let mut nb = String::new();
                    while let Some(&(_, d)) = ib.peek() {
                        if !d.is_ascii_digit() {
                            break;
                        }
                        nb.push(d);
                        ib.next();
                    }
                    let ta = na.trim_start_matches('0');
                    let tb = nb.trim_start_matches('0');
                    // まず有効桁数（=数値の大きさ）で比較
                    let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                    // 数値が等しい場合は元の文字列順（"001" < "01" < "1"）
                    let ord = na.cmp(&nb);
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                } else {
                    let ord = x.cmp(&y);
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                    ia.next();
                    ib.next();
                }
            }
        }
    }
}

/// 画像ファイル一覧を自然順（連番対応）でソートする。
fn sort_image_files(files: &mut [PathBuf]) {
    files.sort_by(|a, b| {
        let na = a
            .file_name()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let nb = b
            .file_name()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        natural_cmp(&na, &nb).then_with(|| a.cmp(b))
    });
}

pub fn collect_siblings(path: &Path) -> Vec<PathBuf> {
    let parent = match path.parent() {
        Some(p) => p,
        None => return vec![path.to_path_buf()],
    };
    let mut files: Vec<PathBuf> = match std::fs::read_dir(parent) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && is_supported(p))
            .collect(),
        Err(_) => return vec![path.to_path_buf()],
    };
    sort_image_files(&mut files);
    if files.is_empty() {
        vec![path.to_path_buf()]
    } else {
        files
    }
}

pub fn first_image_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && is_supported(p))
        .collect();
    sort_image_files(&mut files);
    files.into_iter().next()
}

/// テクスチャ上限に収まるよう高速に縮小する。収まっていればそのまま返す。
/// imageクレートのTriangle（遅い）の代わりにSIMDのfirを使う。
fn downscale_to_cap(rgba: image::RgbaImage, max_side: usize) -> image::RgbaImage {
    let (w, h) = (rgba.width(), rgba.height());
    if w as usize <= max_side && h as usize <= max_side {
        return rgba;
    }
    let s = max_side as f32 / w.max(h) as f32;
    let (nw, nh) = (
        ((w as f32 * s).round() as u32).max(1),
        ((h as f32 * s).round() as u32).max(1),
    );
    let src = image::DynamicImage::ImageRgba8(rgba);
    let mut dst = fast_image_resize::images::Image::new(nw, nh, fast_image_resize::PixelType::U8x4);
    let mut resizer = fast_image_resize::Resizer::new();
    let options = fast_image_resize::ResizeOptions::new().resize_alg(
        fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Bilinear),
    );
    resizer
        .resize(&src, &mut dst, &options)
        .expect("fir resize");
    image::RgbaImage::from_raw(nw, nh, dst.buffer().to_vec()).expect("fir buffer")
}

/// 先読み済み画像（表示可能サイズ＋元サイズ）。
pub struct CachedImage {
    pub rgba: image::RgbaImage,
    pub orig: (u32, u32),
}

struct PreloadReq {
    path: PathBuf,
    max_side: usize,
}

struct CacheInner {
    map: HashMap<PathBuf, CachedImage>,
    pending: HashSet<PathBuf>,
}

/// 前後画像をバックグラウンドでデコードする先読みローダー。
/// UIスレッドを止めないため、重いデコード＋縮小は専用スレッドで行う。
pub struct ImageLoader {
    tx: mpsc::Sender<PreloadReq>,
    inner: Arc<Mutex<CacheInner>>,
}

impl ImageLoader {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<PreloadReq>();
        let inner = Arc::new(Mutex::new(CacheInner {
            map: HashMap::new(),
            pending: HashSet::new(),
        }));
        // 画像ごとに独立なので複数ワーカーで並列デコードする
        let workers = std::thread::available_parallelism()
            .map(|n| n.get().min(4))
            .unwrap_or(2)
            .max(1);
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..workers {
            let rx = Arc::clone(&rx);
            let worker_inner = Arc::clone(&inner);
            std::thread::Builder::new()
                .name(format!("image-preloader-{i}"))
                .spawn(move || loop {
                    let req = { rx.lock().map(|g| g.recv()).unwrap_or(Err(mpsc::RecvError)) };
                    let Ok(req) = req else {
                        break;
                    };
                    let already = worker_inner
                        .lock()
                        .map(|g| g.map.contains_key(&req.path))
                        .unwrap_or(true);
                    if already {
                        worker_inner
                            .lock()
                            .map(|mut g| g.pending.remove(&req.path))
                            .ok();
                        continue;
                    }
                    let result = decode_image(&req.path).ok().map(|img| {
                        let orig = (img.width(), img.height());
                        let rgba = downscale_to_cap(img.to_rgba8(), req.max_side);
                        CachedImage { rgba, orig }
                    });
                    if let Ok(mut g) = worker_inner.lock() {
                        g.pending.remove(&req.path);
                        if let Some(cached) = result {
                            g.map.insert(req.path, cached);
                        }
                    }
                })
                .expect("preloader thread spawn");
        }
        Self { tx, inner }
    }

    /// キャッシュから取り出す（ヒットしたら所有権を移す）。
    pub fn take(&self, path: &Path) -> Option<CachedImage> {
        self.inner.lock().ok()?.map.remove(path)
    }

    pub fn is_cached(&self, path: &Path) -> bool {
        self.inner
            .lock()
            .map(|g| g.map.contains_key(path))
            .unwrap_or(false)
    }

    /// 先読み要求。キャッシュ済み・要求済みなら送らない。
    pub fn request(&self, path: PathBuf, max_side: usize) {
        {
            let mut g = match self.inner.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if g.map.contains_key(&path) || !g.pending.insert(path.clone()) {
                return;
            }
        }
        if self
            .tx
            .send(PreloadReq {
                path: path.clone(),
                max_side,
            })
            .is_err()
        {
            if let Ok(mut g) = self.inner.lock() {
                g.pending.remove(&path);
            }
        }
    }

    /// 指定パス以外をキャッシュから捨てる。pendingは完了時に整理される。
    pub fn prune(&self, keep: &[PathBuf]) {
        if let Ok(mut g) = self.inner.lock() {
            g.map.retain(|p, _| keep.contains(p));
        }
    }
}

impl Default for ImageLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// 日本語フォントをシステムから読み込む。
/// egui既定フォントには日本語グリフが無いため、C:\Windows\Fonts から探して先頭に登録する。
/// 見つからなければ既定のまま（□表示の可能性あり）。
pub fn setup_jp_font(ctx: &egui::Context) {
    let candidates = [
        "NotoSansJP-VF.ttf",
        "yumin.ttf",
        "yumindb.ttf",
        "yuminl.ttf",
        "NotoSerifJP-VF.ttf",
    ];
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    for name in candidates {
        let path = std::path::Path::new(&windir).join("Fonts").join(name);
        if let Ok(bytes) = std::fs::read(&path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "japanese".to_owned(),
                egui::FontData::from_owned(bytes).into(),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&family) {
                    list.insert(0, "japanese".to_owned());
                }
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

pub struct ViewerApp {
    files: Vec<PathBuf>,
    index: usize,
    base: Option<image::RgbaImage>,
    texture: Option<egui::TextureHandle>,
    load_error: Option<String>,
    zoom: f32,
    fit: bool,
    rotation: u8, // 0..3 : 時計回り90度 × n
    fullscreen: bool,
    slideshow: bool,
    slideshow_secs: f32,
    last_advance: Option<Instant>,
    status_msg: String,
    pan_offset: egui::Vec2,
    wheel_accum: f32,
    last_wheel_nav: Option<Instant>,
    view_avail: egui::Vec2,
    view_img: egui::Vec2,
    peek_saved: Option<PeekState>,
    loader: ImageLoader,
    tex_cap: usize,
    orig_dims: Option<(u32, u32)>,
}

/// 中ボタン押下中の等倍覗き見（ルーペ）用に退避する表示状態。
#[derive(Clone, Copy)]
struct PeekState {
    fit: bool,
    zoom: f32,
    pan: egui::Vec2,
}

impl ViewerApp {
    pub fn new(initial: Option<PathBuf>) -> Self {
        let mut app = Self {
            files: Vec::new(),
            index: 0,
            base: None,
            texture: None,
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
            last_wheel_nav: None,
            view_avail: egui::vec2(1000.0, 700.0),
            view_img: egui::vec2(800.0, 600.0),
            peek_saved: None,
            loader: ImageLoader::new(),
            tex_cap: 2048,
            orig_dims: None,
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

    fn oriented_image(&self) -> Option<image::RgbaImage> {
        let base = self.base.as_ref()?;
        let img = match self.rotation % 4 {
            0 => base.clone(),
            1 => image::imageops::rotate90(base),
            2 => image::imageops::rotate180(base),
            3 => image::imageops::rotate270(base),
            _ => base.clone(),
        };
        Some(img)
    }

    fn rebuild_texture(&mut self, ctx: &egui::Context) {
        // GPU上限は環境で変わり得るため都度取得
        self.tex_cap = ctx.input(|i| i.max_texture_side).max(512);
        if let Some(rgba) = self.oriented_image() {
            // 通常は先読み/読込時に縮小済み。念のため上限ガード。
            let rgba = downscale_to_cap(rgba, self.tex_cap);
            let (w, h) = (rgba.width() as usize, rgba.height() as usize);
            let pixels = rgba.into_raw();
            let color = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
            let name = self
                .current_path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "image".to_owned());
            self.texture = Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR));
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

    fn load_current(&mut self, ctx: Option<&egui::Context>) {
        self.load_error = None;
        self.texture = None;
        self.base = None;
        self.orig_dims = None;
        let Some(path) = self.current_path().map(|p| p.to_path_buf()) else {
            return;
        };
        // 先読みキャッシュ優先（移動時のもたつきを消す）
        if let Some(cached) = self.loader.take(&path) {
            self.base = Some(cached.rgba);
            self.orig_dims = Some(cached.orig);
            self.status_msg.clear();
        } else {
            match decode_image(&path) {
                Ok(dyn_img) => {
                    let orig = (dyn_img.width(), dyn_img.height());
                    let rgba = downscale_to_cap(dyn_img.to_rgba8(), self.tex_cap);
                    self.base = Some(rgba);
                    self.orig_dims = Some(orig);
                    self.status_msg.clear();
                }
                Err(e) => {
                    self.load_error = Some(format!(
                        "開けませんでした: {} ({})",
                        path.file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        e
                    ));
                }
            }
        }
        if let Some(ctx) = ctx {
            self.rebuild_texture(ctx);
        }
        self.request_preloads();
    }

    /// 前後画像の先読みを要求し、遠い画像をキャッシュから捨てる。
    /// 前後2枚ずつ（最大5枚保持）でホイール連打にも追従する。
    fn request_preloads(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len();
        let at = |i: usize| self.files[i % n].clone();
        let next = at(self.index + 1);
        let prev = at(self.index + n - 1);
        let next2 = at(self.index + 2);
        let prev2 = at(self.index + n - 2);
        let current = self.files[self.index].clone();
        self.loader.prune(&[
            current,
            next.clone(),
            prev.clone(),
            next2.clone(),
            prev2.clone(),
        ]);
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
        self.rebuild_texture(ctx);
    }

    pub fn rotate_ccw(&mut self, ctx: &egui::Context) {
        if self.base.is_none() {
            return;
        }
        self.rotation = (self.rotation + 3) % 4;
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
        let disp = egui::vec2(
            (iw * new_scale).clamp(1.0, 20000.0),
            (ih * new_scale).clamp(1.0, 20000.0),
        );
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
    pub fn image_dims(&self) -> Option<(u32, u32)> {
        self.orig_dims
    }
    pub fn pan(&self) -> egui::Vec2 {
        self.pan_offset
    }
    pub fn is_preloaded(&self, path: &Path) -> bool {
        self.loader.is_cached(path)
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let mut dlg = rfd::FileDialog::new().set_title("画像を開く");
        dlg = dlg.add_filter("画像", SUPPORTED_EXTS);
        if let Some(path) = dlg.pick_file() {
            self.open_path(path, Some(ctx));
        }
    }

    fn apply_dropped(&mut self, ctx: &egui::Context) {
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

    fn handle_keys(&mut self, ctx: &egui::Context) {
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
        if ctx.input(|i| i.key_pressed(egui::Key::Num0) || i.key_pressed(egui::Key::Num0)) {
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

    /// ホイールで前後画像へ移動（1ノッチ=±40で1枚）。
    /// 中ボタン押下中の誤回転では移動しない。
    fn handle_wheel_nav(&mut self, ctx: &egui::Context) {
        if self.files.len() < 2 {
            self.wheel_accum = 0.0;
            return;
        }
        if ctx.input(|i| i.pointer.middle_down()) {
            self.wheel_accum = 0.0;
            return;
        }
        let y = ctx.input(|i| i.smooth_scroll_delta.y);
        if y == 0.0 {
            return;
        }
        if let Some(t) = self.last_wheel_nav {
            if t.elapsed() < Duration::from_millis(150) {
                self.wheel_accum = 0.0;
                return;
            }
        }
        self.wheel_accum += y;
        if self.wheel_accum <= -40.0 {
            // 手前（下）回し = 次へ
            self.next(ctx);
            self.wheel_accum = 0.0;
            self.last_wheel_nav = Some(Instant::now());
        } else if self.wheel_accum >= 40.0 {
            // 奥（上）回し = 前へ
            self.prev(ctx);
            self.wheel_accum = 0.0;
            self.last_wheel_nav = Some(Instant::now());
        }
    }

    fn tick_slideshow(&mut self, ctx: &egui::Context) {
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

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // GPU上限を最新化（先読みの縮小サイズに使う）
        self.tex_cap = ctx.input(|i| i.max_texture_side).max(512);
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
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        // ---- ツールバー ----
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("開く").clicked() {
                    self.open_dialog(ctx);
                }
                ui.separator();
                let has = !self.files.is_empty();
                ui.add_enabled_ui(has, |ui| {
                    if ui.button("前へ").clicked() {
                        self.prev(ctx);
                    }
                    if ui.button("次へ").clicked() {
                        self.next(ctx);
                    }
                });
                ui.separator();
                ui.add_enabled_ui(self.texture.is_some(), |ui| {
                    if ui.button("-").clicked() {
                        self.zoom_centered(1.0 / 1.25);
                    }
                    if ui.button("+").clicked() {
                        self.zoom_centered(1.25);
                    }
                    if ui.button("フィット").clicked() {
                        self.fit = true;
                        self.pan_offset = egui::Vec2::ZERO;
                    }
                    if ui.button("1:1").clicked() {
                        self.fit = false;
                        self.zoom = 1.0;
                    }
                    if ui.button("左回転").on_hover_text("Shift+R").clicked() {
                        self.rotate_ccw(ctx);
                    }
                    if ui.button("右回転").on_hover_text("R").clicked() {
                        self.rotate_cw(ctx);
                    }
                });
                ui.separator();
                let label = if self.slideshow { "停止" } else { "再生" };
                if ui
                    .button(label)
                    .on_hover_text("スライドショー (Space)")
                    .clicked()
                {
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
                let fs_label = if self.fullscreen {
                    "全画面解除"
                } else {
                    "全画面"
                };
                if ui.button(fs_label).on_hover_text("全画面 (F)").clicked() {
                    self.fullscreen = !self.fullscreen;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                }
            });
        });

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
                    ui.label(format!("{}  ({}/{})", name, self.index + 1, self.files.len()));
                    if let Some((ow, oh)) = self.orig_dims {
                        let (w, h) = if self.rotation % 2 == 1 { (oh, ow) } else { (ow, oh) };
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
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("ホイール:前後 / 中ボタン押下中のみ等倍 / マウス:視点移動 / R:回転 / F:全画面 / Space:再生");
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
            let avail = view_rect.size();
            let (iw, ih) = {
                let s = handle.size();
                (s[0] as f32, s[1] as f32)
            };
            self.view_avail = avail;
            self.view_img = egui::vec2(iw, ih);

            let hover = ctx.input(|i| i.pointer.hover_pos());
            let hover_in_view = hover.map(|p| view_rect.contains(p)).unwrap_or(false);
            let mid_down = ctx.input(|i| i.pointer.middle_down());

            // 中ボタン押下中だけ等倍覗き見（ルーペ）。離したら元の表示に戻る。
            // ホバー連動で覗き見位置がカーソルに追従する。
            if mid_down && hover_in_view {
                self.begin_peek(avail, iw, ih);
            } else if !mid_down {
                self.end_peek();
            }

            // 描画サイズ
            let scale = self.current_scale(avail, iw, ih);
            let disp = egui::vec2(
                (iw * scale).clamp(1.0, 20000.0),
                (ih * scale).clamp(1.0, 20000.0),
            );

            // マウス位置連動で視点移動（ドラッグ不要）。
            // 右端に寄せると画像右側、左端で左側が見える。覗き見中も追従する。
            if self.fit {
                self.pan_offset = egui::Vec2::ZERO;
            } else if hover_in_view && avail.x > 0.0 && avail.y > 0.0 {
                let rel = hover.unwrap() - view_rect.min;
                let tx = (rel.x / avail.x).clamp(0.0, 1.0);
                let ty = (rel.y / avail.y).clamp(0.0, 1.0);
                self.pan_offset = Self::pan_for_hover(avail, disp, tx, ty);
            }
            self.clamp_pan(avail, disp);
            let center = view_rect.center() + self.pan_offset;
            let img_rect = egui::Rect::from_center_size(center, disp);
            ui.put(img_rect, egui::Image::new(&handle).fit_to_exact_size(disp));
        });
    }
}

pub fn run() -> eframe::Result<()> {
    let initial: Option<PathBuf> = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir() || is_supported(p) || p.exists());

    let mut app = ViewerApp::new(initial);
    // 起動時に file が決まっている場合、先にデコードだけしておく
    // （texture は update 内の初回フレームで作り直されるが、base があれば表示できる）
    if app.base.is_none() && app.current_path().is_some() {
        app.load_current(None);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([640.0, 480.0])
            .with_drag_and_drop(true),
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
    fn fit_scale_uses_min_ratio() {
        let app = ViewerApp::new(None);
        // 2000x1000 を 1000x700 にフィット → min(0.5, 0.7) = 0.5
        let s = app.current_scale(egui::vec2(1000.0, 700.0), 2000.0, 1000.0);
        assert!(approx(s, 0.5), "got {s}");
        // 縦長画像 500x2000 → min(2.0, 0.35) = 0.35
        let s = app.current_scale(egui::vec2(1000.0, 700.0), 500.0, 2000.0);
        assert!(approx(s, 0.35), "got {s}");
    }

    #[test]
    fn zoom_at_keeps_cursor_point_stationary() {
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
    fn zoom_clamps_to_range() {
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
    fn hover_pan_endpoints() {
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
    fn clamp_pan_bounds() {
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
    fn natural_order_numeric_names() {
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
    fn sort_image_files_numeric_order() {
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
    fn supported_extensions() {
        use std::path::Path;
        assert!(is_supported(Path::new("a.JPG")));
        assert!(is_supported(Path::new("a.webp")));
        assert!(!is_supported(Path::new("a.txt")));
        assert!(!is_supported(Path::new("a")));
    }

    #[test]
    fn downscale_respects_cap() {
        let big = image::RgbaImage::from_pixel(3000, 2000, image::Rgba([1, 2, 3, 255]));
        let small = downscale_to_cap(big, 2048);
        assert_eq!((small.width(), small.height()), (2048, 1365));
        let tiny = image::RgbaImage::from_pixel(100, 80, image::Rgba([1, 2, 3, 255]));
        let same = downscale_to_cap(tiny, 2048);
        assert_eq!((same.width(), same.height()), (100, 80));
    }
}
