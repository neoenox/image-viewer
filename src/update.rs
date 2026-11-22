//! Silent, checksum-verified self-update.
//!
//! On startup a background worker reads the checksum file from the latest
//! GitHub release. If it names a newer Windows exe, the worker downloads it to
//! a temporary file, verifies its SHA-256 and stages it in the user's data
//! directory. Nothing is shown in the UI. The *next* startup swaps the staged
//! exe into place before the window is created, so a running viewer is never
//! interrupted. A crash between the two renames is recovered on the next start
//! by restoring the backup before retrying.
//!
//! `--no-update` skips the background check. Successful checks are throttled
//! to once per 24 hours. Failures are logged to `update/update.log` and
//! otherwise ignored.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const RELEASE_BASE: &str = "https://github.com/neoenox/image-viewer/releases/latest/download/";
const CHECKSUMS: &str = "SHA256SUMS.txt";
const ASSET_PREFIX: &str = "image-viewer-v";
const ASSET_SUFFIX: &str = "-windows-x86_64.exe";
const PENDING_EXE: &str = "pending.exe";
const PENDING_HASH: &str = "pending.sha256";
const PENDING_VERSION: &str = "pending.version";
const TEMP_EXE: &str = "pending.tmp";
const LAST_CHECK: &str = "last-check.txt";
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
const CURL_TIMEOUT_SECS: u8 = 20;

fn update_dir() -> Option<PathBuf> {
    crate::settings::ViewerSettings::default_path()
        .and_then(|path| path.parent().map(|parent| parent.join("update")))
}

fn log_error(dir: &Path, message: &str) {
    let _ = fs::write(dir.join("update.log"), message.as_bytes());
}

/// Prefer the OS copy so a user-writable PATH entry cannot hijack downloads.
fn curl_program() -> PathBuf {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system = root.join("System32").join("curl.exe");
        if system.is_file() {
            return system;
        }
    }
    PathBuf::from("curl.exe")
}

/// Called at the very start of `run()`. Applies a previously staged update.
pub fn apply_pending() {
    if let Some(dir) = update_dir() {
        let current = match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return,
        };
        if let Err(error) = apply_pending_in(&current, &dir, env!("CARGO_PKG_VERSION")) {
            log_error(&dir, &error);
        }
    }
}

