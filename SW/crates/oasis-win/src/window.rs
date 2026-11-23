//! Foreground window, title and owning process. Port of the Win32 half of `ContextTracker`.

use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible,
};

/// A window handle. Only meaningful while the window exists; every function below tolerates a
/// handle that has gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub isize);

impl WindowId {
    pub(crate) fn hwnd(self) -> HWND {
        self.0 as HWND
    }
}

/// The window the user is working in, or `None` when there is none (lock screen, desktop
/// switching).
#[allow(unsafe_code)]
pub fn foreground_window() -> Option<WindowId> {
    // SAFETY: no arguments, no preconditions.
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_null()).then_some(WindowId(hwnd as isize))
}

/// The window's title; empty if it has none or is gone.
#[allow(unsafe_code)]
pub fn window_title(window: WindowId) -> String {
    // SAFETY: a stale handle makes the call fail (returns 0), it does not fault.
    let len = unsafe { GetWindowTextLengthW(window.hwnd()) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    // SAFETY: `buf` is writable for `buf.len()` UTF-16 units, which is what we pass.
    let copied = unsafe { GetWindowTextW(window.hwnd(), buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..copied.max(0) as usize])
}

/// The id of the process that owns the window.
#[allow(unsafe_code)]
pub fn window_process_id(window: WindowId) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out pointer for the duration of the call.
    let thread = unsafe { GetWindowThreadProcessId(window.hwnd(), &mut pid) };
    (thread != 0 && pid != 0).then_some(pid)
}

/// The executable name without folder or `.exe` ("chrome"), as .NET `Process.ProcessName`.
/// Tries the process's image path first; for processes we may not open (elevated, protected)
/// falls back to the system process list, which names every process.
pub fn process_name(pid: u32) -> Option<String> {
    image_path(pid)
        .and_then(|path| stem(&path))
        .or_else(|| snapshot_exe_name(pid).and_then(|name| stem(&name)))
}

fn stem(path: &str) -> Option<String> {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty())
}

#[allow(unsafe_code)]
pub(crate) fn image_path(pid: u32) -> Option<String> {
    // SAFETY: plain call; a null handle means it failed.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `process` is a live handle we own; `buf` is writable for `len` units and `len`
    // is updated to the number written.
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) };
    // SAFETY: `process` came from OpenProcess above and is closed exactly once.
    unsafe { CloseHandle(process) };
    (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

#[allow(unsafe_code)]
fn snapshot_exe_name(pid: u32) -> Option<String> {
    // SAFETY: plain call; INVALID_HANDLE_VALUE (-1) means it failed.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() || snapshot as isize == -1 {
        return None;
    }
    // SAFETY: an all-zero PROCESSENTRY32W is valid; `dwSize` is set as the API requires.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = None;
    // SAFETY: `snapshot` is a live handle we own and `entry` is a valid, sized out struct.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        if entry.th32ProcessID == pid {
            let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            found = Some(String::from_utf16_lossy(&entry.szExeFile[..end]));
            break;
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: `snapshot` came from CreateToolhelp32Snapshot and is closed exactly once.
    unsafe { CloseHandle(snapshot) };
    found
}

/// The programs that have a titled, visible window now: `(process name, window title)`, one per
/// program, in the order Windows lists them (front to back). Port of `RunningAppsProbe`.
#[allow(unsafe_code)]
pub fn visible_windows() -> Vec<(String, String)> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: `lparam` is the address of the Vec passed to EnumWindows below, alive for the call.
        let found = unsafe { &mut *(lparam as *mut Vec<WindowId>) };
        // SAFETY: a stale handle makes the call return 0.
        if unsafe { IsWindowVisible(hwnd) } != 0 {
            found.push(WindowId(hwnd as isize));
        }
        1
    }
    let mut windows: Vec<WindowId> = Vec::new();
    // SAFETY: the callback only reads its arguments and pushes to the Vec we own.
    unsafe { EnumWindows(Some(collect), &mut windows as *mut Vec<WindowId> as LPARAM) };

    let mut seen = std::collections::HashSet::new();
    let mut programs = Vec::new();
    for window in windows {
        let title = window_title(window);
        if title.trim().is_empty() {
            continue;
        }
        let Some(name) = window_process_id(window).and_then(process_name) else { continue };
        if seen.insert(name.to_lowercase()) {
            programs.push((name, title));
        }
    }
    programs
}

