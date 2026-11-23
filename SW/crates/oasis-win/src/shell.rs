//! Open a URL, file, folder or program the way Explorer would. Port of the launch half of
//! `UserLevelProcess` / `ActionExecutor`; the "start at normal rights while running as
//! administrator" part comes with elevation support (step W6).

use std::ptr;

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `ShellExecute` "open" on `target`, with optional command-line `args`. `Err` carries the
/// code Windows returned (31 = no program is associated, 2 = not found, 5 = access denied).
#[allow(unsafe_code)]
pub fn open(target: &str, args: Option<&str>) -> Result<(), u32> {
    let verb = wide("open");
    let target = wide(target);
    let args = args.map(wide);
    // SAFETY: every pointer is either null or a NUL-terminated UTF-16 buffer that outlives the
    // call.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            args.as_ref().map_or(ptr::null(), |a| a.as_ptr()),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success; below are error codes.
    let code = result as usize;
    if code > 32 {
        Ok(())
    } else {
        Err(code as u32)
    }
}

/// The file a Start Menu shortcut (`.lnk`) starts, or `None` if it cannot be read.
#[allow(unsafe_code)]
pub fn shortcut_target(lnk: &str) -> Option<String> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let mut buf = [0u16; 1024];
    // SAFETY: COM is initialised for this thread first (an "already initialised" result is fine);
    // the interfaces are owned and released by their wrappers; `buf` outlives `GetPath`, which
    // writes at most `buf.len()` units, and a null find-data pointer is allowed.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(lnk), STGM_READ).ok()?;
        link.GetPath(&mut buf, ptr::null_mut(), 0).ok()?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len])).filter(|s| !s.is_empty())
}

/// The folders whose contents are on screen: the Desktop (the user's, wherever OneDrive moved it,
/// and the Public one) and the folder each open File Explorer window shows. Windows that show no
/// real folder (This PC, Home) are left out. `true` marks a folder of the window in front (every
/// tab of it: Windows 11 tabs share one window). Also returns the paths selected in the window in
/// front, or on the Desktop when the desktop is in front (for "open this").
#[allow(unsafe_code)]
pub fn on_screen_folders() -> (Vec<(String, bool)>, Vec<String>) {
    use windows::core::{Interface, HSTRING, PWSTR};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::System::Com::{IDispatch, IServiceProvider};
    use windows::Win32::UI::Shell::{
        IShellBrowser, IShellFolderViewDual, IShellWindows, IWebBrowser2, PathCreateFromUrlW, SHGetKnownFolderPath, ShellWindows,
        CSIDL_DESKTOP, FOLDERID_Desktop, FOLDERID_PublicDesktop, KF_FLAG_DEFAULT, SID_STopLevelBrowser, SVGIO_BACKGROUND, SWC_DESKTOP,
        SWFO_NEEDDISPATCH,
    };

    // The paths selected in one folder view (at most 20).
    // SAFETY: plain COM calls on interfaces owned and released by their wrappers.
    let selection = |view: Option<IShellFolderViewDual>| -> Vec<String> {
        unsafe {
            let Some(items) = view.and_then(|v| v.SelectedItems().ok()) else { return Vec::new() };
            (0..items.Count().unwrap_or(0).min(20))
                .filter_map(|i| items.Item(&VARIANT::from(i)).ok()?.Path().ok().map(|p| p.to_string()))
                .filter(|p| !p.is_empty())
                .collect()
        }
    };
    let mut folders = Vec::new();
    let mut selected = Vec::new();
    // SAFETY: COM is initialised for this thread first ("already initialised" is fine). Each
    // known-folder string is copied, then freed once with CoTaskMemFree as the API requires. The
    // interfaces are owned and released by their wrappers. `buf` outlives PathCreateFromUrlW,
    // which writes at most `len` units.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        for id in [FOLDERID_Desktop, FOLDERID_PublicDesktop] {
            if let Ok(p) = SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None) {
                if let Ok(s) = p.to_string() {
                    folders.push((s, false));
                }
                CoTaskMemFree(Some(p.0 as *const _));
            }
        }
        let front_window = crate::window::foreground_window();
        let front = front_window.map(|w| w.0);
        let Ok(windows) = CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_LOCAL_SERVER) else { return (folders, selected) };
        if front_window.is_some_and(crate::window::is_shell_window) {
            // The desktop's own folder view, reached through its shell browser.
            let mut hwnd = 0i32;
            let view = windows
                .FindWindowSW(&VARIANT::from(CSIDL_DESKTOP as i32), &VARIANT::default(), SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)
                .and_then(|d| d.cast::<IServiceProvider>())
                .and_then(|sp| sp.QueryService::<IShellBrowser>(&SID_STopLevelBrowser))
                .and_then(|b| b.QueryActiveShellView())
                .and_then(|v| v.GetItemObject::<IDispatch>(SVGIO_BACKGROUND))
                .and_then(|d| d.cast::<IShellFolderViewDual>());
            selected.extend(selection(view.ok()));
        }
        for i in 0..windows.Count().unwrap_or(0).min(50) {
            let Ok(browser) = windows.Item(&VARIANT::from(i)).and_then(|d| d.cast::<IWebBrowser2>()) else { continue };
            let Ok(url) = browser.LocationURL() else { continue };
            let url = url.to_string();
            if !url.starts_with("file:") {
                continue;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            if PathCreateFromUrlW(&HSTRING::from(url), PWSTR(buf.as_mut_ptr()), &mut len, 0).is_ok() {
                let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let in_front = front.is_some() && browser.HWND().ok().map(|h| h.0) == front;
                if in_front {
                    selected.extend(selection(browser.Document().and_then(|d| d.cast::<IShellFolderViewDual>()).ok()));
                }
                folders.push((String::from_utf16_lossy(&buf[..end]), in_front));
            }
        }
    }
    (folders, selected)
}

