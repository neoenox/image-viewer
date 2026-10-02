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
                    let mut na = String::new();
                    while let Some(&(_, d)) = ia.peek() {
                        if !d.is_ascii_digit() {
                            break;
                        }
                        na.push(d);
                        ia.next();
                    }
                    let mut nb = String::new();
                    while let Some(&(_, d)) = ib.peek() {
                        if !d.is_ascii_digit() {
                            break;
                        }
                        nb.push(d);
                        ib.next();
                    }
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

/// 画像ファイル一覧を自然順（連番対応）でソートする。
pub(crate) fn sort_image_files(files: &mut [PathBuf]) {
    files.sort_by(|a, b| {
        let na = a
            .file_name()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let nb = b
            .file_name()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        natural_cmp(&na, &nb).then_with(|| a.cmp(b))
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
