//! High-quality magnification: when the view is zoomed past the source pixels, the
//! visible part is resampled with Lanczos3 to exactly the on-screen pixel size on a
//! background thread, replacing the GPU's bilinear stretch once it is ready.

use crate::sync::{wait_recover, LockRecover};
use std::sync::{Arc, Condvar, Mutex};

/// Identifies one resampled view; a result is shown only while the view still matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UpscaleKey {
    pub generation: u64,
    pub rotation: u8,
    /// Identity of the source pixels (detail region or full-resolution base).
    pub source: [u32; 4],
    /// Crop in source pixels, in 1/16 px so tiny float noise does not re-trigger work.
    pub crop: [i64; 4],
    /// Output size in physical pixels.
    pub out: (u32, u32),
    /// Rounds of blur for the unsharp mask applied after resampling; 0 = none.
    pub sharpen: u8,
}
impl UpscaleKey {
    pub fn crop_f64(&self) -> [f64; 4] {
        self.crop.map(|v| v as f64 / 16.0)
    }
    pub fn quantize(v: f32) -> i64 {
        (v * 16.0).round() as i64
    }
}

struct Request {
    key: UpscaleKey,
    source: Arc<image::RgbaImage>,
}
#[derive(Default)]
struct Slot {
    request: Option<Request>,
    /// `None` image: the request could not be resampled (e.g. empty crop).
    result: Option<(UpscaleKey, Option<image::RgbaImage>)>,
    stopping: bool,
}

pub(crate) struct Upscaler {
    shared: Arc<(Mutex<Slot>, Condvar)>,
}
impl Default for Upscaler {
    fn default() -> Self {
        Self::new()
    }
}
impl Upscaler {
    pub fn new() -> Self {
        let shared = Arc::new((Mutex::new(Slot::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("image-upscale".into())
            .spawn(move || loop {
                let (lock, wake) = &*worker;
                let request = {
                    let mut slot = lock.lock_recover();
                    loop {
                        if slot.stopping {
                            return;
                        }
                        if let Some(request) = slot.request.take() {
                            break request;
                        }
                        slot = wait_recover(wake, slot);
                    }
                };
                // A codec panic must not kill the worker; report it as a failed resample.
                let image = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    resample(&request.source, &request.key)
                }))
                .unwrap_or(None);
                let mut slot = lock.lock_recover();
                // A newer request supersedes this result; keep only the latest.
                // Failures are reported too, so the caller stops waiting for them.
                if slot.request.is_none() {
                    slot.result = Some((request.key, image));
                }
            })
            .expect("upscale worker");
        Self { shared }
    }
    /// Replaces any queued request; work already running finishes and is dropped
    /// if a newer request arrived meanwhile.
    pub fn request(&self, key: UpscaleKey, source: Arc<image::RgbaImage>) {
        let mut slot = self.shared.0.lock_recover();
        slot.request = Some(Request { key, source });
        slot.result = None;
        self.shared.1.notify_one();
    }
    pub fn poll(&self) -> Option<(UpscaleKey, Option<image::RgbaImage>)> {
        self.shared.0.lock_recover().result.take()
    }
}
impl Drop for Upscaler {
    fn drop(&mut self) {
        self.shared.0.lock_recover().stopping = true;
        self.shared.1.notify_one();
    }
}

/// Lanczos3 resample of `crop` (source pixels, fractional) to `key.out` pixels.
pub(crate) fn resample(source: &image::RgbaImage, key: &UpscaleKey) -> Option<image::RgbaImage> {
    use fast_image_resize as fr;
    let (w, h) = key.out;
    if w == 0 || h == 0 {
        return None;
    }
    let [left, top, cw, ch] = key.crop_f64();
    let (sw, sh) = (f64::from(source.width()), f64::from(source.height()));
    // Snapping the view to screen pixels can overshoot the image by a fraction of
    // a source pixel; trim that, but reject crops that are really outside.
    if cw <= 0.0
        || ch <= 0.0
        || left < -1.0
        || top < -1.0
        || left + cw > sw + 1.0
        || top + ch > sh + 1.0
    {
        return None;
    }
    let (left, top) = (left.max(0.0), top.max(0.0));
    let (cw, ch) = ((cw).min(sw - left), (ch).min(sh - top));
    if cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let src = fr::images::ImageRef::new(
        source.width(),
        source.height(),
        source.as_raw(),
        fr::PixelType::U8x4,
    )
    .ok()?;
    let mut dst = fr::images::Image::new(w, h, fr::PixelType::U8x4);
    let options = fr::ResizeOptions::new()
        .resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3))
        .crop(left, top, cw, ch);
    fr::Resizer::new().resize(&src, &mut dst, &options).ok()?;
    let resized = image::RgbaImage::from_raw(w, h, dst.into_vec())?;
    Some(if key.sharpen > 0 {
        unsharp(&resized, key.sharpen)
    } else {
        resized
    })
}

