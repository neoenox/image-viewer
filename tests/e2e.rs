//! 画像ビューワーのE2Eテスト。
//!
//! 実バイナリ相当の `ViewerApp` をヘッドレス（`egui::Context::default()`）で駆動し、
//! 一時フォルダに生成した実画像フィクスチャに対して
//! 「開く→移動→回転→ズーム→エラー系」の一連の操作を検証する。
//! GUIイベント注入（キー・マウス）は対象外。ネイティブダイアログも開かない。

use image_viewer::{ViewerApp, collect_siblings, decode_image, is_supported, setup_jp_font};
use std::path::{Path, PathBuf};

/// テストごとに独立したフィクスチャフォルダを作る。tagで並列実行時の衝突を避ける。
fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "image-viewer-e2e-{}-{}",
        std::process::id(),
        tag
    ));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).ok();
    }
    std::fs::create_dir_all(&dir).unwrap();

    fn solid(path: &Path, w: u32, h: u32, px: image::Rgba<u8>) {
        let buf = image::RgbaImage::from_pixel(w, h, px);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "jpg" || ext == "jpeg" {
            // JPEGはアルファ非対応のためRGBに変換
            image::DynamicImage::ImageRgba8(buf)
                .to_rgb8()
                .save(path)
                .unwrap();
        } else {
            buf.save(path).unwrap();
        }
    }
    use image::Rgba;
    // 名前順ソートで決定的な並びになるよう命名
    solid(&dir.join("a.png"), 100, 80, Rgba([200, 30, 30, 255]));
    solid(&dir.join("b.jpg"), 60, 60, Rgba([30, 30, 200, 255]));
    // 誤ラベル: 中身JPEG・拡張子.png
    let mislabeled =
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(40, 30, Rgba([30, 200, 30, 255])));
    let mut bytes = Vec::new();
    mislabeled
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
        .unwrap();
    std::fs::write(dir.join("c.png"), &bytes).unwrap();
    // 壊れたファイル（拡張子は対応形式）
    std::fs::write(dir.join("d.png"), b"not an image at all").unwrap();
    // 未対応拡張子（一覧から除外される）
    std::fs::write(dir.join("e.txt"), b"hello").unwrap();
    // テクスチャ上限(2048)超えの巨大画像。jpgで保存して高速化。
    solid(&dir.join("z-big.jpg"), 3000, 2000, Rgba([120, 120, 120, 255]));
    dir
}

fn cleanup(dir: &Path) {
    std::fs::remove_dir_all(dir).ok();
}

fn headless_ctx() -> egui::Context {
    egui::Context::default()
}

#[test]
fn scan_filters_and_sorts() {
    let dir = fixture_dir("scan");
    let files = collect_siblings(&dir.join("a.png"));
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["a.png", "b.jpg", "c.png", "d.png", "z-big.jpg"]);
    assert!(is_supported(Path::new("x.PNG")));
    assert!(!is_supported(Path::new("x.txt")));
    cleanup(&dir);
}

#[test]
fn open_navigate_wraps_around() {
    let dir = fixture_dir("nav");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("a.png"), Some(&ctx));

    assert_eq!(app.file_count(), 5);
    assert_eq!(app.index(), 0);
    assert_eq!(app.image_dims(), Some((100, 80)));
    assert!(app.has_texture());
    assert_eq!(app.texture_size(), Some([100, 80]));
    assert_eq!(app.load_error(), None);

    app.next(&ctx);
    assert_eq!(app.index(), 1);
    assert_eq!(app.image_dims(), Some((60, 60)));

    // 末尾まで進めて一周
    app.next(&ctx); // c.png（誤ラベルJPEGも開ける）
    assert_eq!(app.index(), 2);
    assert_eq!(app.image_dims(), Some((40, 30)));
    assert_eq!(app.load_error(), None);

    app.next(&ctx); // d.png（壊れている）
    assert_eq!(app.index(), 3);
    assert!(!app.has_texture());
    assert!(app.load_error().is_some());

    app.next(&ctx); // z-big.jpg
    assert_eq!(app.index(), 4);
    assert!(app.has_texture());

    app.next(&ctx); // 先頭へ一周
    assert_eq!(app.index(), 0);
    assert_eq!(app.image_dims(), Some((100, 80)));

    app.prev(&ctx); // 末尾へ逆周り
    assert_eq!(app.index(), 4);
    cleanup(&dir);
}

