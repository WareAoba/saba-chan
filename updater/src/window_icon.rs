//! Tauri 창(GUI·업데이터·인스톨러)의 Windows 아이콘을 exe 리소스에서 크기별로 건다.
//!
//! Tauri는 `icon.ico`의 첫 항목 하나를 작은 아이콘(제목 표시줄)으로만 걸고 큰 아이콘(작업 표시줄·Alt+Tab)은
//! 비워 둔다. 그러면 Windows가 작은 아이콘을 늘려 그려서 흐려진다. exe에는 tauri-build가 같은 .ico 전체를
//! 리소스 32512로 넣어 두므로, 창 DPI에 맞는 크기를 Windows가 그 안에서 고르게 한다.

/// 창 하나에 작은·큰 아이콘을 건다. 실패하면 Tauri가 건 아이콘이 그대로 남는다
#[cfg(windows)]
pub fn apply_exe_icon(hwnd: isize) {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, WM_SETICON,
    };

    // tauri-build가 exe에 넣는 아이콘 리소스 ID
    const APP_ICON_ID: usize = 32512;
    // DPI별로 한 번만 읽고 여러 창(콘솔 팝아웃 등)이 같이 쓴다. 프로세스가 끝날 때까지 둔다
    static CACHE: OnceLock<Mutex<HashMap<(u32, i32), isize>>> = OnceLock::new();

    let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi == 0 { 96 } else { dpi };
    let load = |metric| -> isize {
        let size = unsafe { GetSystemMetricsForDpi(metric, dpi) };
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *cache.entry((dpi, size)).or_insert_with(|| unsafe {
            LoadImageW(
                GetModuleHandleW(std::ptr::null()),
                APP_ICON_ID as *const u16,
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            ) as isize
        })
    };
    for (kind, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
        let icon = load(metric);
        if icon != 0 {
            unsafe { SendMessageW(hwnd, WM_SETICON, kind as usize, icon) };
        }
    }
}

#[cfg(not(windows))]
pub fn apply_exe_icon(_hwnd: isize) {}
