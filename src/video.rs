//! Windows Media Foundation video frame source.
//!
//! A worker decodes RGB32 frames into a bounded queue, mirroring `animation.rs`.
//! Audio is deliberately not handled; the video stream plays once and stops.

use crate::imaging::downscale_to_cap;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::Once;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

pub(crate) struct VideoFrame {
    pub rgba: image::RgbaImage,
    pub delay: Duration,
}

pub(crate) struct Video {
    rx: Option<mpsc::Receiver<Result<VideoFrame, String>>>,
    cancelled: Arc<AtomicBool>,
    failed: Option<String>,
}

impl Video {
    pub fn new(path: PathBuf, cap: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel(2);
        let error_tx = tx.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let spawned = std::thread::Builder::new()
            .name("video-stream".to_owned())
            .spawn(move || {
                #[cfg(windows)]
                {
                    if let Err(error) = decode(&path, cap, &tx, &stop) {
                        send(&tx, &stop, Err(error));
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = (path, cap);
                    send(
                        &tx,
                        &stop,
                        Err("動画再生はWindowsでのみ対応しています".to_owned()),
                    );
                }
            });
        if spawned.is_err() {
            let _ = error_tx.send(Err("video worker could not start".to_owned()));
        }
        Self {
            rx: Some(rx),
            cancelled,
            failed: None,
        }
    }

    pub fn poll(&mut self) -> Option<Result<VideoFrame, String>> {
        if let Some(error) = &self.failed {
            return Some(Err(error.clone()));
        }
        match self.rx.as_ref()?.try_recv() {
            Ok(Err(error)) => {
                self.failed = Some(error.clone());
                Some(Err(error))
            }
            Ok(frame) => Some(frame),
            Err(_) => None,
        }
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.rx.take();
    }
}

fn send(
    tx: &mpsc::SyncSender<Result<VideoFrame, String>>,
    stop: &AtomicBool,
    frame: Result<VideoFrame, String>,
) -> bool {
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    match frame {
        Ok(frame) => match tx.try_send(Ok(frame)) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        },
        Err(error) => tx.send(Err(error)).is_ok(),
    }
}

fn video_log(message: &str) {
    if std::env::var_os("IV_VIDEO_LOG").is_none() {
        return;
    }
    let Some(path) = crate::settings::ViewerSettings::default_path()
        .and_then(|p| p.parent().map(|dir| dir.join("video.log")))
    else {
        return;
    };
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{message}");
    }
}

#[cfg(windows)]
fn ensure_media_foundation() -> bool {
    use windows::Win32::Media::MediaFoundation::{MFStartup, MFSTARTUP_NOSOCKET, MF_VERSION};
    static START: Once = Once::new();
    static OK: AtomicBool = AtomicBool::new(false);
    START.call_once(|| {
        OK.store(
            unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_ok() },
            Ordering::Release,
        );
    });
    OK.load(Ordering::Acquire)
}

#[cfg(windows)]
fn decode(
    path: &Path,
    cap: usize,
    tx: &mpsc::SyncSender<Result<VideoFrame, String>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("COM init: {e}"))?;
        // MFStartup is process-global; per-worker startup/shutdown races when
        // videos are switched quickly, so initialize once and keep it alive.
        let result = if ensure_media_foundation() {
            decode_inner(path, cap, tx, stop)
        } else {
            Err("Media Foundationを初期化できませんでした".to_owned())
        };
        CoUninitialize();
        result
    }
}

