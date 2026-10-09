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
/// リサイズ失敗はパニックではなくエラーとして返す（ワーカーを巻き込まないため）。
pub(crate) fn downscale_to_cap(
    rgba: image::RgbaImage,
    max_side: usize,
) -> Result<image::RgbaImage, String> {
    let (w, h) = (rgba.width(), rgba.height());
    if w as usize <= max_side && h as usize <= max_side {
        return Ok(rgba);
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
        .map_err(|e| format!("resize failed: {e}"))?;
    image::RgbaImage::from_raw(nw, nh, dst.buffer().to_vec())
        .ok_or_else(|| "resize produced a mismatched buffer".to_owned())
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

    /// Minimal PNG claiming absurd dimensions: header only, no pixel data.
    fn bomb_png(path: &Path, w: u32, h: u32) {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &byte in data {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    let mask = crc & 1;
                    crc >>= 1;
                    if mask != 0 {
                        crc ^= 0xEDB8_8320;
                    }
                }
            }
            !crc
        }
        fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let mut covered = Vec::from(&kind[..]);
            covered.extend_from_slice(data);
            out.extend_from_slice(&crc32(&covered).to_be_bytes());
            out
        }
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        // 8-bit truecolor, default compression/filter/interlace.
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(chunk(b"IHDR", &ihdr));
        // Valid but empty zlib stream: construction succeeds, pixel data is
        // never touched because the dimension check rejects first.
        png.extend(chunk(
            b"IDAT",
            &[0x78, 0x01, 0x01, 0x00, 0xFE, 0xFF, 0x00, 0x00, 0x00, 0x01],
        ));
        png.extend(chunk(b"IEND", &[]));
        std::fs::write(path, png).unwrap();
    }

    #[test]
    fn oversized_dimensions_are_rejected_before_decoding() {
        let dir = std::env::temp_dir().join(format!("iv-bomb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bomb.png");
        // 40000x40000 claims ~12 GiB: must fail fast on the header check alone.
        bomb_png(&path, 40_000, 40_000);
        let start = std::time::Instant::now();
        let error = decode_image(&path).expect_err("image bomb must be rejected");
        assert!(
            matches!(error, image::ImageError::Limits(_)),
            "unexpected error: {error}"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "took too long: decoded instead of rejecting?"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
