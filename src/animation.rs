use crate::imaging::{downscale_to_cap, DECODE_BUDGET};
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
    pub total: usize,
}
pub(crate) struct Animation {
    rx: mpsc::Receiver<Result<Frame, String>>,
    cancelled: Arc<AtomicBool>,
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
                let mut total = 0;
                let result = (|| -> Result<(), String> {
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
                            let rgba = downscale_to_cap(frame.into_buffer(), cap.min(1024));
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
                        total = count;
                    }
                })();
                if let Err(error) = result {
                    send(&tx, &stop, Err(error));
                }
            })
            .expect("gif stream");
        Self { rx, cancelled }
    }
    pub fn poll(&self) -> Option<Result<Frame, String>> {
        self.rx.try_recv().ok()
    }
}
fn send(
    tx: &mpsc::SyncSender<Result<Frame, String>>,
    stop: &AtomicBool,
    mut frame: Result<Frame, String>,
) -> bool {
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        match tx.try_send(frame) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            Err(mpsc::TrySendError::Full(value)) => {
                frame = value;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
impl Drop for Animation {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
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
        let stream = Animation::new(path.clone(), 2048);
        let start = std::time::Instant::now();
        let mut seen = 0;
        while seen < 243 {
            if let Some(frame) = stream.poll() {
                let frame = frame.unwrap();
                assert_eq!(frame.index, seen % 241);
                assert_eq!(frame.rgba.get_pixel(0, 0).0[0], (seen % 241) as u8);
                if seen >= 241 {
                    assert_eq!(frame.total, 241);
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
}
