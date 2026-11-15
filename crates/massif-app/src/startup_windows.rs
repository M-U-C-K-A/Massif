//! The Windows side of the start: Massif's window, shown before its first
//! frame; a crash written down with its exception and the module it came
//! from (a graphics driver, say), since it ends the process with no panic;
//! whether another Massif still runs; ending this one at once (the main
//! thread may be stuck in a driver).

use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HMODULE, HWND, STILL_ACTIVE};
use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER, SetUnhandledExceptionFilter};
use windows_sys::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, TerminateProcess};
use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindowVisible, SW_SHOW, ShowWindow};

/// The filter of unhandled exceptions there was before Massif's.
static PREVIOUS: AtomicUsize = AtomicUsize::new(0);

/// Show Massif's window `hwnd` now, from the thread it belongs to; whether
/// it was hidden.
pub fn show_now(hwnd: isize) -> bool {
    let hwnd = hwnd as HWND;
    // Safety: the window eframe made, on its own thread.
    unsafe {
        if IsWindowVisible(hwnd) != 0 {
            return false;
        }
        ShowWindow(hwnd, SW_SHOW);
    }
    true
}

/// Write down an exception no one handles (an access violation in a
/// driver, a stack overflow) before Windows ends the process.
pub fn catch_crashes() {
    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        // Safety: Windows passes the exception's record, alive for the call.
        if let Some(record) = unsafe { info.as_ref() }.and_then(|i| unsafe { i.ExceptionRecord.as_ref() }) {
            let code = record.ExceptionCode as u32;
            let at = record.ExceptionAddress;
            super::write_now(&format!("crash: exception {code:#010x}{} at {at:?}{}", what(code), module_of(at as usize).map(|m| format!(" in {m}")).unwrap_or_default()));
        }
        let previous = PREVIOUS.load(Ordering::Relaxed);
        if previous != 0 {
            // Safety: the filter that was installed before ours, kept as it was.
            let previous: unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32 = unsafe { std::mem::transmute(previous) };
            return unsafe { previous(info) };
        }
        // EXCEPTION_CONTINUE_SEARCH: Windows ends the process as it would have.
        0
    }
    // Safety: the filter only reads the record it is given.
    let previous: LPTOP_LEVEL_EXCEPTION_FILTER = unsafe { SetUnhandledExceptionFilter(Some(filter)) };
    if let Some(previous) = previous {
        PREVIOUS.store(previous as usize, Ordering::Relaxed);
    }
}

/// What an exception code means, for the common ones.
fn what(code: u32) -> &'static str {
    match code {
        0xC000_0005 => " (access violation)",
        0xC000_00FD => " (stack overflow)",
        0xC000_001D => " (illegal instruction)",
        0xC000_0094 => " (integer division by zero)",
        0xC000_0374 => " (heap corruption)",
        0xC000_0409 => " (fail fast)",
        0x8000_0003 => " (breakpoint)",
        _ => "",
    }
}

/// The file of the module (executable or library) holding `address`.
fn module_of(address: usize) -> Option<String> {
    let mut module: HMODULE = std::ptr::null_mut();
    // Safety: asks which module holds an address, without holding it.
    let found = unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, address as *const u16, &mut module) };
    if found == 0 {
        return None;
    }
    let mut name = [0u16; 260];
    // Safety: the buffer and its length in characters.
    let n = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
    (n > 0).then(|| String::from_utf16_lossy(&name[..n.min(name.len())]))
}

/// Whether process `pid` still runs, and is Massif (the same program as
/// this one: a process number is soon given to another).
pub fn running(pid: u32) -> bool {
    let file_name = |path: &str| std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_lowercase());
    let ours = std::env::current_exe().ok().and_then(|p| file_name(&p.to_string_lossy()));
    // Safety: a handle opened to read the exit code and the program's path,
    // closed after; the buffer and its length in characters.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0u32;
        let alive = GetExitCodeProcess(process, &mut code) != 0 && code == STILL_ACTIVE as u32;
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        let theirs = named.then(|| String::from_utf16_lossy(&path[..(len as usize).min(path.len())])).and_then(|p| file_name(&p));
        alive && (ours.is_none() || theirs == ours)
    }
}

/// End this process at once, whatever its threads are stuck in.
pub fn end_now() -> ! {
    // Safety: the pseudo-handle of this process.
    unsafe { TerminateProcess(GetCurrentProcess(), 1) };
    std::process::abort()
}
