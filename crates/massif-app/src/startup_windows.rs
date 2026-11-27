//! The Windows side of the start: Massif's window, shown before its first
//! frame; a crash written down with its exception and the module it came
//! from (a graphics driver, say), since it ends the process with no panic;
//! whether another Massif still runs; ending this one at once (the main
//! thread may be stuck in a driver); where the main thread is when it does
//! not answer (the code it is in, by module and offset); the threads that
//! load in the background below the interface's priority.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, HWND, STILL_ACTIVE};
use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER, SetUnhandledExceptionFilter};
use windows_sys::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, GetCurrentThreadId, GetCurrentThreadStackLimits, GetExitCodeProcess, OpenProcess, OpenThread, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, ResumeThread, SetThreadPriority, SuspendThread, THREAD_GET_CONTEXT, THREAD_PRIORITY_BELOW_NORMAL, THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME, TerminateProcess};
use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindowVisible, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW, SW_SHOW, ShowWindow};

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

/// A message box of the system, waited on: MessageBoxW itself, which needs
/// nothing more than Windows (a crash may leave no COM or event loop).
pub fn message_box(text: &str) {
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Massif"));
    // Safety: two strings ending in 0, alive for the call; no owner window.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR | MB_SETFOREGROUND) };
}

/// Write down an exception no one handles (an access violation in a
/// driver, a stack overflow) before Windows ends the process.
pub fn catch_crashes() {
    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        // Safety: Windows passes the exception's record, alive for the call.
        if let Some(record) = unsafe { info.as_ref() }.and_then(|i| unsafe { i.ExceptionRecord.as_ref() }) {
            let code = record.ExceptionCode as u32;
            let at = record.ExceptionAddress;
            let crash = format!("exception {code:#010x}{} at {at:?}{}", what(code), module_of(at as usize).map(|m| format!(" in {m}")).unwrap_or_default());
            super::write_now(&format!("crash: {crash}"));
            // A stack overflow leaves no stack to show a message box on.
            if code != 0xC000_00FD {
                super::crashed_before_window(&crash);
            }
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

/// The main thread, opened to be looked at from the watch.
static MAIN: AtomicUsize = AtomicUsize::new(0);
/// Its stack's bounds: what is read of it stays within them.
static STACK: (AtomicUsize, AtomicUsize) = (AtomicUsize::new(0), AtomicUsize::new(0));
/// A reading of its stack was given up: it stops where it is.
static GIVEN_UP: AtomicBool = AtomicBool::new(false);

/// Remember the thread this is called on as the main thread.
pub fn remember_main_thread() {
    // Safety: opens the calling thread by its id, kept open for the run.
    let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, 0, GetCurrentThreadId()) };
    if !handle.is_null() {
        MAIN.store(handle as usize, Ordering::SeqCst);
    }
    let (mut low, mut high) = (0usize, 0usize);
    // Safety: writes the calling thread's stack bounds.
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    STACK.0.store(low, Ordering::SeqCst);
    STACK.1.store(high, Ordering::SeqCst);
}

/// Run the threads that call this below the interface's priority: on a
/// computer with few cores, the work done in the background then never
/// keeps the main thread from drawing.
pub fn below_normal_priority() {
    // Safety: the pseudo-handle of the calling thread.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
}

/// The most frames of the main thread's stack written down.
const FRAMES: usize = 24;