/// Applies a staged update to `current`, returning true when a swap happened.
fn apply_pending_in(current: &Path, dir: &Path, version: &str) -> Result<bool, String> {
    let pending = dir.join(PENDING_EXE);
    let hash_file = dir.join(PENDING_HASH);
    let version_file = dir.join(PENDING_VERSION);
    let cleanup_metadata = || {
        let _ = fs::remove_file(&hash_file);
        let _ = fs::remove_file(&version_file);
    };
    let discard = || {
        let _ = fs::remove_file(&pending);
        cleanup_metadata();
    };

    let name = current
        .file_name()
        .ok_or_else(|| "current exe has no file name".to_owned())?
        .to_string_lossy()
        .into_owned();
    let backup = current.with_file_name(format!("{name}.old"));
    let staged_next = current.with_file_name(format!("{name}.new"));

    // A crash between `current -> backup` and `staged_next -> current` leaves
    // no exe at all. Restore the backup first; the staged update can still be
    // applied once the viewer is whole again.
    if !current.exists() && backup.exists() {
        fs::rename(&backup, current).map_err(|e| format!("exe recovery failed: {e}"))?;
    }
    let _ = fs::remove_file(&staged_next);

    if !pending.is_file() {
        discard();
        return Ok(false);
    }
    let staged_version =
        fs::read_to_string(&version_file).map_err(|e| format!("staged version unreadable: {e}"))?;
    if compare_versions(staged_version.trim(), version) != Ordering::Greater {
        discard();
        return Ok(false);
    }
    let expected =
        fs::read_to_string(&hash_file).map_err(|e| format!("staged hash unreadable: {e}"))?;
    let actual = sha256_file(&pending)?;
    if actual != expected.trim() {
        discard();
        return Err("staged exe failed its checksum".to_owned());
    }

    // The staging directory is usually on another volume, so copy next to the
    // exe first and only use same-directory renames for the actual swap.
    fs::copy(&pending, &staged_next).map_err(|e| format!("stage copy failed: {e}"))?;
    if fs::rename(current, &backup).is_err() {
        let _ = fs::remove_file(&staged_next);
        // Keep the verified staged file: a sharing violation is retryable.
        return Err("could not rename the running exe".to_owned());
    }
    if let Err(error) = fs::rename(&staged_next, current) {
        let rollback = fs::rename(&backup, current);
        let _ = fs::remove_file(&staged_next);
        return Err(format!(
            "could not install the new exe: {error}; rollback={rollback:?}"
        ));
    }
    let _ = fs::remove_file(&backup);
    cleanup_metadata();
    let _ = fs::remove_file(&pending);
    Ok(true)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// A future timestamp is treated as stale so a clock change cannot disable updates.
fn check_is_due(dir: &Path) -> bool {
    let Ok(text) = fs::read_to_string(dir.join(LAST_CHECK)) else {
        return true;
    };
    let Ok(last) = text.trim().parse::<u64>() else {
        return true;
    };
    let now = unix_seconds();
    now < last || now.saturating_sub(last) >= CHECK_INTERVAL_SECS
}

fn record_check(dir: &Path) {
    let _ = fs::write(dir.join(LAST_CHECK), unix_seconds().to_string());
}

/// Starts the release check off the UI thread. Failures are intentionally quiet.
pub fn start_background_check(version: &str) {
    #[cfg(windows)]
    {
        let version = version.to_owned();
        if std::thread::Builder::new()
            .name("image-updater".to_owned())
            .spawn(move || {
                if let Some(dir) = update_dir() {
                    match check_and_stage(&version, &dir) {
                        Ok(Some(new_version)) => {
                            log_error(&dir, &format!("staged update {new_version}"));
                        }
                        Ok(None) => {}
                        Err(error) => log_error(&dir, &error),
                    }
                }
            })
            .is_err()
        {
            // A viewer with no update check is still a working viewer.
        }
    }
    #[cfg(not(windows))]
    {
        let _ = version;
    }
}

/// One-shot check used by the installer: no window, no waiting on the UI.
pub fn check_updates_once() {
    let Some(dir) = update_dir() else {
        return;
    };
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    if let Err(error) = apply_pending_in(&current, &dir, env!("CARGO_PKG_VERSION")) {
        log_error(&dir, &error);
    }
    if let Err(error) = check_and_stage(env!("CARGO_PKG_VERSION"), &dir) {
        log_error(&dir, &error);
    }
}

fn check_and_stage(version: &str, dir: &Path) -> Result<Option<String>, String> {
    if !check_is_due(dir) {
        return Ok(None);
    }
    stage_from_sources(version, dir, http_get, http_download)
}

fn stage_from_sources<F, D>(
    version: &str,
    dir: &Path,
    fetch: F,
    download: D,
) -> Result<Option<String>, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
    D: Fn(&str, &Path) -> Result<(), String>,
{
    let checksums = fetch(&format!("{RELEASE_BASE}{CHECKSUMS}"))?;
    let _ = fs::create_dir_all(dir);
    record_check(dir);
    let text = String::from_utf8_lossy(&checksums);
    let Some((asset, new_version)) = find_newer_asset(&text, version) else {
        return Ok(None);
    };
    let temp = dir.join(TEMP_EXE);
    let _ = fs::remove_file(&temp);
    download(&format!("{RELEASE_BASE}{}", asset.name), &temp)?;
    let hash = sha256_file(&temp)?;
    if hash != asset.hash {
        let _ = fs::remove_file(&temp);
        return Err("downloaded exe failed its checksum".to_owned());
    }

    let pending = dir.join(PENDING_EXE);
    let _ = fs::remove_file(&pending);
    fs::rename(&temp, &pending).map_err(|e| format!("stage rename: {e}"))?;
    fs::write(dir.join(PENDING_HASH), hash.as_bytes()).map_err(|e| format!("stage hash: {e}"))?;
    fs::write(dir.join(PENDING_VERSION), new_version.as_bytes())
        .map_err(|e| format!("stage version: {e}"))?;
    Ok(Some(new_version))
}

fn find_newer_asset(checksums: &str, current: &str) -> Option<(Asset, String)> {
    checksums
        .lines()
        .filter_map(parse_checksum_line)
        .filter_map(|asset| version_from_asset(&asset.name).map(|version| (asset, version)))
        .filter(|(_, version)| compare_versions(version, current) == Ordering::Greater)
        .max_by(|a, b| compare_versions(&a.1, &b.1))
}

struct Asset {
    name: String,
    hash: String,
}

fn parse_checksum_line(line: &str) -> Option<Asset> {
    let mut fields = line.split_whitespace();
    let hash = fields.next()?;
    let name = fields.next()?;
    if fields.next().is_some()
        || hash.len() != 64
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    // Only a plain release asset name is accepted: no paths, traversal or URL/input tricks.
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
    {
        return None;
    }
    Some(Asset {
        name: name.to_owned(),
        hash: hash.to_ascii_lowercase(),
    })
}

fn version_from_asset(name: &str) -> Option<String> {
    let version = name
        .strip_prefix(ASSET_PREFIX)?
        .strip_suffix(ASSET_SUFFIX)?;
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return None;
    }
    Some(version.to_owned())
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left: Vec<u64> = left
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    let right: Vec<u64> = right
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    for index in 0..left.len().max(right.len()) {
        let ord = left
            .get(index)
            .unwrap_or(&0)
            .cmp(right.get(index).unwrap_or(&0));
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn http_get(url: &str) -> Result<Vec<u8>, String> {
    #[cfg(windows)]
    {
        let timeout = CURL_TIMEOUT_SECS.to_string();
        curl(&["-fsSL", "--max-time", timeout.as_str(), "--retry", "2", url])
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("updates require Windows".to_owned())
    }
}

fn http_download(url: &str, path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let timeout = CURL_TIMEOUT_SECS.to_string();
        let destination = path.to_string_lossy();
        curl(&[
            "-fsSL",
            "--max-time",
            timeout.as_str(),
            "--retry",
            "2",
            "-o",
            destination.as_ref(),
            url,
        ])?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (url, path);
        Err("updates require Windows".to_owned())
    }
}

#[cfg(windows)]
fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(curl_program())
        .args(args)
        .output()
        .map_err(|error| format!("curl could not start: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// Streaming SHA-256, kept local so the updater adds no dependency.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("hash open: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("hash read: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish_hex())
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Default)]
struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    bytes: u64,
}
impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            ..Default::default()
        }
    }
    fn update(&mut self, data: &[u8]) {
        self.bytes = self.bytes.wrapping_add(data.len() as u64);
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self
                .buffer
                .drain(..64)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            self.compress(&block);
        }
    }
    fn compress(&mut self, block: &[u8; 64]) {
        let mut words = [0u32; 64];
        for (index, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            words[index] = u32::from_be_bytes(*chunk);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for (index, word) in words.iter().enumerate() {
            let big_sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(big_sigma1)
                .wrapping_add(choose)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(*word);
            let big_sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = big_sigma0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    fn finish_hex(mut self) -> String {
        let bit_len = self.bytes.wrapping_mul(8);
        self.buffer.push(0x80);
        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0);
        }
        self.buffer.extend_from_slice(&bit_len.to_be_bytes());
        while !self.buffer.is_empty() {
            let block: [u8; 64] = self
                .buffer
                .drain(..64)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            self.compress(&block);
        }
        self.state
            .iter()
            .map(|word| format!("{word:08x}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    #[cfg(windows)]
    fn curl_program_prefers_the_system_copy() {
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let expected = root.join("System32").join("curl.exe");
        if expected.is_file() {
            assert_eq!(curl_program(), expected);
        }
    }

    fn write_staged(dir: &Path, payload: &[u8], version: &str) {
        fs::write(dir.join(PENDING_EXE), payload).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(payload);
        fs::write(dir.join(PENDING_HASH), hasher.finish_hex()).unwrap();
        fs::write(dir.join(PENDING_VERSION), version).unwrap();
    }
    use super::*;

    fn hash_text(text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        hasher.finish_hex()
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            hash_text(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_text("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hash_text("The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn checksum_lines_accept_only_safe_exe_entries() {
        let hash = "a".repeat(64);
        let line = format!("{hash}  image-viewer-v0.1.11-windows-x86_64.exe");
        let asset = parse_checksum_line(&line).unwrap();
        assert_eq!(asset.name, "image-viewer-v0.1.11-windows-x86_64.exe");
        assert_eq!(asset.hash, hash);
        assert!(parse_checksum_line(&format!("{hash}  ../evil.exe")).is_none());
        assert!(parse_checksum_line(&format!("{hash}  evil?.exe")).is_none());
        assert!(parse_checksum_line("abc  image-viewer-v0.1.11-windows-x86_64.exe").is_none());
        assert!(parse_checksum_line(&format!("{hash}  a b")).is_none());
    }

    #[test]
    fn newer_asset_is_selected_by_version() {
        let old = format!("{}  image-viewer-v0.1.9-windows-x86_64.exe", "1".repeat(64));
        let new = format!("{}  image-viewer-v0.2.0-windows-x86_64.exe", "2".repeat(64));
        let other = format!("{}  notes.txt", "3".repeat(64));
        let checksums = format!("{old}\n{new}\n{other}\n");
        let (asset, version) = find_newer_asset(&checksums, "0.1.10").unwrap();
        assert_eq!(version, "0.2.0");
        assert_eq!(asset.name, "image-viewer-v0.2.0-windows-x86_64.exe");
        assert!(find_newer_asset(&checksums, "0.2.0").is_none());
    }

    #[test]
    fn version_order_handles_missing_components() {
        assert_eq!(compare_versions("0.2", "0.1.10"), Ordering::Greater);
        assert_eq!(compare_versions("0.1.10", "0.1.10"), Ordering::Equal);
        assert_eq!(compare_versions("0.1.9", "0.1.10"), Ordering::Less);
    }

    #[test]
    fn staged_update_is_verified_and_applied() {
        let dir = std::env::temp_dir().join(format!("iv-update-apply-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let current = dir.join("app.exe");
        fs::write(&current, b"old").unwrap();
        let pending = dir.join(PENDING_EXE);
        fs::write(&pending, b"new").unwrap();
        let hash = sha256_file(&pending).unwrap();
        fs::write(dir.join(PENDING_HASH), hash.as_bytes()).unwrap();
        fs::write(dir.join(PENDING_VERSION), "9.9.9").unwrap();
        assert!(apply_pending_in(&current, &dir, "0.1.10").unwrap());
        assert_eq!(fs::read(&current).unwrap(), b"new");
        assert!(!pending.exists());
        assert!(!dir.join(PENDING_HASH).exists());
        assert!(!current.with_file_name("app.exe.new").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_staged_update_is_discarded_without_replacing_exe() {
        let dir = std::env::temp_dir().join(format!("iv-update-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let current = dir.join("app.exe");
        fs::write(&current, b"old").unwrap();
        fs::write(dir.join(PENDING_EXE), b"new").unwrap();
        fs::write(dir.join(PENDING_HASH), "0".repeat(64)).unwrap();
        fs::write(dir.join(PENDING_VERSION), "9.9.9").unwrap();
        assert!(apply_pending_in(&current, &dir, "0.1.10").is_err());
        assert_eq!(fs::read(&current).unwrap(), b"old");
        assert!(!dir.join(PENDING_EXE).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn staging_verifies_the_download_before_publishing_it() {
        let dir = std::env::temp_dir().join(format!("iv-update-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let payload = b"new exe bytes";
        let mut hasher = Sha256::new();
        hasher.update(payload);
        let hash = hasher.finish_hex();
        let checksums = format!(
            "{hash}  image-viewer-v9.9.9-windows-x86_64.exe\n{}\n",
            "0".repeat(64)
        );
        let result = stage_from_sources(
            "0.1.10",
            &dir,
            |_| Ok(checksums.as_bytes().to_vec()),
            |_, path| fs::write(path, payload).map_err(|e| e.to_string()),
        );
        assert_eq!(result.unwrap().as_deref(), Some("9.9.9"));
        assert_eq!(fs::read(dir.join(PENDING_EXE)).unwrap(), payload);
        assert_eq!(
            fs::read_to_string(dir.join(PENDING_VERSION)).unwrap(),
            "9.9.9"
        );
        assert!(fs::read_to_string(dir.join(LAST_CHECK))
            .unwrap()
            .parse::<u64>()
            .is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_checksum_mismatch_never_stages_an_executable() {
        let dir = std::env::temp_dir().join(format!("iv-update-mismatch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let checksums = format!(
            "{}  image-viewer-v9.9.9-windows-x86_64.exe\n",
            "1".repeat(64)
        );
        let result = stage_from_sources(
            "0.1.10",
            &dir,
            |_| Ok(checksums.as_bytes().to_vec()),
            |_, path| fs::write(path, b"tampered").map_err(|e| e.to_string()),
        );
        assert!(result.is_err());
        assert!(!dir.join(PENDING_EXE).exists());
        assert!(!dir.join(TEMP_EXE).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_crashed_swap_restores_the_backup_first() {
        let dir = std::env::temp_dir().join(format!("iv-update-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let current = dir.join("app.exe");
        fs::write(current.with_file_name("app.exe.old"), b"old").unwrap();
        assert!(!current.exists());
        assert!(!apply_pending_in(&current, &dir, "0.1.10").unwrap());
        assert_eq!(fs::read(&current).unwrap(), b"old");
        assert!(!current.with_file_name("app.exe.old").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recovery_still_applies_the_staged_update() {
        let dir = std::env::temp_dir().join(format!("iv-update-recover-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let current = dir.join("app.exe");
        fs::write(current.with_file_name("app.exe.old"), b"old").unwrap();
        write_staged(&dir, b"new", "9.9.9");
        assert!(apply_pending_in(&current, &dir, "0.1.10").unwrap());
        assert_eq!(fs::read(&current).unwrap(), b"new");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_retryable_failure_keeps_the_staged_file() {
        let dir = std::env::temp_dir().join(format!("iv-update-retry-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let current = dir.join("missing").join("app.exe");
        write_staged(&dir, b"new", "9.9.9");
        assert!(apply_pending_in(&current, &dir, "0.1.10").is_err());
        assert!(dir.join(PENDING_EXE).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn checks_are_throttled_for_a_day_but_not_by_future_clocks() {
        let dir = std::env::temp_dir().join(format!("iv-update-throttle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let check_file = dir.join(LAST_CHECK);
        assert!(check_is_due(&dir));
        let now = unix_seconds();
        fs::write(&check_file, now.to_string()).unwrap();
        assert!(!check_is_due(&dir));
        fs::write(&check_file, (now - CHECK_INTERVAL_SECS - 1).to_string()).unwrap();
        assert!(check_is_due(&dir));
        fs::write(&check_file, (now + 3600).to_string()).unwrap();
        assert!(check_is_due(&dir));
        fs::write(&check_file, "nonsense").unwrap();
        assert!(check_is_due(&dir));
        fs::remove_dir_all(dir).unwrap();
    }
}