#[test]
fn mislabeled_and_corrupt_files() {
    let dir = fixture_dir("decode");
    // 中身JPEG・拡張子png
    let img = decode_image(&dir.join("c.png")).expect("mislabeled file should decode");
    assert_eq!((img.width(), img.height()), (40, 30));
    // 壊れたファイルはエラー
    assert!(decode_image(&dir.join("d.png")).is_err());
    cleanup(&dir);
}

#[test]
fn oversized_image_fits_texture_limit() {
    // 3000x2000画像もテクスチャ上限（既定コンテキスト=2048）内に収まり、
    // debugビルドの上限アサートに引っかからないことの回帰テスト。
    let dir = fixture_dir("big");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("z-big.jpg"), Some(&ctx));
    assert!(app.has_texture());
    let [w, h] = app.texture_size().unwrap();
    assert!(w <= 2048 && h <= 2048, "texture was {w}x{h}");
    assert_eq!((w, h), (2048, 1365));
    // 元画像サイズの記録はフル解像度のまま
    assert_eq!(app.image_dims(), Some((3000, 2000)));
    cleanup(&dir);
}

#[test]
fn rotate_swaps_dimensions() {
    let dir = fixture_dir("rotate");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("a.png"), Some(&ctx)); // 100x80
    app.rotate_cw(&ctx);
    assert_eq!(app.rotation_steps(), 1);
    assert_eq!(app.texture_size(), Some([80, 100]));
    app.rotate_ccw(&ctx);
    assert_eq!(app.rotation_steps(), 0);
    assert_eq!(app.texture_size(), Some([100, 80]));
    // 4回転で元に戻る
    for _ in 0..4 {
        app.rotate_cw(&ctx);
    }
    assert_eq!(app.rotation_steps(), 0);
    assert_eq!(app.texture_size(), Some([100, 80]));
    cleanup(&dir);
}

#[test]
fn toggle_fit_zoom_logic() {
    // 中クリック相当のトグルがヘッドレスでも正しく動くこと。
    // （実機の不具合切り分け用：ここが通れば残るは入力検出側）
    let dir = fixture_dir("toggle");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("z-big.jpg"), Some(&ctx));
    assert!(app.is_fit());

    let [tw, th] = app.texture_size().unwrap();
    let avail = egui::vec2(1000.0, 700.0);
    let center = egui::pos2(500.0, 350.0);
    let cursor = egui::pos2(700.0, 400.0);

    app.toggle_fit_zoom(cursor, center, avail, tw as f32, th as f32);
    assert!(!app.is_fit());
    assert!((app.zoom_level() - 1.0).abs() < 1e-3);

    app.toggle_fit_zoom(cursor, center, avail, tw as f32, th as f32);
    assert!(app.is_fit());
    assert!((app.pan().x, app.pan().y) == (0.0, 0.0));
    cleanup(&dir);
}

#[test]
fn navigation_resets_view_state() {
    let dir = fixture_dir("reset");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("a.png"), Some(&ctx));
    app.rotate_cw(&ctx);
    app.zoom_centered(2.0);
    assert_eq!(app.rotation_steps(), 1);
    assert!(!app.is_fit());
    app.next(&ctx);
    assert_eq!(app.rotation_steps(), 0);
    assert!(app.is_fit());
    assert!((app.zoom_level() - 1.0).abs() < 1e-3);
    assert!((app.pan().x, app.pan().y) == (0.0, 0.0));
    cleanup(&dir);
}

#[test]
fn momentary_zoom_while_holding() {
    // 中ボタン押下中の一時ズーム。上移動で拡大・下移動で縮小・無移動で不変。
    let dir = fixture_dir("moment");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("a.png"), Some(&ctx)); // 100x80
    assert!(app.is_fit());

    let avail = egui::vec2(1000.0, 700.0);
    let center = egui::pos2(500.0, 350.0);
    let cursor = egui::pos2(600.0, 400.0);

    app.momentary_zoom(-100.0, cursor, center, avail, 100.0, 80.0);
    assert!(!app.is_fit());
    let z1 = app.zoom_level();
    assert!(z1 > 8.0, "zoom was {z1}"); // fit倍率8.0から拡大

    app.momentary_zoom(100.0, cursor, center, avail, 100.0, 80.0);
    assert!(app.zoom_level() < z1, "zoom was {}", app.zoom_level());

    let z2 = app.zoom_level();
    app.momentary_zoom(0.0, cursor, center, avail, 100.0, 80.0);
    assert!((app.zoom_level() - z2).abs() < 1e-6);
    cleanup(&dir);
}

