//! The Windows side of the start's watch: finding Massif's window while it
//! is still hidden, and showing it.

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, SW_SHOW, ShowWindow};
use windows_sys::core::BOOL;

/// What [`show_hidden_windows`] looks for, and how many it showed.
struct Search {
    title: Vec<u16>,
    process: u32,
    shown: usize,
}

/// Show this process's top-level windows titled `title` that are hidden;
/// returns how many.
pub fn show_hidden_windows(title: &str) -> usize {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // Safety: `lparam` is the `Search` below, alive for the whole of
        // `EnumWindows`, which calls this on the same thread.
        let search = unsafe { &mut *(lparam as *mut Search) };
        let mut process = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut process) };
        if process == search.process && unsafe { IsWindowVisible(hwnd) } == 0 {
            let mut text = [0u16; 64];
            let n = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
            if n > 0 && text[..n as usize] == search.title[..] {
                unsafe { ShowWindow(hwnd, SW_SHOW) };
                search.shown += 1;
            }
        }
        // Go on to the next window.
        1
    }
    let mut search = Search {
        title: title.encode_utf16().collect(),
        process: unsafe { GetCurrentProcessId() },
        shown: 0,
    };
    unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
    search.shown
}
