use std::cmp::Ordering;
use std::path::{Path, PathBuf};
pub const SUPPORTED_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "ico", "dds", "hdr", "exr", "qoi",
    "pbm", "pgm", "ppm", "pam",
];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// ファイル名の自然順ソート用に、末尾の連番を数値として比較する。
/// "1.png" < "2.png" < "10.png" の順になる（辞書順だと "10" < "2" になる問題の対策）。
/// 数字の先頭ゼロは少ない方を先にする（"001" < "01" < "1"）。
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut ia, mut ib) = (a.char_indices().peekable(), b.char_indices().peekable());
    loop {
        let (ca, cb) = (ia.peek().map(|&(_, c)| c), ib.peek().map(|&(_, c)| c));
        match (ca, cb) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => {
                if x.is_ascii_digit() && y.is_ascii_digit() {
                    // 数字ブロックを切り出して数値比較
                    // char_indices gives UTF-8 boundaries. Compare borrowed
                    // digit slices instead of allocating Strings per comparison.
                    let start_a = ia.peek().map(|&(idx, _)| idx).unwrap();
                    while ia.peek().is_some_and(|&(_, d)| d.is_ascii_digit()) {
                        ia.next();
                    }
                    let end_a = ia.peek().map(|&(idx, _)| idx).unwrap_or(a.len());
                    let na = &a[start_a..end_a];
                    let start_b = ib.peek().map(|&(idx, _)| idx).unwrap();
                    while ib.peek().is_some_and(|&(_, d)| d.is_ascii_digit()) {
                        ib.next();
                    }
                    let end_b = ib.peek().map(|&(idx, _)| idx).unwrap_or(b.len());
                    let nb = &b[start_b..end_b];
                    let ta = na.trim_start_matches('0');
                    let tb = nb.trim_start_matches('0');
                    // まず有効桁数（=数値の大きさ）で比較
                    let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                    // 数値が等しい場合は元の文字列順（"001" < "01" < "1"）
                    let ord = na.cmp(nb);
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                } else {
                    let ord = x.cmp(&y);
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                    ia.next();
                    ib.next();
                }
            }
        }
    }
}

/// Each filename is converted to UTF-8 once per sort, not on every compare.
#[derive(Eq, PartialEq)]
struct NaturalSortKey {
    filename: String,
    path: PathBuf,
}

impl Ord for NaturalSortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        natural_cmp(&self.filename, &other.filename).then_with(|| self.path.cmp(&other.path))
    }
}

impl PartialOrd for NaturalSortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 画像ファイル一覧を自然順（連番対応）でソートする。
pub(crate) fn sort_image_files(files: &mut [PathBuf]) {
    files.sort_by_cached_key(|path| NaturalSortKey {
        filename: path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.clone(),
    });
}

pub fn collect_siblings(path: &Path) -> Vec<PathBuf> {
    let parent = match path.parent() {
        Some(p) => p,
        None => return vec![path.to_path_buf()],
    };
    let mut files: Vec<PathBuf> = match std::fs::read_dir(parent) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && is_supported(p))
            .collect(),
        Err(_) => return vec![path.to_path_buf()],
    };
    sort_image_files(&mut files);
    if !files.iter().any(|p| p == path) {
        files.push(path.to_path_buf());
        sort_image_files(&mut files);
    }
    if files.is_empty() {
        vec![path.to_path_buf()]
    } else {
        files
    }
}

pub fn first_image_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && is_supported(p))
        .collect();
    sort_image_files(&mut files);
    files.into_iter().next()
}

#[cfg(test)]
mod performance_regression_tests {
    use super::*;

