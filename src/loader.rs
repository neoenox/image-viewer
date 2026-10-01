use crate::imaging::{decode_image, downscale_to_cap, rotate_rgba};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

pub(crate) const CACHE_BUDGET: usize = 64 * 1024 * 1024;
#[derive(Clone)]
pub struct CachedImage {
    pub rgba: Arc<image::RgbaImage>,
    pub orig: (u32, u32),
    pub(crate) gif: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetailKey {
    pub generation: u64,
    pub rotation: u8,
    pub region: [u32; 4],
}
pub(crate) struct DetailImage {
    pub key: DetailKey,
    pub rgba: image::RgbaImage,
}
type Key = (PathBuf, usize);
enum Job {
    Current(u64, Key),
    Detail(PathBuf, DetailKey),
    Preload(Key),
}
#[derive(Default)]
struct State {
    current: Option<(u64, Key)>,
    generation: u64,
    detail: Option<(PathBuf, DetailKey)>,
    preloads: VecDeque<Key>,
    active: Option<Key>,
    failed: HashSet<Key>,
    keep_cap: Option<usize>,
    cache: HashMap<Key, CachedImage>,
    order: VecDeque<Key>,
    bytes: usize,
    result: Option<(u64, Result<CachedImage, String>)>,
    detail_result: Option<Result<DetailImage, (DetailKey, String)>>,
    keep: Vec<PathBuf>,
    stopping: bool,
}
impl State {
    fn insert(&mut self, key: Key, value: CachedImage) {
        let bytes = value.rgba.as_raw().len();
        if bytes > CACHE_BUDGET {
            return;
        }
        if let Some(old) = self.cache.remove(&key) {
            self.bytes -= old.rgba.as_raw().len();
        }
        self.order.retain(|k| k != &key);
        while self.bytes + bytes > CACHE_BUDGET {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(value) = self.cache.remove(&old) {
                self.bytes -= value.rgba.as_raw().len();
            }
        }
        self.bytes += bytes;
        self.order.push_back(key.clone());
        self.cache.insert(key, value);
    }
}
pub struct ImageLoader {
    shared: Arc<(Mutex<State>, Condvar)>,
}
impl Default for ImageLoader {
    fn default() -> Self {
        Self::new()
    }
}
impl ImageLoader {
    pub fn new() -> Self {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("image-loader".into())
            .spawn(move || loop {
                let (lock, wake) = &*worker;
                let job = {
                    let mut state = lock.lock().unwrap();
                    loop {
                        if state.stopping {
                            return;
                        }
                        if let Some((gen, key)) = state.current.take() {
                            state.active = Some(key.clone());
                            break Job::Current(gen, key);
                        }
                        if let Some((path, key)) = state.detail.take() {
                            break Job::Detail(path, key);
                        }
                        if let Some(key) = state.preloads.pop_front() {
                            state.active = Some(key.clone());
                            break Job::Preload(key);
                        }
                        state = wake.wait(state).unwrap();
                    }
                };
                match job {
                    Job::Current(gen, key) => {
                        let cached = lock.lock().unwrap().cache.get(&key).cloned();
                        let result = cached.map(Ok).unwrap_or_else(|| load(&key));
                        let mut state = lock.lock().unwrap();
                        state.active = None;
                        if state.generation == gen && !state.stopping {
                            if let Ok(image) = &result {
                                state.insert(key, image.clone());
                            }
                            state.result = Some((gen, result));
                        }
                    }
                    Job::Preload(key) => {
                        let cached = lock.lock().unwrap().cache.contains_key(&key);
                        let result = if cached { None } else { Some(load(&key)) };
                        let mut state = lock.lock().unwrap();
                        state.active = None;
                        if state.keep.contains(&key.0)
                            && state.keep_cap.is_none_or(|cap| cap == key.1)
                            && !state.stopping
                        {
                            match result {
                                Some(Ok(image)) => state.insert(key, image),
                                Some(Err(_)) => {
                                    state.failed.insert(key);
                                }
                                None => {}
                            }
                        }
                    }
                    Job::Detail(path, key) => {
                        let result = load_detail(&path, &key);
                        let mut state = lock.lock().unwrap();
                        if state.generation == key.generation && !state.stopping {
                            state.detail_result = Some(
                                result
                                    .map(|rgba| DetailImage {
                                        key: key.clone(),
                                        rgba,
                                    })
                                    .map_err(|e| (key, e)),
                            );
                        }
                    }
                }
            })
            .expect("image loader");
        Self { shared }
    }
    pub(crate) fn select(&self, generation: u64, path: PathBuf, cap: usize) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap();
        state.generation = generation;
        state.current = Some((generation, (path, cap)));
        state.detail = None;
        state.result = None;
        state.detail_result = None;
        wake.notify_one();
    }
    pub(crate) fn poll(&self) -> Option<(u64, Result<CachedImage, String>)> {
        self.shared.0.lock().unwrap().result.take()
    }
    pub(crate) fn detail(&self, path: PathBuf, key: DetailKey) {
        let mut state = self.shared.0.lock().unwrap();
        state.detail = Some((path, key));
        self.shared.1.notify_one();
    }
    pub(crate) fn poll_detail(&self) -> Option<Result<DetailImage, (DetailKey, String)>> {
        self.shared.0.lock().unwrap().detail_result.take()
    }
    pub fn take(&self, path: &Path, cap: usize) -> Option<CachedImage> {
        self.shared
            .0
            .lock()
            .unwrap()
            .cache
            .get(&(path.to_owned(), cap))
            .cloned()
    }
    pub fn is_cached(&self, path: &Path, cap: usize) -> bool {
        self.shared
            .0
            .lock()
            .unwrap()
            .cache
            .contains_key(&(path.to_owned(), cap))
    }
    pub fn request(&self, path: PathBuf, cap: usize) {
        let key = (path, cap);
        let mut state = self.shared.0.lock().unwrap();
        if state.cache.contains_key(&key)
            || state.failed.contains(&key)
            || state.preloads.contains(&key)
            || state.active.as_ref() == Some(&key)
        {
            return;
        }
        if !state.keep.contains(&key.0) {
            state.keep.push(key.0.clone());
        }
        if state.preloads.len() < 16 {
            state.preloads.push_back(key);
        }
        self.shared.1.notify_one();
    }
    pub(crate) fn is_failed(&self, path: &Path, cap: usize) -> bool {
        self.shared
            .0
            .lock()
            .unwrap()
            .failed
            .contains(&(path.to_owned(), cap))
    }
    pub fn prune(&self, keep: &[PathBuf], cap: usize) {
        let mut state = self.shared.0.lock().unwrap();
        state.keep = keep.to_vec();
        state.keep_cap = Some(cap);
        state.failed.retain(|(p, c)| keep.contains(p) && *c == cap);
        state
            .preloads
            .retain(|(p, c)| keep.contains(p) && *c == cap);
        state
            .cache
            .retain(|(p, c), _| keep.contains(p) && *c == cap);
        let keys: Vec<_> = state.cache.keys().cloned().collect();
        state.order.retain(|k| keys.contains(k));
        state.bytes = state.cache.values().map(|v| v.rgba.as_raw().len()).sum();
    }
}
impl Drop for ImageLoader {
    fn drop(&mut self) {
        self.shared.0.lock().unwrap().stopping = true;
        self.shared.1.notify_one();
    }
}
fn load((path, cap): &Key) -> Result<CachedImage, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let gif = reader.format() == Some(image::ImageFormat::Gif);
    // Explicit format fallback follows ImageReader's extension handling.
    reader.limits(image::Limits::default());
    let image = decode_image(path).map_err(|e| e.to_string())?;
    let orig = (image.width(), image.height());
    let rgba = Arc::new(downscale_to_cap(image.into_rgba8(), (*cap).min(2048)));
    Ok(CachedImage { rgba, orig, gif })
}
fn load_detail(path: &Path, key: &DetailKey) -> Result<image::RgbaImage, String> {
    let image = decode_image(path).map_err(|e| e.to_string())?.into_rgba8();
    let image = if key.rotation == 0 {
        image
    } else {
        rotate_rgba(&image, key.rotation)
    };
    let [x, y, w, h] = key.region;
    if x >= image.width() || y >= image.height() || w == 0 || h == 0 {
        return Err("画像サイズが変わりました。画像を開き直してください".into());
    }
    let w = w.min(image.width() - x);
    let h = h.min(image.height() - y);
    if u64::from(w) * u64::from(h) * 4 > CACHE_BUDGET as u64 {
        return Err("原寸表示領域がメモリ上限を超えています".into());
    }
    Ok(image::imageops::crop_imm(&image, x, y, w, h).to_image())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_evicts_by_bytes_and_shared_reads_do_not_copy_pixels() {
        let mut state = State::default();
        let make = || CachedImage {
            rgba: Arc::new(image::RgbaImage::new(2048, 2048)),
            orig: (2048, 2048),
            gif: false,
        };
        for index in 0..6 {
            state.insert((PathBuf::from(format!("{index}.png")), 2048), make());
        }
        assert_eq!(state.bytes, CACHE_BUDGET);
        assert_eq!(state.cache.len(), 4);
        assert!(!state.cache.contains_key(&(PathBuf::from("0.png"), 2048)));
        let image = state.cache.get(&(PathBuf::from("5.png"), 2048)).unwrap();
        assert!(Arc::ptr_eq(&image.rgba, &image.clone().rgba));
    }
    #[test]
    fn current_selection_supersedes_pending_work_and_errors_recover() {
        let dir = std::env::temp_dir().join(format!("viewer-priority-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.png");
        image::RgbaImage::new(8, 6).save(&good).unwrap();
        let loader = ImageLoader::new();
        loader.select(1, dir.join("missing.png"), 2048);
        loader.select(2, good.clone(), 2048);
        let start = std::time::Instant::now();
        loop {
            if let Some((generation, result)) = loader.poll() {
                assert_eq!(generation, 2);
                assert_eq!(result.unwrap().orig, (8, 6));
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        loader.select(3, dir.join("missing.png"), 2048);
        loop {
            if let Some((generation, result)) = loader.poll() {
                assert_eq!(generation, 3);
                assert!(result.is_err());
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        loader.select(4, good, 2048);
        loop {
            if let Some((generation, result)) = loader.poll() {
                assert_eq!(generation, 4);
                assert!(result.is_ok());
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
#[cfg(test)]
mod detail_tests {
    use super::*;
    #[test]
    fn changed_dimensions_return_error_instead_of_panicking_worker() {
        let path =
            std::env::temp_dir().join(format!("viewer-detail-bounds-{}.png", std::process::id()));
        image::RgbaImage::new(8, 6).save(&path).unwrap();
        let key = DetailKey {
            generation: 1,
            rotation: 0,
            region: [1024, 1024, 128, 128],
        };
        assert!(load_detail(&path, &key).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