#[cfg(windows)]
unsafe fn decode_inner(
    path: &Path,
    cap: usize,
    tx: &mpsc::SyncSender<Result<VideoFrame, String>>,
    stop: &AtomicBool,
) -> Result<(), String> {
    use std::slice;
    use windows::core::PCWSTR;
    use windows::Win32::Media::MediaFoundation::*;
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut attrs = None;
    MFCreateAttributes(&mut attrs, 1).map_err(|e| format!("attributes: {e}"))?;
    if let Some(attrs) = &attrs {
        attrs
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .map_err(|e| format!("video processing: {e}"))?;
    }
    let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), attrs.as_ref())
        .map_err(|e| format!("source reader: {e}"))?;
    reader
        .SetStreamSelection(0xFFFF_FFFE, false)
        .map_err(|e| format!("stream selection: {e}"))?;
    reader
        .SetStreamSelection(0xFFFF_FFFC, true)
        .map_err(|e| format!("video selection: {e}"))?;

    let output = MFCreateMediaType().map_err(|e| format!("media type: {e}"))?;
    output
        .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
        .map_err(|e| format!("major type: {e}"))?;
    output
        .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
        .map_err(|e| format!("subtype: {e}"))?;
    reader
        .SetCurrentMediaType(0xFFFF_FFFC, None, &output)
        .map_err(|e| format!("RGB32 conversion unavailable: {e}"))?;

    let mut width = 0u32;
    let mut height = 0u32;
    let mut stride = 0isize;
    let mut delay = Duration::from_millis(33);

    let started = std::time::Instant::now();
    let mut index = 0usize;
    while !stop.load(Ordering::Relaxed) {
        let mut flags = 0u32;
        let mut sample = None;
        reader
            .ReadSample(
                0xFFFF_FFFC,
                0,
                None,
                Some(&mut flags),
                None,
                Some(&mut sample),
            )
            .map_err(|e| format!("read sample: {e}"))?;
        if flags & 2 != 0 {
            break;
        }
        if flags & 32 != 0 {
            let current = reader
                .GetCurrentMediaType(0xFFFF_FFFC)
                .map_err(|e| format!("current media type: {e}"))?;
            let size = current.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
            let w = (size >> 32) as u32;
            let h = (size & 0xFFFF_FFFF) as u32;
            if w == 0 || h == 0 || w > 4096 || h > 4096 {
                return Err("invalid video frame size".to_owned());
            }
            width = w;
            height = h;
            stride = current.GetUINT64(&MF_MT_DEFAULT_STRIDE).unwrap_or(0) as isize;
            if stride == 0 || stride.unsigned_abs() < width as usize * 4 {
                stride = width as isize * 4;
            }
            let rate = current.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
            let fps_num = (rate >> 32) as u32;
            let fps_den = (rate & 0xFFFF_FFFF) as u32;
            delay = if fps_num > 0 && fps_den > 0 {
                Duration::from_secs_f64(fps_den as f64 / fps_num as f64)
            } else {
                Duration::from_millis(33)
            };
        }
        let Some(sample) = sample else {
            continue;
        };
        if width == 0 || height == 0 {
            return Err("video format was not reported".to_owned());
        }
        let sample_delay = match sample.GetSampleDuration() {
            Ok(ticks) if ticks > 0 => Duration::from_nanos((ticks as u64) / 100),
            _ => delay,
        };
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| format!("sample buffer: {e}"))?;
        let len = buffer
            .GetCurrentLength()
            .map_err(|e| format!("buffer length: {e}"))? as usize;
        let row_bytes = width as usize * 4;
        let needed = row_bytes * height as usize;
        let step = stride.unsigned_abs();
        if step.checked_mul(height as usize).is_none_or(|n| n > len) || len < needed {
            return Err("short video frame".to_owned());
        }
        let mut ptr = std::ptr::null_mut();
        buffer
            .Lock(&mut ptr, None, None)
            .map_err(|e| format!("buffer lock: {e}"))?;
        if ptr.is_null() {
            buffer.Unlock().ok();
            return Err("empty video buffer".to_owned());
        }
        let mut rgba = vec![0u8; needed];
        {
            // Convert directly from the locked buffer; no full-frame copy.
            let bytes = slice::from_raw_parts(ptr, len);
            let top_down = stride >= 0;
            for y in 0..height as usize {
                let source_row = if top_down {
                    y * step
                } else {
                    (height as usize - 1 - y) * step
                };
                let source = &bytes[source_row..source_row + row_bytes];
                let dest = &mut rgba[y * row_bytes..(y + 1) * row_bytes];
                for (src, dst) in source.chunks_exact(4).zip(dest.chunks_exact_mut(4)) {
                    // MF RGB32 is BGRA in memory.
                    dst[0] = src[2];
                    dst[1] = src[1];
                    dst[2] = src[0];
                    dst[3] = 255;
                }
            }
        }
        buffer.Unlock().ok();
        let image = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| "frame buffer mismatch".to_owned())?;
        let image = downscale_to_cap(image, cap.min(2048))?;
        let frame = VideoFrame {
            rgba: image,
            delay: sample_delay.clamp(Duration::from_millis(5), Duration::from_secs(1)),
        };
        if index % 30 == 0 {
            video_log(&format!(
                "frame {index}: {} ms",
                started.elapsed().as_millis()
            ));
        }
        if !send(tx, stop, Ok(frame)) {
            break;
        }
        index += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_video_reports_a_sticky_error() {
        let path = std::env::temp_dir().join(format!("iv-missing-{}.mp4", std::process::id()));
        let mut video = Video::new(path, 128);
        let start = std::time::Instant::now();
        let error = loop {
            match video.poll() {
                Some(Err(error)) => break error,
                Some(Ok(_)) => panic!("missing file produced a frame"),
                None => {
                    assert!(start.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        };
        assert!(!error.is_empty());
        assert!(matches!(video.poll(), Some(Err(_))));
    }
}
