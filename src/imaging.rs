use std::path::Path;
pub(crate) const DECODE_BUDGET: u64 = 256 * 1024 * 1024;
pub fn decode_image(path: &Path) -> image::ImageResult<image::DynamicImage> {
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(DECODE_BUDGET);
    reader.limits(limits);
    let decoder = reader.into_decoder()?;
    let (w, h) = image::ImageDecoder::dimensions(&decoder);
    // Includes formats whose decoder only partially supports allocation limits.
    if u64::from(w) * u64::from(h) * 8 > DECODE_BUDGET {
        return Err(image::ImageError::Limits(
            image::error::LimitError::from_kind(image::error::LimitErrorKind::InsufficientMemory),
        ));
    }
    image::DynamicImage::from_decoder(decoder)
}
/// 回転を適用する（時計回り90度×n）。
pub(crate) fn rotate_rgba(base: &image::RgbaImage, rotation: u8) -> image::RgbaImage {
    match rotation % 4 {
        0 => base.clone(),
        1 => image::imageops::rotate90(base),
        2 => image::imageops::rotate180(base),
        3 => image::imageops::rotate270(base),
        _ => base.clone(),
    }
}

/// テクスチャ上限に収まるよう高速に縮小する。収まっていればそのまま返す。
/// imageクレートのTriangle（遅い）の代わりにSIMDのfirを使う。
pub(crate) fn downscale_to_cap(rgba: image::RgbaImage, max_side: usize) -> image::RgbaImage {
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
