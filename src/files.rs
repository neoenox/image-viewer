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
                    let ord = na.cmp(&nb);
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
}