/// The program Windows opens web links with (the default browser), as a full path; `None` when
/// none is set.
#[allow(unsafe_code)]
pub fn default_browser() -> Option<String> {
    use windows_sys::Win32::UI::Shell::{AssocQueryStringW, ASSOCF_NONE, ASSOCSTR_EXECUTABLE};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (scheme, verb) = (wide("https"), wide("open"));
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: both strings are NUL-terminated and live across the call; `buf` is writable for
    // `len` UTF-16 units, which the call reads and updates.
    let hr = unsafe { AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_EXECUTABLE, scheme.as_ptr(), verb.as_ptr(), buf.as_mut_ptr(), &mut len) };
    if hr != 0 {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let path = String::from_utf16_lossy(&buf[..end]);
    (!path.trim().is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_is_reported_not_ignored() {
        assert!(open("C:\\definitely\\not\\here\\oasis-test.exe", None).is_err());
    }

    #[test]
    fn strings_are_nul_terminated_utf16() {
        assert_eq!(wide("hé"), [0x68, 0xE9, 0]);
    }

    #[test]
    fn a_shortcut_target_is_read() {
        let dir = std::env::temp_dir();
        let lnk = dir.join(format!("oasis-lnk-test-{}.lnk", std::process::id()));
        let target = std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
        let made = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath='{}'; $s.Save()",
                lnk.display(),
                target
            ))
            .status()
            .unwrap();
        assert!(made.success());
        let read = shortcut_target(&lnk.to_string_lossy());
        let _ = std::fs::remove_file(&lnk);
        assert_eq!(read.map(|s| s.to_lowercase()), Some(target.to_lowercase()));
        assert_eq!(shortcut_target(r"C:\no\such\file.lnk"), None);
    }

    #[test]
    fn the_default_browser_is_a_program_that_exists() {
        // On a PC without a browser set this is None; where one is set it must be a real file.
        if let Some(path) = default_browser() {
            assert!(std::path::Path::new(&path).is_file(), "{path}");
            assert!(path.to_lowercase().ends_with(".exe"), "{path}");
        }
    }
}

/// A Yes/No box in front of every window; `true` when Yes is pressed. Blocks until answered.
#[allow(unsafe_code)]
pub fn ask_yes_no(title: &str, text: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO};
    let (title, text) = (wide(title), wide(text));
    // SAFETY: both are NUL-terminated UTF-16 buffers that outlive the call; a null owner is allowed.
    let answer = unsafe { MessageBoxW(ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND) };
    answer == IDYES
}
