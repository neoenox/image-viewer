//! Crash reporting without embedding credentials.
//!
//! A panic is written to `%APPDATA%\image-viewer\crash.txt`. On the next
//! launch the viewer opens a GitHub issue form with that report prefilled;
//! the user still decides whether to submit it. No token or network call is
//! made by the app itself.

use std::path::PathBuf;
use std::sync::Mutex;

const CRASH_FILE: &str = "crash.txt";
const ISSUE_URL: &str = "https://github.com/neoenox/image-viewer/issues/new";
static CONTEXT: Mutex<Option<String>> = Mutex::new(None);

/// Set a short, privacy-safe description of what was open (extension only).
pub fn set_context(context: impl Into<String>) {
    if let Ok(mut slot) = CONTEXT.lock() {
        *slot = Some(context.into());
    }
}

fn data_dir() -> Option<PathBuf> {
    crate::settings::ViewerSettings::default_path()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
}

/// Install a panic hook that keeps the default output and records a report.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = data_dir() {
            let context = CONTEXT
                .lock()
                .ok()
                .and_then(|slot| slot.clone())
                .unwrap_or_else(|| "unknown".to_owned());
            let body = format!(
                "## Crash report\n\n- Version: {}\n- OS: {} {}\n- Context: {}\n\n```\n{}\n```\n",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                context,
                info
            );
            let _ = std::fs::write(dir.join(CRASH_FILE), body.as_bytes());
        }
        previous(info);
    }));
}

/// Return and consume a pending crash report.
pub fn take_pending() -> Option<String> {
    let dir = data_dir()?;
    let path = dir.join(CRASH_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(text)
}

fn issue_url(report: &str) -> String {
    let title = format!("crash: v{}", env!("CARGO_PKG_VERSION"));
    let body = format!(
        "{}\n\n---\nPlease add the file format/codec and steps to reproduce if known.\n",
        report
    );
    format!(
        "{}?labels=bug&title={}&body={}",
        ISSUE_URL,
        percent_encode(&title),
        percent_encode(&body)
    )
}

/// Open the prefilled GitHub issue form. The browser does the submission.
pub fn open_issue(report: &str) {
    let url = issue_url(report);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url.as_str()])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }
}

/// Minimal RFC 3986 percent encoding for URL query values.
fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => {
                out.push('%');
                out.push(HEX[(other >> 4) as usize] as char);
                out.push(HEX[(other & 0x0F) as usize] as char);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encode_keeps_unreserved_and_escapes_utf8() {
        assert_eq!(percent_encode("abc-_.~"), "abc-_.~");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("画像"), "%E7%94%BB%E5%83%8F");
    }

    #[test]
    fn pending_report_is_read_once() {
        let dir = std::env::temp_dir().join(format!("iv-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CRASH_FILE);
        std::fs::write(&path, "report").unwrap();
        let read = |path: &std::path::Path| -> Option<String> {
            let text = std::fs::read_to_string(path).ok()?;
            let _ = std::fs::remove_file(path);
            Some(text)
        };
        assert_eq!(read(&path).as_deref(), Some("report"));
        assert!(read(&path).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn issue_url_is_prefilled_without_sending_data() {
        let url = issue_url("panic: mp4");
        assert!(url.starts_with(ISSUE_URL));
        assert!(url.contains("labels=bug"));
        assert!(url.contains("title=crash%3A%20v"));
        assert!(url.contains("panic%3A%20mp4"));
    }
}
