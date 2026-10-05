use std::path::Path;
pub(crate) const DECODE_BUDGET: u64 = 256 * 1024 * 1024;
pub fn decode_image(path: &Path) -> image::ImageResult<image::DynamicImage> {
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(DECODE_BUDGET);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (w, h) = image::ImageDecoder::dimensions(&decoder);
    // Includes formats whose decoder only partially supports allocation limits.
    if u64::from(w) * u64::from(h) * 8 > DECODE_BUDGET {
        return Err(image::ImageError::Limits(
            image::error::LimitError::from_kind(image::error::LimitErrorKind::InsufficientMemory),
        ));
    }
    // Camera and phone photos store their upright direction in EXIF. Unreadable or
    // missing metadata leaves the pixels as stored rather than failing the decode.
    let orientation = image::ImageDecoder::orientation(&mut decoder)
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    /// A 4x2 JPEG, red on the left half and blue on the right, with an EXIF orientation.
    fn jpeg_with_orientation(path: &Path, orientation: u16) {
        let img = image::RgbImage::from_fn(4, 2, |x, _| {
            if x < 2 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        // Big-endian TIFF header with one IFD entry: Orientation (0x0112), SHORT, count 1.
        let mut exif = b"MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
        exif.extend_from_slice(&orientation.to_be_bytes());
        exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100);
        encoder.set_exif_metadata(exif).unwrap();
        encoder
            .write_image(img.as_raw(), 4, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn is_red(px: &image::Rgba<u8>) -> bool {
        px[0] > 200 && px[2] < 60
    }

    #[test]
    fn exif_orientation_is_applied_on_decode() {
        let dir = std::env::temp_dir().join(format!("iv-exif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rotated.jpg");

        // 6 = rotate 90 degrees clockwise: the left (red) half ends up on top.
        jpeg_with_orientation(&path, 6);
        let img = decode_image(&path).unwrap().into_rgba8();
        assert_eq!(img.dimensions(), (2, 4));
        assert!(is_red(img.get_pixel(0, 0)) && !is_red(img.get_pixel(0, 3)));

        // 3 = rotate 180 degrees: red moves to the right.
        jpeg_with_orientation(&path, 3);
        let img = decode_image(&path).unwrap().into_rgba8();
        assert_eq!(img.dimensions(), (4, 2));
        assert!(!is_red(img.get_pixel(0, 0)) && is_red(img.get_pixel(3, 0)));

        // 1 = as stored.
        jpeg_with_orientation(&path, 1);
        let img = decode_image(&path).unwrap().into_rgba8();
        assert_eq!(img.dimensions(), (4, 2));
        assert!(is_red(img.get_pixel(0, 0)));

        std::fs::remove_dir_all(&dir).ok();
    }
}
