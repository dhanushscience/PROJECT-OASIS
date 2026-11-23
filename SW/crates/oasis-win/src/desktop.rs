//! The desktop for Quick Launch: list app windows, read where a window is, and move it. Port of
//! `Features/QuickLaunch/Win32QuickLaunchDesktop.cs`.
//!
//! Every call runs per-monitor DPI aware, so rectangles are real pixels even with two screens
//! scaled differently. A window that has gone, or that Windows will not let OASIS touch (one
//! running as administrator), yields `None` or is left alone, never a panic.

use std::mem::size_of;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, PostMessageW, GetCursorPos, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowRect, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
    SetWindowPos, ShowWindow, GWL_EXSTYLE, GW_OWNER, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WM_CLOSE, WS_EX_TOOLWINDOW,
};

use crate::window::{process_name, window_process_id, window_title, WindowId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Rect {
        Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppWindow {
    pub handle: i64,
    pub process: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Monitor {
    pub handle: isize,
    pub bounds: Rect,
    pub work: Rect,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub outer: Rect,
    pub visible: Rect,
    pub maximized: bool,
    pub minimized: bool,
    /// Which monitor (`Monitor::handle`) the window is mostly on.
    pub monitor: isize,
}

/// Runs `action` on a per-monitor DPI aware thread, then puts the old awareness back.
#[allow(unsafe_code)]
fn per_monitor<T>(action: impl FnOnce() -> T) -> T {
    // SAFETY: plain call; returns the previous context, or null on an old Windows.
    let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let result = action();
    if !previous.is_null() {
        // SAFETY: restores the context returned above.
        unsafe { SetThreadDpiAwarenessContext(previous) };
    }
    result
}

/// Visible top-level app windows, front to back: shown, not owned, not tool windows, not cloaked
/// (a suspended Store app keeps an invisible window), titled and of a real size.
#[allow(unsafe_code)]
pub fn app_windows() -> Vec<AppWindow> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: `lparam` is the address of the Vec passed to EnumWindows below, alive for the call.
        let found = unsafe { &mut *(lparam as *mut Vec<HWND>) };
        found.push(hwnd);
        1
    }
    per_monitor(|| {
        let mut all: Vec<HWND> = Vec::new();
        // SAFETY: the callback only pushes to the Vec we own.
        unsafe { EnumWindows(Some(collect), &mut all as *mut Vec<HWND> as LPARAM) };
        let mut out = Vec::new();
        for hwnd in all {
            // SAFETY: each call tolerates a handle that has just gone.
            let skip = unsafe {
                IsWindowVisible(hwnd) == 0
                    || !GetWindow(hwnd, GW_OWNER).is_null()
                    || (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW) != 0
            };
            if skip || is_cloaked(hwnd) {
                continue;
            }
            let id = WindowId(hwnd as isize);
            let title = window_title(id);
            let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            // SAFETY: valid out-pointer.
            let got = unsafe { GetWindowRect(hwnd, &mut rect) } != 0;
            // A minimized window reports a small strip far off screen, so its size says nothing: it is
            // still the app's window (Quick Launch must reuse and restore it, not start the app again).
            // SAFETY: plain call on a window handle.
            let minimized = unsafe { IsIconic(hwnd) } != 0;
            let too_small = !minimized && (rect.right - rect.left < 120 || rect.bottom - rect.top < 80);
            if title.is_empty() || !got || too_small {
                continue;
            }
            let process = window_process_id(id).and_then(process_name).unwrap_or_default();
            out.push(AppWindow { handle: hwnd as i64, process, title });
        }
        out
    })
}

#[allow(unsafe_code)]
fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: i32 = 0;
    // SAFETY: valid out-pointer of the stated size.
    let ok = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut cloaked as *mut i32 as *mut _, size_of::<i32>() as u32) };
    ok == 0 && cloaked != 0
}

#[allow(unsafe_code)]
pub fn foreground() -> i64 {
    // SAFETY: plain call.
    unsafe { GetForegroundWindow() as i64 }
}

/// The display (`Monitor::handle`) the mouse pointer is on.
#[allow(unsafe_code)]
pub fn cursor_monitor() -> Option<isize> {
    per_monitor(|| {
        let mut at = POINT { x: 0, y: 0 };
        // SAFETY: valid out-pointer.
        if unsafe { GetCursorPos(&mut at) } == 0 {
            return None;
        }
        // SAFETY: plain call.
        Some(unsafe { MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST) } as isize)
    })
}