#[test]
fn peek_zoom_while_holding() {
    // 中ボタン押下中だけ等倍、離したら元の表示に戻る。
    let dir = fixture_dir("peek");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("z-big.jpg"), Some(&ctx)); // 3000x2000
    assert!(app.is_fit());

    let avail = egui::vec2(1000.0, 700.0);
    let [tw, th] = app.texture_size().unwrap();

    // 押下 → 等倍へ
    assert!(app.begin_peek(avail, tw as f32, th as f32));
    assert!(!app.is_fit());
    assert!((app.zoom_level() - 1.0).abs() < 1e-3);
    // 押下中の再呼び出しは維持のみ
    assert!(!app.begin_peek(avail, tw as f32, th as f32));
    assert!((app.zoom_level() - 1.0).abs() < 1e-3);

    // 解放 → フィットに復帰
    assert!(app.end_peek());
    assert!(app.is_fit());
    assert!(!app.end_peek());

    // すでに等倍以上（小画像のフィット表示）は覗き見なし
    app.open_path(dir.join("a.png"), Some(&ctx)); // 100x80 → fit倍率8.0
    assert!(!app.begin_peek(avail, 100.0, 80.0));
    assert!(app.is_fit());
    cleanup(&dir);
}

#[test]
fn peek_follows_cursor_while_holding() {
    // 覗き見中はホバー位置に視点が追従する（update内の合成のAPIレベル再現）。
    let dir = fixture_dir("peekfollow");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("z-big.jpg"), Some(&ctx));
    let avail = egui::vec2(1000.0, 700.0);
    let [tw, th] = app.texture_size().unwrap();
    let (iw, ih) = (tw as f32, th as f32);

    assert!(app.begin_peek(avail, iw, ih));
    // カーソルを右下へ → 画像右下が見える位置へ
    let disp = egui::vec2(iw, ih); // zoom=1.0
    let pan = ViewerApp::pan_for_hover(avail, disp, 1.0, 1.0);
    assert!(pan.x < 0.0 && pan.y < 0.0, "got {pan:?}");
    // カーソルを左上へ → 画像左上が見える位置へ
    let pan = ViewerApp::pan_for_hover(avail, disp, 0.0, 0.0);
    assert!(pan.x > 0.0 && pan.y > 0.0, "got {pan:?}");
    // 解放 → 復帰
    assert!(app.end_peek());
    assert!(app.is_fit());
    cleanup(&dir);
}

#[test]
fn preloads_neighbors_in_background() {
    // 前後画像がバックグラウンドで揃い、次の移動がキャッシュから即時表示されること。
    let dir = fixture_dir("preload");
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("a.png"), Some(&ctx));

    let targets = [dir.join("b.jpg"), dir.join("z-big.jpg")];
    let start = std::time::Instant::now();
    while !targets.iter().all(|p| app.is_preloaded(p)) {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(15),
            "preload timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    app.next(&ctx);
    assert_eq!(app.index(), 1);
    assert!(app.has_texture());
    assert_eq!(app.load_error(), None);
    cleanup(&dir);
}

#[test]
fn numeric_filenames_sort_naturally() {
    // 1,10,2.. ではなく 1,2,..,10 の順で並ぶこと（E2E）。
    let dir = std::env::temp_dir().join(format!("image-viewer-e2e-{}-num", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).ok();
    }
    std::fs::create_dir_all(&dir).unwrap();
    for n in ["1.png", "10.png", "2.png", "9.png", "3.png"] {
        image::RgbaImage::from_pixel(10, 10, image::Rgba([9, 9, 9, 255]))
            .save(dir.join(n))
            .unwrap();
    }
    let files = collect_siblings(&dir.join("1.png"));
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["1.png", "2.png", "3.png", "9.png", "10.png"]);

    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    app.open_path(dir.join("1.png"), Some(&ctx));
    assert_eq!(app.index(), 0);
    app.next(&ctx);
    assert!(app.current_path().unwrap().ends_with("2.png"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_app_operations_are_safe() {
    let ctx = headless_ctx();
    let mut app = ViewerApp::new(None);
    assert_eq!(app.file_count(), 0);
    assert!(app.current_path().is_none());
    // 空状態での操作はpanicしない
    app.next(&ctx);
    app.prev(&ctx);
    app.goto(3, &ctx);
    app.rotate_cw(&ctx);
    app.rotate_ccw(&ctx);
    assert!(!app.has_texture());
}

#[test]
fn jp_font_setup_does_not_panic() {
    let ctx = headless_ctx();
    setup_jp_font(&ctx);
}