/// Unsharp mask: `out = in + AMOUNT * (in - blur(in))` on the colour channels.
/// The blur is `passes` rounds of a separable 5-tap binomial kernel (sigma about
/// sqrt(passes)); callers pass more rounds the more the view is magnified, so the
/// mask acts on the width of the stretched edges. A general Gaussian blur was ~10x
/// slower on a 4K view. The strength is deliberately gentle; stronger values ring
/// around edges. Rows are processed in parallel.
const SHARPEN_AMOUNT: f32 = 0.6;
fn unsharp(image: &image::RgbaImage, passes: u8) -> image::RgbaImage {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let stride = w * 4;
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    let rows_per = h.div_ceil(workers).max(1);
    let mut blurred = image.as_raw().clone();
    for _ in 0..passes.max(1) {
        blurred = blur_once(&blurred, w, h, rows_per);
    }
    let mut out = image.clone();
    std::thread::scope(|scope| {
        let blurred = &blurred;
        for (chunk_index, chunk) in out.as_mut().chunks_mut(rows_per * stride).enumerate() {
            scope.spawn(move || {
                let base = chunk_index * rows_per * stride;
                for (i, v) in chunk.iter_mut().enumerate() {
                    if i % 4 == 3 {
                        continue;
                    }
                    let orig = f32::from(*v);
                    let blur = f32::from(blurred[base + i]);
                    *v = (orig + SHARPEN_AMOUNT * (orig - blur))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            });
        }
    });
    out
}

/// One separable [1 4 6 4 1] / 16 blur of the RGB channels (alpha copied), edges clamped.
fn blur_once(src: &[u8], w: usize, h: usize, rows_per: usize) -> Vec<u8> {
    let stride = w * 4;
    let mut tmp = vec![0u8; src.len()];
    std::thread::scope(|scope| {
        for (chunk_index, chunk) in tmp.chunks_mut(rows_per * stride).enumerate() {
            scope.spawn(move || {
                for (r, out_row) in chunk.chunks_mut(stride).enumerate() {
                    let y = chunk_index * rows_per + r;
                    let row = &src[y * stride..(y + 1) * stride];
                    for x in 0..w {
                        let at = |dx: isize| {
                            let sx = (x as isize + dx).clamp(0, w as isize - 1) as usize * 4;
                            &row[sx..sx + 3]
                        };
                        let (m2, m1, c0, p1, p2) = (at(-2), at(-1), at(0), at(1), at(2));
                        for c in 0..3 {
                            let acc = u32::from(m2[c])
                                + 4 * u32::from(m1[c])
                                + 6 * u32::from(c0[c])
                                + 4 * u32::from(p1[c])
                                + u32::from(p2[c]);
                            out_row[x * 4 + c] = ((acc + 8) / 16) as u8;
                        }
                        out_row[x * 4 + 3] = row[x * 4 + 3];
                    }
                }
            });
        }
    });
    let mut out = tmp.clone();
    let tmp = &tmp;
    std::thread::scope(|scope| {
        for (chunk_index, chunk) in out.chunks_mut(rows_per * stride).enumerate() {
            scope.spawn(move || {
                for (r, out_row) in chunk.chunks_mut(stride).enumerate() {
                    let y = chunk_index * rows_per + r;
                    let at = |dy: isize| {
                        let sy = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                        &tmp[sy * stride..(sy + 1) * stride]
                    };
                    let (m2, m1, c0, p1, p2) = (at(-2), at(-1), at(0), at(1), at(2));
                    for i in 0..stride {
                        if i % 4 == 3 {
                            continue;
                        }
                        let acc = u32::from(m2[i])
                            + 4 * u32::from(m1[i])
                            + 6 * u32::from(c0[i])
                            + 4 * u32::from(p1[i])
                            + u32::from(p2[i]);
                        out_row[i] = ((acc + 8) / 16) as u8;
                    }
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(crop: [f32; 4], out: (u32, u32)) -> UpscaleKey {
        UpscaleKey {
            generation: 1,
            rotation: 0,
            source: [0, 0, 8, 8],
            crop: crop.map(UpscaleKey::quantize),
            out,
            sharpen: 0,
        }
    }

    #[test]
    fn resample_crops_and_scales_to_the_requested_pixels() {
        // Left half black, right half white.
        let src = image::RgbaImage::from_fn(8, 8, |x, _| {
            let v = if x < 4 { 0 } else { 255 };
            image::Rgba([v, v, v, 255])
        });
        let out = resample(&src, &key([0.0, 0.0, 4.0, 4.0], (32, 32))).unwrap();
        assert_eq!(out.dimensions(), (32, 32));
        // The crop covers the black half. Lanczos also reads neighbours just outside
        // the crop (so the visible edge blends with what lies beyond it), so only
        // the last source pixel's worth of output may pick up the white side.
        for (x, _, p) in out.enumerate_pixels() {
            if x < 24 {
                assert!(p.0[0] < 16, "white leaked into the interior at x={x}");
            }
        }
        let out = resample(&src, &key([2.0, 0.0, 4.0, 8.0], (40, 80))).unwrap();
        assert_eq!(out.dimensions(), (40, 80));
        assert!(out.get_pixel(2, 40).0[0] < 16);
        assert!(out.get_pixel(37, 40).0[0] > 240);
        // Lanczos gives a short ramp at the edge rather than one hard step.
        let ramp = (0..40).filter(|&x| {
            let v = out.get_pixel(x, 40).0[0];
            v > 16 && v < 240
        });
        assert!(ramp.count() >= 2);
    }

    #[test]
    fn sharpening_steepens_edges_without_changing_flat_areas() {
        // Soft edge: a horizontal ramp from black to white over 8 pixels.
        let src = image::RgbaImage::from_fn(64, 8, |x, _| {
            let v = (((x as f32 - 28.0) / 8.0).clamp(0.0, 1.0) * 255.0) as u8;
            image::Rgba([v, v, v, 255])
        });
        let plain = unsharp(&src, 1);
        // Flat black and white areas are untouched.
        assert_eq!(plain.get_pixel(2, 4).0, [0, 0, 0, 255]);
        assert_eq!(plain.get_pixel(60, 4).0, [255, 255, 255, 255]);
        // Alpha is never modified.
        assert!(plain.pixels().all(|p| p.0[3] == 255));
        // The edge gets steeper: the maximum step between neighbours grows.
        let step = |img: &image::RgbaImage| {
            (1..64)
                .map(|x| {
                    i32::from(img.get_pixel(x, 4).0[0]) - i32::from(img.get_pixel(x - 1, 4).0[0])
                })
                .max()
                .unwrap()
        };
        assert!(
            step(&plain) > step(&src),
            "{} vs {}",
            step(&plain),
            step(&src)
        );
    }

    #[test]
    fn sharpen_flag_changes_the_resampled_output() {
        let src = image::RgbaImage::from_fn(16, 16, |x, _| {
            let v = if x < 8 { 30 } else { 220 };
            image::Rgba([v, v, v, 255])
        });
        let soft = key([0.0, 0.0, 16.0, 16.0], (64, 64));
        let mut sharp = soft.clone();
        sharp.sharpen = 1;
        let a = resample(&src, &soft).unwrap();
        let b = resample(&src, &sharp).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.dimensions(), b.dimensions());
    }

    #[test]
    fn resample_rejects_crops_outside_the_source() {
        let src = image::RgbaImage::new(8, 8);
        assert!(resample(&src, &key([6.0, 0.0, 4.0, 4.0], (8, 8))).is_none());
        // A fraction of a pixel past the edge (screen-pixel snapping) is trimmed.
        let out = resample(&src, &key([0.0, 0.0, 8.06, 8.0], (16, 16))).unwrap();
        assert_eq!(out.dimensions(), (16, 16));
        assert!(resample(&src, &key([0.0, 0.0, 4.0, 4.0], (0, 8))).is_none());
    }

    #[test]
    fn worker_returns_only_the_latest_request() {
        let up = Upscaler::new();
        let src = Arc::new(image::RgbaImage::new(64, 64));
        let first = key([0.0, 0.0, 32.0, 32.0], (64, 64));
        let second = key([16.0, 16.0, 32.0, 32.0], (96, 96));
        up.request(first, src.clone());
        up.request(second.clone(), src);
        let start = std::time::Instant::now();
        let (got, image) = loop {
            if let Some(done) = up.poll() {
                break done;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(got, second);
        assert_eq!(image.unwrap().dimensions(), (96, 96));
        // A request that cannot be resampled still reports back.
        let bad = key([100.0, 0.0, 4.0, 4.0], (8, 8));
        up.request(bad.clone(), Arc::new(image::RgbaImage::new(64, 64)));
        let start = std::time::Instant::now();
        let (got, image) = loop {
            if let Some(done) = up.poll() {
                break done;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(got, bad);
        assert!(image.is_none());
    }
}

/// `cargo test --release upscale::bench -- --ignored --nocapture`
#[cfg(test)]
mod bench {
    use super::*;

    #[test]
    #[ignore]
    fn resample_latency() {
        let mut s = 1u64;
        let src = image::RgbaImage::from_fn(2000, 2000, |_, _| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            image::Rgba([s as u8, (s >> 8) as u8, (s >> 16) as u8, 255])
        });
        for (label, out, zoom) in [
            ("window 1100x750 at 2x", (1100u32, 750u32), 2.0f32),
            ("window 1100x750 at 8x", (1100, 750), 8.0),
            ("4K 3840x2160 at 2x", (3840, 2160), 2.0),
        ] {
            for sharpen in [0u8, 1, 3] {
                let key = UpscaleKey {
                    generation: 1,
                    rotation: 0,
                    source: [0, 0, 2000, 2000],
                    crop: [10.0, 10.0, out.0 as f32 / zoom, out.1 as f32 / zoom]
                        .map(UpscaleKey::quantize),
                    out,
                    sharpen,
                };
                let mut times: Vec<f64> = (0..5)
                    .map(|_| {
                        let start = std::time::Instant::now();
                        resample(&src, &key).unwrap();
                        start.elapsed().as_secs_f64() * 1000.0
                    })
                    .collect();
                times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let mode = format!("sharp{sharpen}");
                println!("BENCH upscale {mode} {label}: median {:.1} ms", times[2]);
            }
        }
    }
}