/// Every display, in Windows' own order.
#[allow(unsafe_code)]
pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, lparam: LPARAM) -> i32 {
        // SAFETY: `lparam` is the address of the Vec passed below, alive for the call.
        let found = unsafe { &mut *(lparam as *mut Vec<Monitor>) };
        // SAFETY: an all-zero MONITORINFO is valid plain data.
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MONITORINFO>() as u32;
        // SAFETY: `info` has its size set and is a valid out-pointer.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            found.push(Monitor { handle: monitor as isize, bounds: info.rcMonitor.into(), work: info.rcWork.into(), primary: info.dwFlags & 1 != 0 });
        }
        1
    }
    per_monitor(|| {
        let mut list: Vec<Monitor> = Vec::new();
        // SAFETY: the callback only pushes to the Vec we own.
        unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(collect), &mut list as *mut Vec<Monitor> as LPARAM) };
        list
    })
}

/// Where a window is, or `None` if it has gone. `visible` is the frame the user sees (outer minus
/// the invisible resize border); it falls back to `outer` when DWM cannot say.
#[allow(unsafe_code)]
pub fn read(handle: i64) -> Option<Placement> {
    let hwnd = handle as HWND;
    per_monitor(|| {
        let mut outer = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: a handle check, then a valid out-pointer.
        if unsafe { IsWindow(hwnd) } == 0 || unsafe { GetWindowRect(hwnd, &mut outer) } == 0 {
            return None;
        }
        let mut visible = outer;
        // SAFETY: valid out-pointer of the stated size.
        let ok = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS as u32, &mut visible as *mut RECT as *mut _, size_of::<RECT>() as u32) };
        if ok != 0 {
            visible = outer;
        }
        // SAFETY: plain calls on a window handle.
        let (maximized, minimized, monitor) =
            unsafe { (IsZoomed(hwnd) != 0, IsIconic(hwnd) != 0, MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) as isize) };
        Some(Placement { outer: outer.into(), visible: visible.into(), maximized, minimized, monitor })
    })
}

#[allow(unsafe_code)]
pub fn restore(handle: i64) {
    // SAFETY: a stale handle makes the call do nothing.
    per_monitor(|| unsafe { ShowWindow(handle as HWND, SW_RESTORE) });
}

#[allow(unsafe_code)]
pub fn maximize(handle: i64) {
    // SAFETY: a stale handle makes the call do nothing.
    per_monitor(|| unsafe { ShowWindow(handle as HWND, SW_MAXIMIZE) });
}

#[allow(unsafe_code)]
pub fn minimize(handle: i64) {
    // SAFETY: a stale handle makes the call do nothing.
    per_monitor(|| unsafe { ShowWindow(handle as HWND, SW_MINIMIZE) });
}

/// Ask one window to close, as its own close button would (it may still ask to save). Never the
/// desktop or the taskbar ([`crate::window::is_shell_window`]): that opens the shut-down dialog.
#[allow(unsafe_code)]
pub fn close(handle: i64) -> bool {
    if crate::window::is_shell_window(WindowId(handle as isize)) {
        return false;
    }
    // SAFETY: a stale handle makes the call fail, not fault.
    unsafe { PostMessageW(handle as HWND, WM_CLOSE, 0, 0) != 0 }
}

/// Put a topmost window back at the top of the topmost windows, without moving or activating it.
/// Windows can leave a never-focused topmost window under ordinary ones after "Show desktop", a
/// full-screen app or a display change, while its topmost flag still says otherwise.
#[allow(unsafe_code)]
pub fn raise_topmost(handle: i64) {
    // SAFETY: a stale handle makes the call do nothing.
    unsafe {
        SetWindowPos(handle as HWND, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    }
}

/// Move and size the window's outer rectangle, without activating it.
#[allow(unsafe_code)]
pub fn set_outer(handle: i64, outer: Rect) {
    // SAFETY: a stale handle makes the call do nothing.
    per_monitor(|| unsafe {
        SetWindowPos(handle as HWND, std::ptr::null_mut(), outer.left, outer.top, outer.right - outer.left, outer.bottom - outer.top, SWP_NOZORDER | SWP_NOACTIVATE)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_is_never_asked_to_close() {
        // "Program Manager" is the desktop: a close would open the shut-down dialog. Only the
        // guard is exercised: `close` returns false before posting anything.
        let Some(desktop) = app_windows().into_iter().find(|w| w.title == "Program Manager") else { return };
        assert!(crate::window::is_shell_window(WindowId(desktop.handle as isize)));
        assert!(!close(desktop.handle));
    }

    #[test]
    fn this_machine_has_a_screen_with_a_work_area_inside_it() {
        let screens = monitors();
        if screens.is_empty() {
            return; // a headless build machine
        }
        assert!(screens.iter().any(|m| m.primary));
        for m in screens {
            assert!(m.work.left >= m.bounds.left && m.work.right <= m.bounds.right);
        }
    }

    #[test]
    fn a_window_that_does_not_exist_reads_as_none() {
        assert!(read(0x7fff_fff0).is_none());
    }
}
