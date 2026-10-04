//! Window and taskbar icon taken from the exe's own icon resource (embedded by
//! build.rs). Giving eframe any icon (`ViewportBuilder::with_icon`) keeps its icon
//! rescaling and PNG encoding code in the binary, about 2.8 MB, so the icon is set
//! here with Win32 calls instead.

/// Call every frame: eframe applies its default icon after the window is created, so
/// this re-applies ours whenever the window's icon is not ours. Cheap when it is.
#[cfg(windows)]
pub(crate) fn apply(frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;
    use std::sync::OnceLock;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }
    #[link(name = "user32")]
    extern "system" {
        fn LoadImageW(
            instance: *mut c_void,
            name: *const u16,
            kind: u32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> *mut c_void;
        fn GetSystemMetrics(index: i32) -> i32;
        fn SendMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
    }
    const IMAGE_ICON: u32 = 1;
    const WM_GETICON: u32 = 0x007F;
    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;
    const SM_CXICON: i32 = 11;
    const SM_CYICON: i32 = 12;
    const SM_CXSMICON: i32 = 49;
    const SM_CYSMICON: i32 = 50;
    // winresource's `set_icon` stores the icon under resource id 1.
    const ICON_RESOURCE_ID: usize = 1;

    // (big, small) icon handles, loaded once and kept for the life of the process.
    static ICONS: OnceLock<(isize, isize)> = OnceLock::new();
    let (big, small) = *ICONS.get_or_init(|| {
        // SAFETY: loads this module's own icon resource (MAKEINTRESOURCE(1)).
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let load = |cx, cy| {
                LoadImageW(
                    module,
                    ICON_RESOURCE_ID as *const u16,
                    IMAGE_ICON,
                    GetSystemMetrics(cx),
                    GetSystemMetrics(cy),
                    0,
                ) as isize
            };
            (load(SM_CXICON, SM_CYICON), load(SM_CXSMICON, SM_CYSMICON))
        }
    });
    if big == 0 || small == 0 {
        return;
    }
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(window) = handle.as_raw() else {
        return;
    };
    let hwnd = window.hwnd.get() as *mut c_void;
    // SAFETY: plain Win32 messages to this process's own window.
    unsafe {
        if SendMessageW(hwnd, WM_GETICON, ICON_BIG, 0) != big {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG, big);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL, small);
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn apply(_frame: &eframe::Frame) {}