    #[test]
    fn natural_sort_keeps_number_semantics_and_unicode_paths() {
        let mut files = vec![
            PathBuf::from("画像/10.png"),
            PathBuf::from("画像/1.png"),
            PathBuf::from("画像/01.png"),
            PathBuf::from("画像/2.png"),
            PathBuf::from("画像/001.png"),
        ];
        sort_image_files(&mut files);
        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["001.png", "01.png", "1.png", "2.png", "10.png"]);
    }

    #[test]
    fn sort_large_numbered_directory_without_changing_order() {
        let mut files: Vec<_> = (0..2000)
            .rev()
            .map(|n| PathBuf::from(format!("images/photo{n}.jpg")))
            .collect();
        sort_image_files(&mut files);
        assert_eq!(files.first().unwrap(), &PathBuf::from("images/photo0.jpg"));
        assert_eq!(
            files.last().unwrap(),
            &PathBuf::from("images/photo1999.jpg")
        );
    }

    /// Deterministic pseudo-shuffle (coprime stride): no RNG dependency.
    /// 7919 is prime; `n` must not be a multiple of it or the output duplicates.
    fn shuffled_numbered(n: usize) -> Vec<PathBuf> {
        debug_assert!(!n.is_multiple_of(7919), "stride shares a factor with n");
        // Non-padded names so the numeric digit-block path in natural_cmp runs
        // (zero-padded names would reduce to plain lexicographic order).
        (0..n)
            .map(|i| PathBuf::from(format!("img-{}.png", (i * 7919) % n)))
            .collect()
    }

    #[test]
    fn sort_tens_of_thousands_with_numeric_names() {
        let mut files = shuffled_numbered(20_000);
        let start = std::time::Instant::now();
        sort_image_files(&mut files);
        let elapsed = start.elapsed();
        // Exact expected order: proves ordering and permutation completeness
        // (no loss/duplicates) in one assertion.
        let expected: Vec<PathBuf> = (0..20_000)
            .map(|i| PathBuf::from(format!("img-{i}.png")))
            .collect();
        assert_eq!(files, expected);
        // Generous bound (debug measured ~0.3s): trips on algorithmic
        // regressions, not on normal CI jitter.
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "sort 20000 took {elapsed:?}"
        );
        println!(
            "BENCH sort 20000 files: {:.1} ms",
            elapsed.as_secs_f64() * 1000.0
        );
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// Benchmarks target release (the `perf` CI job). Debug builds are roughly
    /// 20x slower, so the same gate would otherwise false-fail an accidental
    /// `cargo test` in debug; scaling the ceiling keeps both modes honest.
    fn budget(release_ms: f64) -> f64 {
        if cfg!(debug_assertions) {
            release_ms * 20.0
        } else {
            release_ms
        }
    }

    fn median(mut times: Vec<f64>) -> f64 {
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times[times.len() / 2]
    }

    #[test]
    #[ignore = "perf gate: run in the release `perf` CI job (cargo test --release -- --ignored bench::)"]
    fn sort_hundreds_of_thousands_reports_median() {
        // Ceilings are ~20x the measured release median (50k ~110 ms,
        // 200k ~520 ms), so they trip on algorithmic/allocator regressions,
        // never on runner jitter. Pure CPU, deterministic input, no I/O.
        let mut medians = Vec::new();
        for (n, ceiling_ms) in [(50_000usize, 2_500.0), (200_000, 10_000.0)] {
            debug_assert!(!n.is_multiple_of(7919), "stride shares a factor with n");
            let mut times = Vec::new();
            for _ in 0..3 {
                // Rebuild the input each round so every round sorts afresh.
                // Non-padded names so the numeric path runs, as in the fast test.
                let mut files: Vec<PathBuf> = (0..n)
                    .map(|i| PathBuf::from(format!("img-{}.png", (i * 7919) % n)))
                    .collect();
                let start = std::time::Instant::now();
                sort_image_files(&mut files);
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let m = median(times);
            println!("BENCH sort {n} files: median {m:.1} ms");
            assert!(
                m < budget(ceiling_ms),
                "sort {n} median {m:.1} ms exceeded budget {:.0} ms",
                budget(ceiling_ms)
            );
            medians.push(m);
        }
        // Scaling guard: O(n log n) means 4x input is ~4.7x time. A ratio near
        // 16x would indicate an accidental O(n^2); cap well below that.
        let ratio = medians[1] / medians[0].max(1.0);
        println!("BENCH sort scaling 200k/50k: {ratio:.2}x");
        assert!(
            ratio < 12.0,
            "sort scaling regressed to {ratio:.2}x (O(n^2)?)"
        );
    }
}
