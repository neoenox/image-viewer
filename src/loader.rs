use crate::imaging::{decode_image, downscale_to_cap, rotate_rgba};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const THUMBNAIL_WORKERS: usize = 3;
/// Which queue a worker thread serves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Current,
    Preload,
    Detail,
    /// Current selection first, then preloads (thumbnail workers).
    Any,
}
/// Failed loads are retried after this long (e.g. a file still being written).
const FAILED_RETRY: Duration = Duration::from_secs(5);

// Holds the five-image keep window (current ±2) at the 2048px cap (16MB each).
pub(crate) const CACHE_BUDGET: usize = 80 * 1024 * 1024;
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
    active: Vec<Key>,
    failed: HashMap<Key, Instant>,
    keep_cap: Option<usize>,
    cache: HashMap<Key, CachedImage>,
    order: VecDeque<Key>,
    bytes: usize,
    result: Option<(u64, Result<CachedImage, String>)>,
    detail_result: Option<Result<DetailImage, (DetailKey, String)>>,
    keep: Vec<PathBuf>,
    detail_src: Option<DetailSource>,
    stopping: bool,
}
/// The decoded (already rotated) source of the last detail request, kept so panning
/// across one image does not decode the whole file again for every region.
struct DetailSource {
    path: PathBuf,
    rotation: u8,
    modified: Option<std::time::SystemTime>,
    image: Arc<image::RgbaImage>,
}
impl State {
    /// How many workers are currently decoding `key`.
    fn in_flight(&self, key: &Key) -> usize {
        self.active.iter().filter(|k| *k == key).count()
    }
    /// Drop one in-flight marker for `key` (another worker may hold the same key).
    fn finish(&mut self, key: &Key) {
        if let Some(pos) = self.active.iter().position(|k| k == key) {
            self.active.swap_remove(pos);
        }
    }
    /// Mark a cached key as most recently used.
    fn touch(&mut self, key: &Key) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            if let Some(k) = self.order.remove(pos) {
                self.order.push_back(k);
            }
        }
    }
    fn is_failed(&self, key: &Key) -> bool {
        self.failed
            .get(key)
            .is_some_and(|at| at.elapsed() < FAILED_RETRY)
    }
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
        // Each kind of work has its own thread, so the image being opened never
        // waits behind a neighbour preload or a slow original-size region decode.
        Self::with_workers(&[Role::Current, Role::Preload, Role::Detail])
    }
    /// Thumbnails are small but each needs a full decode, so a visible page is
    /// decoded in parallel. Peak decode memory grows with the worker count.
    pub fn for_thumbnails() -> Self {
        Self::with_workers(&[Role::Any; THUMBNAIL_WORKERS])
    }
    fn with_workers(roles: &[Role]) -> Self {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        for &role in roles {
            Self::spawn_worker(&shared, role);
        }
        Self { shared }
    }
    fn spawn_worker(shared: &Arc<(Mutex<State>, Condvar)>, role: Role) {
        let worker = shared.clone();
        std::thread::Builder::new()
            .name(
                match role {
                    Role::Detail => "image-detail",
                    Role::Preload => "image-preload",
                    Role::Current | Role::Any => "image-loader",
                }
                .into(),
            )
            .spawn(move || loop {
                let (lock, wake) = &*worker;
                let job = {
                    let mut state = lock.lock().unwrap();
                    loop {
                        if state.stopping {
                            return;
                        }
                        if role == Role::Detail {
                            if let Some((path, key)) = state.detail.take() {
                                break Job::Detail(path, key);
                            }
                        }
                        if matches!(role, Role::Current | Role::Any) {
                            if let Some((gen, key)) = state.current.take() {
                                state.active.push(key.clone());
                                break Job::Current(gen, key);
                            }
                        }
                        if matches!(role, Role::Preload | Role::Any) {
                            if let Some(key) = state.preloads.pop_front() {
                                state.active.push(key.clone());
                                break Job::Preload(key);
                            }
                        }
                        state = wake.wait(state).unwrap();
                    }
                };
                match job {
                    Job::Current(gen, key) => {
                        let cached = {
                            let mut state = lock.lock().unwrap();
                            // The same image may already be decoding as a preload:
                            // wait for that result instead of decoding it twice.
                            while state.in_flight(&key) > 1
                                && state.generation == gen
                                && !state.stopping
                            {
                                state = wake.wait(state).unwrap();
                            }
                            if state.generation != gen || state.stopping {
                                state.finish(&key);
                                continue;
                            }
                            state.touch(&key);
                            state.cache.get(&key).cloned()
                        };
                        let result = cached.map(Ok).unwrap_or_else(|| load(&key));
                        let mut state = lock.lock().unwrap();
                        state.finish(&key);
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
                        state.finish(&key);
                        // A current-image job may be waiting for this result.
                        wake.notify_all();
                        if state.keep.contains(&key.0)
                            && state.keep_cap.is_none_or(|cap| cap == key.1)
                            && !state.stopping
                        {
                            match result {
                                Some(Ok(image)) => state.insert(key, image),
                                Some(Err(_)) => {
                                    state.failed.insert(key, Instant::now());
                                }
                                None => {}
                            }
                        }
                    }
                    Job::Detail(path, key) => {
                        let result = detail_source(lock, &path, &key)
                            .and_then(|source| crop_detail(&source, &key));
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
    }
    pub(crate) fn select(&self, generation: u64, path: PathBuf, cap: usize) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap();
        state.generation = generation;
        if state.detail_src.as_ref().is_some_and(|s| s.path != path) {
            state.detail_src = None;
        }
        state.current = Some((generation, (path, cap)));
        state.detail = None;
        state.result = None;
        state.detail_result = None;
        wake.notify_all();
    }
    pub(crate) fn poll(&self) -> Option<(u64, Result<CachedImage, String>)> {
        self.shared.0.lock().unwrap().result.take()
    }
    pub(crate) fn detail(&self, path: PathBuf, key: DetailKey) {
        let mut state = self.shared.0.lock().unwrap();
        state.detail = Some((path, key));
        self.shared.1.notify_all();
    }
    pub(crate) fn poll_detail(&self) -> Option<Result<DetailImage, (DetailKey, String)>> {
        self.shared.0.lock().unwrap().detail_result.take()
    }
    pub fn take(&self, path: &Path, cap: usize) -> Option<CachedImage> {
        let key = (path.to_owned(), cap);
        let mut state = self.shared.0.lock().unwrap();
        state.touch(&key);
        state.cache.get(&key).cloned()
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
            || state.is_failed(&key)
            || state.preloads.contains(&key)
            || state.active.contains(&key)
        {
            return;
        }
        if !state.keep.contains(&key.0) {
            state.keep.push(key.0.clone());
        }
        if state.preloads.len() < 16 {
            state.preloads.push_back(key);
        }
        self.shared.1.notify_all();
    }
    pub(crate) fn is_failed(&self, path: &Path, cap: usize) -> bool {
        self.shared
            .0
            .lock()
            .unwrap()
            .is_failed(&(path.to_owned(), cap))
    }
    pub fn prune(&self, keep: &[PathBuf], cap: usize) {
        let mut state = self.shared.0.lock().unwrap();
        state.keep = keep.to_vec();
        state.keep_cap = Some(cap);
        state
            .failed
            .retain(|(p, c), _| keep.contains(p) && *c == cap);
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
        self.shared.1.notify_all();
    }
}
/// Per-path decode counts, so tests can assert that work was not repeated.
#[cfg(test)]
static LOAD_COUNTS: Mutex<Option<HashMap<PathBuf, usize>>> = Mutex::new(None);
#[cfg(test)]
fn load_count(path: &Path) -> usize {
    LOAD_COUNTS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(path).copied())
        .unwrap_or(0)
}
fn load((path, cap): &Key) -> Result<CachedImage, String> {
    #[cfg(test)]
    {
        *LOAD_COUNTS
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .entry(path.clone())
            .or_insert(0) += 1;
    }
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
fn modified_time(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
/// Decoded, rotated source for a detail request: reused while the file, its
/// modification time and the rotation are unchanged, otherwise decoded and stored.
fn detail_source(
    lock: &Mutex<State>,
    path: &Path,
    key: &DetailKey,
) -> Result<Arc<image::RgbaImage>, String> {
    let modified = modified_time(path);
    if let Some(source) = &lock.lock().unwrap().detail_src {
        if source.path == path && source.rotation == key.rotation && source.modified == modified {
            return Ok(source.image.clone());
        }
    }
    let image = Arc::new(decode_detail_source(path, key.rotation)?);
    let mut state = lock.lock().unwrap();
    if state.generation == key.generation && !state.stopping {
        state.detail_src = Some(DetailSource {
            path: path.to_owned(),
            rotation: key.rotation,
            modified,
            image: image.clone(),
        });
    }
    Ok(image)
}
fn decode_detail_source(path: &Path, rotation: u8) -> Result<image::RgbaImage, String> {
    let image = decode_image(path).map_err(|e| e.to_string())?.into_rgba8();
    Ok(if rotation == 0 {
        image
    } else {
        rotate_rgba(&image, rotation)
    })
}
#[cfg(test)]
fn load_detail(path: &Path, key: &DetailKey) -> Result<image::RgbaImage, String> {
    crop_detail(&decode_detail_source(path, key.rotation)?, key)
}
fn crop_detail(image: &image::RgbaImage, key: &DetailKey) -> Result<image::RgbaImage, String> {
    let [x, y, w, h] = key.region;
    if x >= image.width() || y >= image.height() || w == 0 || h == 0 {
        return Err("画像サイズが変わりました。画像を開き直してください".into());
    }
    let w = w.min(image.width() - x);
    let h = h.min(image.height() - y);
    if u64::from(w) * u64::from(h) * 4 > CACHE_BUDGET as u64 {
        return Err("原寸表示領域がメモリ上限を超えています".into());
    }
    Ok(image::imageops::crop_imm(image, x, y, w, h).to_image())
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
        assert_eq!(state.cache.len(), 5);
        assert!(!state.cache.contains_key(&(PathBuf::from("0.png"), 2048)));
        let image = state.cache.get(&(PathBuf::from("5.png"), 2048)).unwrap();
        assert!(Arc::ptr_eq(&image.rgba, &image.clone().rgba));
    }
    #[test]
    fn touched_entries_survive_eviction() {
        let mut state = State::default();
        let make = || CachedImage {
            rgba: Arc::new(image::RgbaImage::new(2048, 2048)),
            orig: (2048, 2048),
            gif: false,
        };
        let key = |i: usize| (PathBuf::from(format!("{i}.png")), 2048);
        for index in 0..5 {
            state.insert(key(index), make());
        }
        state.touch(&key(0));
        state.insert(key(5), make());
        assert!(state.cache.contains_key(&key(0)));
        assert!(!state.cache.contains_key(&key(1)));
    }
    #[test]
    fn failed_entries_expire() {
        let mut state = State::default();
        let key = (PathBuf::from("bad.png"), 2048);
        state.failed.insert(key.clone(), Instant::now());
        assert!(state.is_failed(&key));
        state.failed.insert(
            key.clone(),
            Instant::now() - FAILED_RETRY - Duration::from_secs(1),
        );
        assert!(!state.is_failed(&key));
    }
    #[test]
    fn in_flight_markers_are_counted_and_released_one_at_a_time() {
        let mut state = State::default();
        let key = (PathBuf::from("same.png"), 2048);
        state.active.push(key.clone());
        state.active.push(key.clone());
        assert_eq!(state.in_flight(&key), 2);
        state.finish(&key);
        assert_eq!(state.in_flight(&key), 1);
        state.finish(&key);
        assert_eq!(state.in_flight(&key), 0);
    }
    #[test]
    fn selecting_an_image_that_is_being_preloaded_reuses_the_preload() {
        let dir = std::env::temp_dir().join(format!("viewer-dedupe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.png");
        image::RgbaImage::from_fn(1500, 1000, |x, y| image::Rgba([x as u8, y as u8, 7, 255]))
            .save(&path)
            .unwrap();
        let loader = ImageLoader::new();
        loader.prune(std::slice::from_ref(&path), 2048);
        loader.request(path.clone(), 2048);
        // Wait until the preload worker has picked the job up.
        let start = std::time::Instant::now();
        while loader.shared.0.lock().unwrap().active.is_empty() {
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::yield_now();
        }
        loader.select(1, path.clone(), 2048);
        loop {
            if let Some((generation, result)) = loader.poll() {
                assert_eq!(generation, 1);
                assert_eq!(result.unwrap().orig, (1500, 1000));
                // The current job used the preload's result instead of decoding again.
                assert_eq!(load_count(&path), 1);
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(loader.shared.0.lock().unwrap().active.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
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

    fn key(generation: u64, rotation: u8, region: [u32; 4]) -> DetailKey {
        DetailKey {
            generation,
            rotation,
            region,
        }
    }
    fn detail_and_wait(loader: &ImageLoader, path: &Path, key: DetailKey) -> DetailImage {
        loader.detail(path.to_owned(), key);
        let start = std::time::Instant::now();
        loop {
            if let Some(result) = loader.poll_detail() {
                return result.map_err(|(_, e)| e).unwrap();
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn source(loader: &ImageLoader) -> Option<(Arc<image::RgbaImage>, u8)> {
        let state = loader.shared.0.lock().unwrap();
        state
            .detail_src
            .as_ref()
            .map(|s| (s.image.clone(), s.rotation))
    }

    #[test]
    fn detail_source_is_reused_until_rotation_file_or_selection_changes() {
        let dir = std::env::temp_dir().join(format!("viewer-detail-reuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        image::RgbaImage::from_pixel(300, 200, image::Rgba([1, 2, 3, 255]))
            .save(&a)
            .unwrap();
        image::RgbaImage::from_pixel(100, 100, image::Rgba([9, 9, 9, 255]))
            .save(&b)
            .unwrap();
        let loader = ImageLoader::new();
        loader.select(1, a.clone(), 2048);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = loader.poll();

        // Two different regions of the same image share one decoded source.
        let first = detail_and_wait(&loader, &a, key(1, 0, [0, 0, 64, 64]));
        let (src1, _) = source(&loader).unwrap();
        let second = detail_and_wait(&loader, &a, key(1, 0, [128, 64, 64, 64]));
        let (src2, _) = source(&loader).unwrap();
        assert!(Arc::ptr_eq(&src1, &src2), "source decoded twice");
        assert_eq!((first.rgba.width(), second.rgba.width()), (64, 64));

        // A rotation change needs a re-oriented source.
        let rotated = detail_and_wait(&loader, &a, key(1, 1, [0, 0, 64, 64]));
        let (src3, rotation) = source(&loader).unwrap();
        assert!(!Arc::ptr_eq(&src2, &src3));
        assert_eq!(rotation, 1);
        assert_eq!(src3.dimensions(), (200, 300));
        assert_eq!(rotated.rgba.dimensions(), (64, 64));

        // A rewritten file (new modification time) is decoded again.
        std::thread::sleep(std::time::Duration::from_millis(30));
        image::RgbaImage::from_pixel(320, 220, image::Rgba([4, 5, 6, 255]))
            .save(&a)
            .unwrap();
        detail_and_wait(&loader, &a, key(1, 1, [0, 0, 64, 64]));
        assert_eq!(source(&loader).unwrap().0.dimensions(), (220, 320));

        // Selecting another file drops the retained source.
        loader.select(2, b, 2048);
        assert!(source(&loader).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Latency benchmarks, run on demand:
/// `cargo test --release loader::bench -- --ignored --nocapture --test-threads=1`
#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    fn noise_png(path: &Path, w: u32, h: u32, seed: u64) {
        let mut s = seed | 1;
        let img = image::RgbaImage::from_fn(w, h, |_, _| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            image::Rgba([s as u8, (s >> 8) as u8, (s >> 16) as u8, 255])
        });
        img.save(path).unwrap();
    }
    fn wait_result(loader: &ImageLoader, generation: u64) -> f64 {
        let start = Instant::now();
        loop {
            if let Some((g, result)) = loader.poll() {
                if g == generation {
                    assert!(result.is_ok());
                    return start.elapsed().as_secs_f64() * 1000.0;
                }
            }
            assert!(start.elapsed() < Duration::from_secs(60), "timed out");
            std::thread::sleep(Duration::from_micros(200));
        }
    }
    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }

    #[test]
    #[ignore]
    fn navigation_latency() {
        let dir = std::env::temp_dir().join(format!("viewer-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let small: Vec<PathBuf> = (0..6)
            .map(|i| {
                let p = dir.join(format!("s{i}.png"));
                noise_png(&p, 3000, 2000, i + 1);
                p
            })
            .collect();
        let big = dir.join("big.png");
        noise_png(&big, 6000, 4000, 99);

        // 1. Cold: select an image that is not cached.
        let mut cold = Vec::new();
        for (i, p) in small.iter().enumerate() {
            let loader = ImageLoader::new();
            loader.select(1, p.clone(), 2048);
            cold.push(wait_result(&loader, 1));
            let _ = i;
        }

        // 2. Warm: neighbour was preloaded before the user moves to it.
        let mut warm = Vec::new();
        for i in 0..5 {
            let loader = ImageLoader::new();
            loader.prune(&small, 2048);
            loader.request(small[i + 1].clone(), 2048);
            while !loader.is_cached(&small[i + 1], 2048) {
                std::thread::sleep(Duration::from_millis(5));
            }
            loader.select(1, small[i + 1].clone(), 2048);
            warm.push(wait_result(&loader, 1));
        }

        // 3. Contention: a detail decode of a large image is running, then the user navigates.
        let mut busy = Vec::new();
        for _ in 0..5 {
            let loader = ImageLoader::new();
            loader.select(1, big.clone(), 2048);
            wait_result(&loader, 1);
            loader.detail(
                big.clone(),
                DetailKey {
                    generation: 1,
                    rotation: 0,
                    region: [0, 0, 512, 512],
                },
            );
            std::thread::sleep(Duration::from_millis(15));
            loader.select(2, small[0].clone(), 2048);
            busy.push(wait_result(&loader, 2));
        }

        // 7. A neighbour preload of a large image is decoding, then the user jumps
        //    to an image that is not cached.
        let mut preload_busy = Vec::new();
        for _ in 0..5 {
            let loader = ImageLoader::new();
            let mut keep = small.clone();
            keep.push(big.clone());
            loader.prune(&keep, 2048);
            loader.request(big.clone(), 2048);
            std::thread::sleep(Duration::from_millis(15));
            loader.select(1, small[3].clone(), 2048);
            preload_busy.push(wait_result(&loader, 1));
        }

        // 5. Thumbnails: one visible page (6 files) requested at once, 128px cap.
        let mut thumbs = Vec::new();
        for _ in 0..5 {
            let loader = ImageLoader::for_thumbnails();
            let start = Instant::now();
            for p in &small {
                loader.request(p.clone(), 128);
            }
            while !small.iter().all(|p| loader.is_cached(p, 128)) {
                assert!(start.elapsed() < Duration::from_secs(60), "timed out");
                std::thread::sleep(Duration::from_micros(200));
            }
            thumbs.push(start.elapsed().as_secs_f64() * 1000.0);
        }

        // 6. GIF: time until the first frame of an animation is ready.
        let gif = dir.join("anim.gif");
        {
            let file = std::fs::File::create(&gif).unwrap();
            let mut encoder = image::codecs::gif::GifEncoder::new_with_speed(file, 30);
            for i in 0..20u8 {
                let frame = image::RgbaImage::from_fn(800, 600, |x, y| {
                    image::Rgba([(x as u8).wrapping_add(i * 12), y as u8, i * 12, 255])
                });
                encoder
                    .encode_frame(image::Frame::from_parts(
                        frame,
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(100, 1),
                    ))
                    .unwrap();
            }
        }
        let mut gif_first = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let animation = crate::animation::Animation::new(gif.clone(), 2048);
            loop {
                if let Some(frame) = animation.poll() {
                    assert!(frame.is_ok());
                    break;
                }
                assert!(start.elapsed() < Duration::from_secs(60), "timed out");
                std::thread::sleep(Duration::from_micros(200));
            }
            gif_first.push(start.elapsed().as_secs_f64() * 1000.0);
        }

        // 4. Panning: first region decodes the file, later regions reuse the source.
        let (mut first, mut later) = (Vec::new(), Vec::new());
        for _ in 0..5 {
            let loader = ImageLoader::new();
            loader.select(1, big.clone(), 2048);
            wait_result(&loader, 1);
            for step in 0..4u32 {
                let start = Instant::now();
                loader.detail(
                    big.clone(),
                    DetailKey {
                        generation: 1,
                        rotation: 0,
                        region: [step * 512, 0, 512, 512],
                    },
                );
                while loader.poll_detail().is_none() {
                    std::thread::sleep(Duration::from_micros(200));
                }
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                if step == 0 {
                    first.push(ms);
                } else {
                    later.push(ms);
                }
            }
        }

        let fmt = |v: &[f64]| {
            v.iter()
                .map(|x| format!("{x:.0}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!(
            "BENCH cold   (3000x2000 noise) median {:.0} ms  [{}]",
            median(cold.clone()),
            fmt(&cold)
        );
        println!(
            "BENCH warm   (preloaded)       median {:.0} ms  [{}]",
            median(warm.clone()),
            fmt(&warm)
        );
        println!(
            "BENCH during detail decode     median {:.0} ms  [{}]",
            median(busy.clone()),
            fmt(&busy)
        );
        println!(
            "BENCH during preload decode    median {:.0} ms  [{}]",
            median(preload_busy.clone()),
            fmt(&preload_busy)
        );
        println!(
            "BENCH pan, first region        median {:.0} ms  [{}]",
            median(first.clone()),
            fmt(&first)
        );
        println!(
            "BENCH pan, later regions       median {:.0} ms  [{}]",
            median(later.clone()),
            fmt(&later)
        );
        println!(
            "BENCH thumbnails, 6 at 128px   median {:.0} ms  [{}]",
            median(thumbs.clone()),
            fmt(&thumbs)
        );
        println!(
            "BENCH GIF first frame (800x600) median {:.0} ms  [{}]",
            median(gif_first.clone()),
            fmt(&gif_first)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