/// The desktop or the taskbar. Never closed: a close sent to "Program Manager" opens the
/// shut-down dialog.
#[allow(unsafe_code)]
pub fn is_shell_window(window: WindowId) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;
    let mut class = [0u16; 64];
    // SAFETY: `class` is writable for its length, which is what we pass; a stale handle returns 0.
    let len = unsafe { GetClassNameW(window.hwnd(), class.as_mut_ptr(), class.len() as i32) };
    let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
    matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

/// Ask every titled, visible top-level window of the program named `process` to close, as its own
/// close button would: the program may still ask to save. Returns how many windows were asked.
/// The desktop and the taskbar are never asked (closing "Program Manager" opens the shut-down
/// dialog), nor is OASIS itself.
#[allow(unsafe_code)]
pub fn close_program(process: &str) -> usize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: `lparam` is the address of the Vec passed to EnumWindows below, alive for the call.
        let found = unsafe { &mut *(lparam as *mut Vec<WindowId>) };
        // SAFETY: a stale handle makes the call return 0.
        if unsafe { IsWindowVisible(hwnd) } != 0 {
            found.push(WindowId(hwnd as isize));
        }
        1
    }
    let wanted = process.trim().trim_end_matches(".exe").to_lowercase();
    if wanted.is_empty() {
        return 0;
    }
    let mut windows: Vec<WindowId> = Vec::new();
    // SAFETY: the callback only reads its arguments and pushes to the Vec we own.
    unsafe { EnumWindows(Some(collect), &mut windows as *mut Vec<WindowId> as LPARAM) };
    let own = std::process::id();
    let mut asked = 0;
    for window in windows {
        if window_title(window).trim().is_empty() {
            continue;
        }
        let Some(pid) = window_process_id(window).filter(|p| *p != own) else { continue };
        if process_name(pid).is_none_or(|n| n.trim_end_matches(".exe").to_lowercase() != wanted) {
            continue;
        }
        if is_shell_window(window) {
            continue;
        }
        // SAFETY: posting to a window that has since closed simply fails.
        if unsafe { PostMessageW(window.hwnd(), WM_CLOSE, 0, 0) } != 0 {
            asked += 1;
        }
    }
    asked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_a_program_that_is_not_running_asks_no_window() {
        // Only safe names: a test must never close a program the developer has open.
        assert_eq!(close_program(""), 0);
        assert_eq!(close_program("no-such-program-oasis-test"), 0);
    }

    #[test]
    fn names_our_own_process() {
        let name = process_name(std::process::id()).expect("own process has a name");
        assert!(name.starts_with("oasis_win"), "{name}");
    }

    #[test]
    fn names_a_process_we_cannot_open_through_the_process_list() {
        // pid 4 is the System process: not openable, but present in the process list.
        assert_eq!(snapshot_exe_name(4).as_deref(), Some("System"));
        assert_eq!(process_name(4).as_deref(), Some("System"));
    }

    #[test]
    fn unknown_process_has_no_name() {
        assert_eq!(process_name(u32::MAX - 1), None);
    }

    #[test]
    fn the_visible_windows_name_their_programs_once_each() {
        let programs = visible_windows();
        let mut names: Vec<String> = programs.iter().map(|(n, _)| n.to_lowercase()).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "one entry per program");
        assert!(programs.iter().all(|(n, t)| !n.is_empty() && !t.trim().is_empty()));
    }

    #[test]
    fn a_missing_window_is_harmless() {
        let gone = WindowId(0x7fff_fff0);
        assert_eq!(window_title(gone), "");
        assert_eq!(window_process_id(gone), None);
    }

    #[test]
    fn the_foreground_window_resolves_when_there_is_one() {
        // Headless sessions have none; then there is nothing to check.
        if let Some(window) = foreground_window() {
            let pid = window_process_id(window).expect("foreground window has a process");
            assert!(process_name(pid).is_some());
        }
    }
}
