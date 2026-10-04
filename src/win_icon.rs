//! Window and taskbar icon taken from the exe's own icon resource (embedded by
//! build.rs). Passing the icon to eframe instead would pull in its icon rescaling and
//! PNG encoding code, which added about 2.8 MB to the exe.

/// Sets the icon on the native window; returns `false` while the window is not ready.
#[cfg(windows)]
pub(crate) fn apply(frame: &eframe::Frame) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;

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
    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;
    const SM_CXICON: i32 = 11;
    const SM_CYICON: i32 = 12;
    const SM_CXSMICON: i32 = 49;
    const SM_CYSMICON: i32 = 50;
    // winresource's `set_icon` stores the icon under resource id 1.
    const ICON_RESOURCE_ID: usize = 1;

    let Ok(handle) = frame.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(window) = handle.as_raw() else {
        return true;
    };
    let hwnd = window.hwnd.get() as *mut c_void;
    // SAFETY: plain Win32 calls with a valid window handle owned by this process; the
    // resource id is passed as MAKEINTRESOURCE. The loaded icons live as long as the app.
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        for (which, cx, cy) in [
            (ICON_BIG, SM_CXICON, SM_CYICON),
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
        ] {
            let icon = LoadImageW(
                module,
                ICON_RESOURCE_ID as *const u16,
                IMAGE_ICON,
                GetSystemMetrics(cx),
                GetSystemMetrics(cy),
                0,
            );
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, which, icon as isize);
            }
        }
    }
    true
}

#[cfg(not(windows))]
pub(crate) fn apply(_frame: &eframe::Frame) -> bool {
    true
}
