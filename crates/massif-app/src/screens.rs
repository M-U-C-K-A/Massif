//! The screens a floating panel's window can stand on: a window saved on a
//! screen unplugged since, or moved off every screen, opens on one there is.

use egui::{Pos2, Rect, pos2, vec2};

/// The usable part of each screen (without the menu bar, the Dock or the
/// taskbar) in egui's points, as the windows' positions are given: the
/// system's logical pixels over the interface's zoom. Empty where the system
/// does not say (Linux, where the window manager places windows itself).
pub fn visible(ctx: &egui::Context) -> Vec<Rect> {
    let native = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0);
    let zoom = ctx.zoom_factor();
    logical(native).into_iter().map(|r| Rect::from_min_max((r.min.to_vec2() / zoom).to_pos2(), (r.max.to_vec2() / zoom).to_pos2())).collect()
}

/// Where a window of `rect` (its frame's top left corner, its inside's
/// size) opens on `screens`: there when enough of its top, the part it is
/// moved by, is on one of them; else whole on the screen `home` is on (the
/// first one without it), as large as it fits. Unchanged without screens.
pub fn place(rect: Rect, screens: &[Rect], home: Option<Pos2>) -> Rect {
    if screens.is_empty() {
        return rect;
    }
    let grip = Rect::from_min_size(rect.min, vec2(rect.width(), 28.0));
    let reachable = screens.iter().any(|s| {
        let common = s.intersect(grip);
        common.width() >= rect.width().min(80.0) && common.height() >= 12.0
    });
    if reachable {
        return rect;
    }
    let screen = home.and_then(|p| screens.iter().find(|s| s.contains(p))).unwrap_or(&screens[0]);
    let size = rect.size().min(screen.size() * 0.9);
    let min = pos2(rect.min.x.clamp(screen.left(), screen.right() - size.x), rect.min.y.clamp(screen.top(), screen.bottom() - size.y));
    Rect::from_min_size(min, size)
}

/// The screens' usable parts in the system's logical pixels: AppKit's
/// points, turned to count down from the top of the first screen as winit
/// does.
#[cfg(target_os = "macos")]
fn logical(_native: f32) -> Vec<Rect> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    let Some(mtm) = MainThreadMarker::new() else { return Vec::new() };
    let screens = NSScreen::screens(mtm);
    if screens.count() == 0 {
        return Vec::new();
    }
    // AppKit counts up from the bottom of the first screen.
    let top = screens.objectAtIndex(0).frame().size.height;
    (0..screens.count())
        .map(|k| {
            let f = screens.objectAtIndex(k).visibleFrame();
            Rect::from_min_size(pos2(f.origin.x as f32, (top - f.origin.y - f.size.height) as f32), vec2(f.size.width as f32, f.size.height as f32))
        })
        .collect()
}

/// The monitors' work areas, in physical pixels turned logical by the
/// main window's scale (close enough on screens of other scales: only to
/// keep a window in reach).
#[cfg(windows)]
fn logical(native: f32) -> Vec<Rect> {
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> windows_sys::core::BOOL {
        // SAFETY: `data` is the vector `logical` passes, alive during the call.
        let out = unsafe { &mut *(data as *mut Vec<Rect>) };
        // SAFETY: MONITORINFO is plain data, its size set as asked.
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        // SAFETY: a monitor the system enumerates, and the structure above.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            let r = info.rcWork;
            out.push(Rect::from_min_max(pos2(r.left as f32, r.top as f32), pos2(r.right as f32, r.bottom as f32)));
        }
        1
    }
    let mut out: Vec<Rect> = Vec::new();
    // SAFETY: every monitor of the desktop, the callback given the vector.
    unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(each), &mut out as *mut Vec<Rect> as LPARAM) };
    let scale = native.max(0.5);
    out.into_iter().map(|r| Rect::from_min_max((r.min.to_vec2() / scale).to_pos2(), (r.max.to_vec2() / scale).to_pos2())).collect()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn logical(_native: f32) -> Vec<Rect> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_open_where_they_can_be_reached() {
        let laptop = Rect::from_min_size(pos2(0.0, 25.0), vec2(1512.0, 920.0));
        let left = Rect::from_min_size(pos2(-2560.0, -400.0), vec2(2560.0, 1415.0));
        let screens = [laptop, left];
        // On a screen, or on the one beside it: where it was.
        let on = Rect::from_min_size(pos2(200.0, 300.0), vec2(400.0, 300.0));
        assert_eq!(place(on, &screens, None), on);
        let beside = Rect::from_min_size(pos2(-1800.0, 100.0), vec2(400.0, 300.0));
        assert_eq!(place(beside, &screens, None), beside);
        // Its top on a screen while the rest hangs below: still reachable.
        let low = Rect::from_min_size(pos2(300.0, 900.0), vec2(400.0, 300.0));
        assert_eq!(place(low, &screens, None), low);
        // Its top under the menu bar, or on a screen unplugged since: whole
        // on the screen of the main window.
        let up = Rect::from_min_size(pos2(300.0, 0.0), vec2(400.0, 300.0));
        assert_eq!(place(up, &screens, Some(pos2(100.0, 100.0))).min, pos2(300.0, 25.0));
        let gone = Rect::from_min_size(pos2(3000.0, 200.0), vec2(400.0, 300.0));
        let back = place(gone, &screens, Some(pos2(-100.0, 0.0)));
        assert!(left.contains_rect(back), "{back:?}");
        assert_eq!(back.size(), gone.size());
        // Larger than the screen: made to fit.
        let huge = Rect::from_min_size(pos2(5000.0, 5000.0), vec2(3000.0, 2000.0));
        let fit = place(huge, &screens, None);
        assert!(laptop.contains_rect(fit), "{fit:?}");
        // Screens unknown: as saved.
        assert_eq!(place(gone, &[], None), gone);
    }
}
