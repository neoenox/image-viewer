use crate::imaging::{downscale_to_cap, DECODE_BUDGET};
use crate::sync::catch_panic;
use image::{AnimationDecoder, ImageDecoder};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

pub(crate) struct Frame {
    pub rgba: image::RgbaImage,
    pub delay: Duration,
    pub index: usize,
    /// Total frame count, known only after the first loop completes.
    pub total: Option<usize>,
}

/// Endless GIF player fed by a background thread (two queued frames).
///
/// Policy (#38):
/// - Loops the file until cancelled. A file open/decode error terminates the
///   stream with a sticky `Err`; playback never auto-retries. Recovery happens
///   by recreating the player (e.g. navigating images recreates it).
/// - `total` is unknown during the first loop (`None`); exact afterwards.
/// - Dropping the player signals cancellation and disconnects the channel, so
///   the worker can only be inside one bounded decode before it exits. It is
///   deliberately never joined: dropping must not block the UI thread.
pub(crate) struct Animation {
    rx: Option<mpsc::Receiver<Result<Frame, String>>>,
    cancelled: Arc<AtomicBool>,
    failed: Option<String>,
}
impl Animation {
    pub fn new(path: PathBuf, cap: usize) -> Self {
        // Two queued frames, one presented frame; frame size capped at 1024.
        let (tx, rx) = mpsc::sync_channel(2);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::Builder::new()
            .name("gif-stream".into())
            .spawn(move || {
                let mut total = None;
                // A codec panic must surface as a sticky stream error (same
                // contract as the image loader). Without this, a panicking
                // decoder would kill the worker thread and leave playback
                // hanging on "pending" forever: the disconnected channel
                // reads as None, never as Err.
                let result = catch_panic(|| -> Result<(), String> {
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                        let mut decoder =
                            image::codecs::gif::GifDecoder::new(std::io::BufReader::new(file))
                                .map_err(|e| e.to_string())?;
                        let (w, h) = decoder.dimensions();
                        if u64::from(w) * u64::from(h) * 8 > DECODE_BUDGET {
                            return Err("GIFの画像サイズがメモリ上限を超えています".into());
                        }
                        let mut limits = image::Limits::default();
                        limits.max_alloc = Some(DECODE_BUDGET);
                        decoder.set_limits(limits).map_err(|e| e.to_string())?;
                        let mut count = 0;
                        let mut frames = decoder.into_frames();
                        loop {
                            if stop.load(Ordering::Relaxed) {
                                return Ok(());
                            }
                            let Some(frame) = frames.next() else {
                                break;
                            };
                            let frame = frame.map_err(|e| e.to_string())?;
                            let (n, d) = frame.delay().numer_denom_ms();
                            let ms = if d == 0 {
                                100
                            } else {
                                (u64::from(n) / u64::from(d)).clamp(20, 60000)
                            };
                            let rgba = downscale_to_cap(frame.into_buffer(), cap.min(1024))?;
                            if !send(
                                &tx,
                                &stop,
                                Ok(Frame {
                                    rgba,
                                    delay: Duration::from_millis(ms),
                                    index: count,
                                    total,
                                }),
                            ) {
                                return Ok(());
                            }
                            count += 1;
                        }
                        if count == 0 {
                            return Err("GIFにフレームがありません".into());
                        }
                        total = Some(count);
                    }
                });
                if let Err(error) = result {
                    send(&tx, &stop, Err(error));
                }
            })
            .expect("gif stream");
        Self {
            rx: Some(rx),
            cancelled,
            failed: None,
        }
    }
    pub fn poll(&mut self) -> Option<Result<Frame, String>> {
        // A terminal error sticks: once failed, polls keep reporting the
        // error instead of reverting to "pending" (None).
        if let Some(error) = &self.failed {
            return Some(Err(error.clone()));
        }
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(Err(error)) => {
                self.failed = Some(error.clone());
                Some(Err(error))
            }
            Ok(frame) => Some(frame),
            Err(_) => None,
        }
    }
}
/// Blocking send with backpressure instead of a sleep-spin. Only the worker
/// thread ever blocks here (never the UI thread). A dropped player
/// disconnects the channel, which wakes a parked send immediately, so
/// cancellation is always noticed promptly.
fn send(
    tx: &mpsc::SyncSender<Result<Frame, String>>,
    stop: &AtomicBool,
    frame: Result<Frame, String>,
) -> bool {
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    tx.send(frame).is_ok()
}
impl Drop for Animation {
    fn drop(&mut self) {
        // Signal first so a worker between frames exits without touching the
        // channel; then disconnect so a worker parked in send wakes up. Never
        // joined: dropping must not block the UI thread.
        self.cancelled.store(true, Ordering::Relaxed);
        self.rx.take();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_reaches_frames_beyond_200_and_loops_without_collecting() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let path = std::env::temp_dir().join(format!("viewer-long-{}.gif", std::process::id()));
        {
            let mut encoder = GifEncoder::new(std::fs::File::create(&path).unwrap());
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for index in 0..241 {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(4, 3, image::Rgba([index as u8, 0, 0, 255])),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(20, 1),
                    ))
                    .unwrap();
            }
        }
        let mut stream = Animation::new(path.clone(), 2048);
        let start = std::time::Instant::now();
        let mut seen = 0;
        while seen < 243 {
            if let Some(frame) = stream.poll() {
                let frame = frame.unwrap();
                assert_eq!(frame.index, seen % 241);
                assert_eq!(frame.rgba.get_pixel(0, 0).0[0], (seen % 241) as u8);
                if seen >= 241 {
                    assert_eq!(frame.total, Some(241));
                } else {
                    // Unknown until the first loop completes.
                    assert_eq!(frame.total, None);
                }
                seen += 1;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        let cancelled = stream.cancelled.clone();
        drop(stream);
        assert!(cancelled.load(Ordering::Relaxed));
        // Decoder may be finishing a frame; wait for its file handle to be released on Windows.
        for _ in 0..100 {
            if std::fs::remove_file(&path).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("GIF worker did not release file after cancellation");
    }

    #[test]
    fn partial_frame_is_composited_onto_full_canvas() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let path = std::env::temp_dir().join(format!("viewer-partial-{}.gif", std::process::id()));
        {
            let mut encoder = GifEncoder::new(std::fs::File::create(&path).unwrap());
            encoder.set_repeat(Repeat::Infinite).unwrap();
            // Full-canvas red base frame.
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(20, 1),
                ))
                .unwrap();
            // Smaller blue patch: the decoder must composite it onto the
            // canvas, not hand through a 2x2 image. (The encoder drops the
            // offset, so the patch lands at the top-left; the point here is
            // the full-canvas composite our pipeline serves.)
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255])),
                    1,
                    0,
                    image::Delay::from_numer_denom_ms(20, 1),
                ))
                .unwrap();
        }
        let mut stream = Animation::new(path.clone(), 2048);
        let start = std::time::Instant::now();
        // Skip the base frame; the next frame is the composited patch.
        let mut seen = 0;
        loop {
            if let Some(frame) = stream.poll() {
                let frame = frame.unwrap();
                if seen == 1 {
                    assert_eq!(frame.rgba.dimensions(), (4, 3));
                    assert_eq!(frame.rgba.get_pixel(0, 0).0, [0, 0, 255, 255]);
                    assert_eq!(frame.rgba.get_pixel(1, 1).0, [0, 0, 255, 255]);
                    // Outside the patch: previous frame was disposed.
                    assert_eq!(frame.rgba.get_pixel(2, 0).0, [0, 0, 0, 0]);
                    assert_eq!(frame.rgba.get_pixel(0, 2).0, [0, 0, 0, 0]);
                    break;
                }
                seen += 1;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(stream);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mid_stream_failure_becomes_sticky_error() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let path = std::env::temp_dir().join(format!("viewer-swap-{}.gif", std::process::id()));
        {
            let mut encoder = GifEncoder::new(std::fs::File::create(&path).unwrap());
            encoder.set_repeat(Repeat::Infinite).unwrap();
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(4, 3, image::Rgba([1, 2, 3, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(20, 1),
                ))
                .unwrap();
        }
        let mut stream = Animation::new(path.clone(), 2048);
        let start = std::time::Instant::now();
        // Wait until the first loop completes (total known): the worker is
        // mid-playback, then the file turns to garbage under it.
        loop {
            if let Some(frame) = stream.poll() {
                if frame.unwrap().total.is_some() {
                    break;
                }
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        std::fs::write(&path, b"definitely not a gif").unwrap();
        // The next pass fails to open and the error sticks; playback never
        // busy-retries behind the UI's back.
        let mut polls = 0;
        loop {
            match stream.poll() {
                // Draining frames decoded before the swap is fine.
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
                None => {
                    assert!(start.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            polls += 1;
            assert!(polls < 100_000, "worker kept quiet after the file broke");
        }
        assert!(matches!(stream.poll(), Some(Err(_))));
        drop(stream);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn worker_panic_surfaces_as_sticky_error() {
        // Documents the wiring used by the stream worker: a codec panic
        // becomes the sticky Err the UI shows, instead of a silent hang on a
        // dead worker thread (same contract as the image loader).
        let result: Result<(), String> = catch_panic(|| panic!("codec bug"));
        assert_eq!(result, Err("decoder panicked".to_owned()));
    }

    #[test]
    fn failure_is_sticky_and_distinguishable_from_pending() {
        let missing =
            std::env::temp_dir().join(format!("viewer-no-such-{}.gif", std::process::id()));
        let mut animation = Animation::new(missing, 2048);
        let start = std::time::Instant::now();
        loop {
            match animation.poll() {
                Some(Err(_)) => break,
                Some(Ok(_)) => panic!("unexpected frame from a missing file"),
                None => {
                    assert!(start.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        // Stays failed instead of reverting to "pending".
        assert!(matches!(animation.poll(), Some(Err(_))));
        assert!(matches!(animation.poll(), Some(Err(_))));
    }
}