/// Where the main thread is now: each frame of its stack as the module it
/// is in and the offset there (`massif.exe+0x1a2b3c`, `amdvlk64.dll+0x…`),
/// innermost first. `None` when it cannot be told.
///
/// The thread is stopped while its stack is read, by a thread of its own:
/// nothing that could wait on a lock the main thread holds (memory, say)
/// runs while it is stopped, and if the reading does not end at once the
/// main thread is let go on anyway.
#[cfg(target_arch = "x86_64")]
pub fn main_stack() -> Option<String> {
    let main = MAIN.load(Ordering::SeqCst);
    let stack = (STACK.0.load(Ordering::SeqCst) as u64, STACK.1.load(Ordering::SeqCst) as u64);
    if main == 0 || stack.0 >= stack.1 || GIVEN_UP.load(Ordering::SeqCst) {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    std::thread::Builder::new()
        .name("massif main thread's stack".into())
        .spawn(move || {
            let mut frames = [0u64; FRAMES];
            // Safety: see `read_stack`.
            let n = unsafe { read_stack(main as HANDLE, stack, &mut frames) };
            finished.store(true, Ordering::SeqCst);
            let _ = tx.send((frames, n));
        })
        .ok()?;
    match rx.recv_timeout(std::time::Duration::from_millis(500)) {
        Ok((frames, n)) => (n > 0).then(|| frames[..n].iter().map(|&pc| place(pc as usize)).collect::<Vec<_>>().join(" < ")),
        Err(_) => {
            if !done.load(Ordering::SeqCst) {
                // The reading stops at its next frame, and is not tried
                // again in this run.
                GIVEN_UP.store(true, Ordering::SeqCst);
                // Safety: lets the main thread go on, if the reading
                // stopped it and is held up; a thread already running
                // stays as it is.
                unsafe { ResumeThread(main as HANDLE) };
            }
            Some("(its stack could not be read in time)".to_owned())
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn main_stack() -> Option<String> {
    None
}

/// Stop `thread`, read the return addresses on its stack into `frames`,
/// let it go on. Returns how many were read.
///
/// # Safety
/// `thread` is a handle opened with THREAD_SUSPEND_RESUME and
/// THREAD_GET_CONTEXT, of a thread other than the caller, whose stack lies
/// within `stack` (low, high). Nothing between the stop and the start again
/// allocates or takes a lock of Massif's.
#[cfg(target_arch = "x86_64")]
unsafe fn read_stack(thread: HANDLE, stack: (u64, u64), frames: &mut [u64; FRAMES]) -> usize {
    use windows_sys::Win32::System::Diagnostics::Debug::{CONTEXT, CONTEXT_FULL_AMD64, GetThreadContext, RtlLookupFunctionEntry, RtlVirtualUnwind, UNW_FLAG_NHANDLER};
    // GetThreadContext wants it on 16 bytes.
    #[repr(C, align(16))]
    struct Aligned(CONTEXT);
    // Safety: a CONTEXT is plain data, zero a valid value.
    let mut context: Aligned = unsafe { std::mem::zeroed() };
    context.0.ContextFlags = CONTEXT_FULL_AMD64;
    // Safety: as the caller promised.
    if unsafe { SuspendThread(thread) } == u32::MAX {
        return 0;
    }
    let mut n = 0;
    // Safety: the context is aligned and as large as Windows writes.
    if unsafe { GetThreadContext(thread, &mut context.0) } != 0 {
        let ctx = &mut context.0;
        // Only within the stack: an address read wrong ends the reading
        // instead of reading memory elsewhere.
        while n < FRAMES && ctx.Rip != 0 && ctx.Rsp >= stack.0 && ctx.Rsp + 64 <= stack.1 && !GIVEN_UP.load(Ordering::Relaxed) {
            frames[n] = ctx.Rip;
            n += 1;
            let mut base = 0u64;
            // Safety: looks the address up in the unwind tables of the
            // modules loaded; no history table.
            let entry = unsafe { RtlLookupFunctionEntry(ctx.Rip, &mut base, std::ptr::null_mut()) };
            if entry.is_null() {
                // A leaf function: its return address is on top of the stack.
                if ctx.Rsp == 0 {
                    break;
                }
                // Safety: the stopped thread's stack, in this process.
                ctx.Rip = unsafe { (ctx.Rsp as *const u64).read_unaligned() };
                ctx.Rsp += 8;
            } else {
                let mut data = std::ptr::null_mut();
                let mut frame = 0u64;
                // Safety: the entry found for this address, the context
                // unwound in place.
                unsafe { RtlVirtualUnwind(UNW_FLAG_NHANDLER, base, ctx.Rip, entry, ctx, &mut data, &mut frame, std::ptr::null_mut()) };
            }
        }
    }
    // Safety: the thread stopped above.
    unsafe { ResumeThread(thread) };
    n
}

/// An address as the module it is in and the offset there.
fn place(address: usize) -> String {
    let mut module: HMODULE = std::ptr::null_mut();
    // Safety: asks which module holds an address, without holding it.
    let found = unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, address as *const u16, &mut module) };
    if found == 0 {
        return format!("{address:#x}");
    }
    let name = module_of(address).and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "?".to_owned());
    format!("{name}+{:#x}", address - module as usize)
}
