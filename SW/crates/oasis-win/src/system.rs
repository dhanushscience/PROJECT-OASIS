//! Whether OASIS runs elevated, who the user is, the single-copy lock, and the login-startup
//! (Run key) entry. Ports of `ProcessElevation`, `SingleInstanceLock` and `AutostartService`.

use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS,
    ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_ELEVATION,
    TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, GetExitCodeProcess, OpenEventW, OpenMutexW,
    OpenProcessToken, ReleaseMutex, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// The standard access right to wait on an object (not exported by the Threading module).
const SYNCHRONIZE: u32 = 0x0010_0000;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Closes a handle when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the handle was returned by Windows and is closed once, here.
        unsafe { CloseHandle(self.0) };
    }
}

/// Whether this process has administrator rights right now.
#[allow(unsafe_code)]
pub fn is_elevated() -> bool {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `token` is a valid out-pointer; the pseudo-handle of the current process needs no
    // closing.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let token = Owned(token);
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: the buffer is a `TOKEN_ELEVATION` and its size is passed.
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    ok != 0 && elevation.TokenIsElevated != 0
}

/// The signed-in user's security identifier as text (`S-1-5-21-…`), or `None` if unavailable.
#[allow(unsafe_code)]
pub fn current_user_sid() -> Option<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: as in `is_elevated`.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = Owned(token);

    let mut needed = 0u32;
    // SAFETY: a null buffer with size 0 asks Windows for the size it needs.
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return None;
    }
    // u64 elements keep the buffer aligned for TOKEN_USER.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: the buffer is at least `needed` bytes.
    let ok = unsafe {
        GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), needed, &mut needed)
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: Windows filled the buffer with a TOKEN_USER whose SID points into the same buffer.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: `sid` is valid for the call; `text` receives a LocalAlloc'd string.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return None;
    }
    // SAFETY: Windows returned a NUL-terminated UTF-16 string, freed with LocalFree below.
    let result = unsafe {
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    // SAFETY: `text` came from ConvertSidToStringSidW, which documents LocalFree.
    unsafe { LocalFree(text.cast()) };
    Some(result)
}

/// Run `file` with `arguments` using the "runas" verb (one UAC prompt), hidden, and wait for it.
/// Returns its exit code, or `None` if the prompt was declined or it could not start.
#[allow(unsafe_code)]
pub fn run_elevated_and_wait(file: &str, arguments: &str, timeout: Duration) -> Option<u32> {
    let verb = wide("runas");
    let file = wide(file);
    let arguments = wide(arguments);
    // SAFETY: a zeroed SHELLEXECUTEINFOW is a valid starting point; the fields set below are the
    // ones ShellExecuteExW reads, and the buffers outlive the call.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = arguments.as_ptr();
    info.nShow = SW_HIDE;
    // SAFETY: `info` is fully initialised above.
    if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess.is_null() {
        return None;
    }
    let process = Owned(info.hProcess);
    // SAFETY: `process` is a valid process handle we own.
    let waited = unsafe { WaitForSingleObject(process.0, timeout.as_millis().min(u32::MAX as u128 - 1) as u32) };
    if waited != WAIT_OBJECT_0 {
        return None;
    }
    let mut code = 0u32;
    // SAFETY: valid process handle and out-pointer.
    (unsafe { GetExitCodeProcess(process.0, &mut code) } != 0).then_some(code)
}

// ---- single copy -----------------------------------------------------------------------------

/// The lock that keeps a second OASIS from running, plus the event a second launch uses to ask
/// the first to show its window. Both are open to the signed-in user only: without that rule an
/// OASIS running as administrator would make objects a normal-level OASIS cannot even see.
pub struct SingleInstance {
    mutex: usize,
    show: usize,
    stop: Arc<AtomicBool>,
    watcher: Option<JoinHandle<()>>,
}

/// A security descriptor granting the signed-in user full control, freed on drop.
struct UserOnlyAccess {
    descriptor: *mut core::ffi::c_void,
}

impl UserOnlyAccess {
    #[allow(unsafe_code)]
    fn new() -> Option<UserOnlyAccess> {
        let sddl = wide(&format!("D:(A;;GA;;;{})", current_user_sid()?));
        let mut descriptor = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; `descriptor` receives a LocalAlloc'd descriptor.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        (ok != 0).then_some(UserOnlyAccess { descriptor })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor,
            bInheritHandle: 0,
        }
    }
}

impl Drop for UserOnlyAccess {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW (LocalAlloc).
        unsafe { LocalFree(self.descriptor) };
    }
}

impl SingleInstance {
    /// Take the lock. `None` means another OASIS holds it (or its lock is not ours to open,
    /// which also means one is running).
    #[allow(unsafe_code)]
    pub fn try_own(mutex_name: &str, show_event_name: &str) -> Option<SingleInstance> {
        let access = UserOnlyAccess::new()?;
        let attributes = access.attributes();
        let mutex_name = wide(mutex_name);
        // SAFETY: the name is NUL-terminated and the attributes outlive the call.
        let mutex = unsafe { CreateMutexW(&attributes, 1, mutex_name.as_ptr()) };
        // SAFETY: reads the calling thread's last error, set by the call above.
        let error = unsafe { GetLastError() };
        if mutex.is_null() {
            return None;
        }
        if error == ERROR_ALREADY_EXISTS {
            drop(Owned(mutex));
            return None;
        }
        let event_name = wide(show_event_name);
        // SAFETY: as above; auto-reset, initially not set.
        let show = unsafe { CreateEventW(&attributes, 0, 0, event_name.as_ptr()) };
        if show.is_null() {
            // SAFETY: we own the mutex we just created.
            unsafe { ReleaseMutex(mutex) };
            drop(Owned(mutex));
            return None;
        }
        Some(SingleInstance {
            mutex: mutex as usize,
            show: show as usize,
            stop: Arc::new(AtomicBool::new(false)),
            watcher: None,
        })
    }

    /// Whether an OASIS holds the lock right now. Takes nothing.
    #[allow(unsafe_code)]
    pub fn is_held(mutex_name: &str) -> bool {
        let name = wide(mutex_name);
        // SAFETY: NUL-terminated name.
        let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
        if !handle.is_null() {
            drop(Owned(handle));
            return true;
        }
        // It exists but is not ours to open: an instance holds it.
        // SAFETY: reads the last error of the call above.
        unsafe { GetLastError() == ERROR_ACCESS_DENIED }
    }

    /// Ask the running OASIS to show its window. Retries for a second, since the first copy may
    /// still be creating its event.
    #[allow(unsafe_code)]
    pub fn signal_show(show_event_name: &str) -> bool {
        let name = wide(show_event_name);
        for _ in 0..40 {
            // SAFETY: NUL-terminated name.
            let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
            if !handle.is_null() {
                let handle = Owned(handle);
                // SAFETY: valid event handle with modify rights.
                return unsafe { SetEvent(handle.0) } != 0;
            }
            // SAFETY: reads the last error of the call above.
            if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
                return false;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }

    /// Call `on_show` (on a background thread) each time a second launch asks for the window.
    #[allow(unsafe_code)]
    pub fn watch_show_requests(&mut self, on_show: impl Fn() + Send + 'static) {
        if self.watcher.is_some() {
            return;
        }
        let stop = Arc::clone(&self.stop);
        let show = self.show;
        self.watcher = thread::Builder::new()
            .name("oasis-single-instance".into())
            .spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    // SAFETY: the event handle stays open until `drop` has joined this thread.
                    let waited = unsafe { WaitForSingleObject(show as HANDLE, 250) };
                    if waited == WAIT_OBJECT_0 && !stop.load(Ordering::SeqCst) {
                        on_show();
                    }
                }
            })
            .ok();
    }
}

impl Drop for SingleInstance {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // SAFETY: both handles are ours and still open; the watcher is joined before the event
        // is closed.
        unsafe { SetEvent(self.show as HANDLE) };
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        // SAFETY: as above; the mutex is released on the thread that took it (the owner's).
        unsafe { ReleaseMutex(self.mutex as HANDLE) };
        drop(Owned(self.show as HANDLE));
        drop(Owned(self.mutex as HANDLE));
    }
}

// ---- login startup (Run key) ------------------------------------------------------------------

/// The command stored under `name` in the current user's Run key, if any.
#[allow(unsafe_code)]
pub fn run_key_value(name: &str) -> Option<String> {
    registry_string(HKEY_CURRENT_USER, RUN_KEY, name)
}

/// A text value (`name` empty means the key's default value) under `path` in `hive`.
#[allow(unsafe_code)]
fn registry_string(hive: HKEY, path: &str, name: &str) -> Option<String> {
    let key_path = wide(path);
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: NUL-terminated path, valid out-pointer.
    if unsafe { RegOpenKeyExW(hive, key_path.as_ptr(), 0, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return None;
    }
    let name = wide(name);
    let mut size = 0u32;
    // SAFETY: a null data pointer asks for the size.
    let probe = unsafe {
        RegQueryValueExW(key, name.as_ptr(), ptr::null(), ptr::null_mut(), ptr::null_mut(), &mut size)
    };
    let mut value = None;
    if probe == ERROR_SUCCESS && size > 0 {
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: the buffer holds `size` bytes.
        let read = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if read == ERROR_SUCCESS {
            value = Some(from_wide(&buffer));
        }
    }
    // SAFETY: `key` was opened above.
    unsafe { RegCloseKey(key) };
    value.filter(|v| !v.trim().is_empty())
}

/// Set (`Some`) or remove (`None`) the value `name` in the current user's Run key.
#[allow(unsafe_code)]
pub fn set_run_key_value(name: &str, command: Option<&str>) -> bool {
    let key_path = wide(RUN_KEY);
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: NUL-terminated path, valid out-pointers; creates the key if it is missing.
    let opened = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    };
    if opened != ERROR_SUCCESS {
        return false;
    }
    let name = wide(name);
    let status = match command {
        Some(command) => {
            let data = wide(command);
            // SAFETY: `data` is a NUL-terminated UTF-16 string of the stated byte length.
            unsafe {
                RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
            }
        }
        // SAFETY: valid key and name.
        None => match unsafe { RegDeleteValueW(key, name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            other => other,
        },
    };
    // SAFETY: `key` was opened above.
    unsafe { RegCloseKey(key) };
    status == ERROR_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(what: &str) -> String {
        format!(r"Local\OasisTest.{what}.{}", std::process::id())
    }

    #[test]
    fn the_user_has_a_sid() {
        let sid = current_user_sid().expect("a signed-in user has a SID");
        assert!(sid.starts_with("S-1-"), "{sid}");
    }

    #[test]
    fn elevation_can_be_read() {
        let _ = is_elevated(); // either answer is fine; it must not crash
    }

    #[test]
    fn a_second_copy_cannot_take_the_lock_until_the_first_lets_go() {
        let (lock, show) = (unique("lock"), unique("show"));
        assert!(!SingleInstance::is_held(&lock));
        let first = SingleInstance::try_own(&lock, &show).expect("the first copy gets it");
        assert!(SingleInstance::is_held(&lock));
        assert!(SingleInstance::try_own(&lock, &show).is_none(), "the second copy is refused");
        drop(first);
        assert!(!SingleInstance::is_held(&lock));
        assert!(SingleInstance::try_own(&lock, &show).is_some(), "free again after the first exits");
    }

    #[test]
    fn a_second_launch_can_ask_the_first_to_show_its_window() {
        let (lock, show) = (unique("lock2"), unique("show2"));
        let mut first = SingleInstance::try_own(&lock, &show).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        first.watch_show_requests(move || {
            let _ = tx.send(());
        });
        assert!(SingleInstance::signal_show(&show));
        rx.recv_timeout(Duration::from_secs(2)).expect("the first copy was told to show");
    }

    #[test]
    fn signalling_with_nobody_running_gives_up() {
        assert!(!SingleInstance::signal_show(&unique("nobody")));
    }

    #[test]
    fn the_run_key_value_can_be_written_read_and_removed() {
        let name = format!("OasisTest{}", std::process::id());
        assert_eq!(run_key_value(&name), None);
        assert!(set_run_key_value(&name, Some(r#""C:\x y\oasis.exe" --tray"#)));
        assert_eq!(run_key_value(&name).as_deref(), Some(r#""C:\x y\oasis.exe" --tray"#));
        assert!(set_run_key_value(&name, None));
        assert_eq!(run_key_value(&name), None);
        assert!(set_run_key_value(&name, None), "removing what is not there is fine");
    }
}

// ---- registered programs (App Paths) ------------------------------------------------------------

const APP_PATHS_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";

/// Read a key's default (unnamed) value as text.
#[allow(unsafe_code)]
fn default_value_of(key: HKEY) -> Option<String> {
    let mut size = 0u32;
    // SAFETY: a null data pointer asks for the size; the name is null (the default value).
    let probe = unsafe { RegQueryValueExW(key, ptr::null(), ptr::null(), ptr::null_mut(), ptr::null_mut(), &mut size) };
    if probe != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer holds `size` bytes.
    let read = unsafe {
        RegQueryValueExW(key, ptr::null(), ptr::null(), ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size)
    };
    (read == ERROR_SUCCESS).then(|| from_wide(&buffer)).filter(|v| !v.trim().is_empty())
}

/// The programs Windows knows by name: `(registered name, path)` from `App Paths`, machine-wide and
/// per-user. The name is the key's name (`chrome.exe`); the path is its default value, with
/// quotes removed.
#[allow(unsafe_code)]
pub fn app_paths() -> Vec<(String, String)> {
    use windows_sys::Win32::System::Registry::{RegEnumKeyExW, HKEY_LOCAL_MACHINE};

    let mut found = Vec::new();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let path = wide(APP_PATHS_KEY);
        let mut base: HKEY = ptr::null_mut();
        // SAFETY: NUL-terminated path, valid out-pointer.
        if unsafe { RegOpenKeyExW(hive, path.as_ptr(), 0, KEY_READ, &mut base) } != ERROR_SUCCESS {
            continue;
        }
        for index in 0u32.. {
            let mut name = [0u16; 256];
            let mut length = name.len() as u32;
            // SAFETY: `name` is writable for `length` units, which Windows updates.
            let status = unsafe {
                RegEnumKeyExW(base, index, name.as_mut_ptr(), &mut length, ptr::null(), ptr::null_mut(), ptr::null_mut(), ptr::null_mut())
            };
            if status != ERROR_SUCCESS {
                break;
            }
            let sub_name = String::from_utf16_lossy(&name[..length as usize]);
            let sub_path = wide(&format!("{APP_PATHS_KEY}\\{sub_name}"));
            let mut sub: HKEY = ptr::null_mut();
            // SAFETY: as above.
            if unsafe { RegOpenKeyExW(hive, sub_path.as_ptr(), 0, KEY_READ, &mut sub) } == ERROR_SUCCESS {
                if let Some(value) = default_value_of(sub) {
                    found.push((sub_name, value.trim().trim_matches('"').to_string()));
                }
                // SAFETY: `sub` was opened just above.
                unsafe { RegCloseKey(sub) };
            }
        }
        // SAFETY: `base` was opened above.
        unsafe { RegCloseKey(base) };
    }
    found
}

#[cfg(test)]
mod app_paths_tests {
    use super::*;

    #[test]
    fn the_registered_programs_can_be_listed() {
        let programs = app_paths();
        // Every Windows install registers some (for example the shell's own).
        assert!(!programs.is_empty(), "no App Paths entries at all");
        assert!(programs.iter().all(|(name, path)| !name.is_empty() && !path.is_empty()));
    }
}

/// Local time minus UTC right now, in seconds (daylight saving included).
#[allow(unsafe_code)]
pub fn utc_offset_seconds() -> i32 {
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    // SAFETY: an all-zero TIME_ZONE_INFORMATION is valid plain data, and the call only fills it in.
    let mut zone: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: valid out-pointer.
    let id = unsafe { GetTimeZoneInformation(&mut zone) };
    // Windows stores the bias as minutes to ADD to local time to get UTC.
    let extra = match id {
        1 => zone.StandardBias,
        2 => zone.DaylightBias,
        _ => 0,
    };
    -(zone.Bias + extra) * 60
}

/// The Windows "exclamation" sound, once.
#[allow(unsafe_code)]
pub fn beep() {
    use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONEXCLAMATION;
    // SAFETY: plain call with a constant sound id.
    unsafe { MessageBeep(MB_ICONEXCLAMATION) };
}

/// The program Windows opens https links with, from the user's own choice (its full path), if it
/// can be worked out. Port of `ResolveBrowser("")` in `QuickLaunchLauncher.cs`.
pub fn default_browser_exe() -> Option<String> {
    use windows_sys::Win32::System::Registry::HKEY_CLASSES_ROOT;
    let choice = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice";
    let prog_id = registry_string(HKEY_CURRENT_USER, choice, "ProgId")?;
    let command = registry_string(HKEY_CLASSES_ROOT, &format!(r"{prog_id}\shell\open\command"), "")?;
    executable_of(&command).filter(|exe| std::path::Path::new(exe).is_file())
}

/// The program in a registry "open" command such as `"C:\...\chrome.exe" --single-argument %1`.
pub fn executable_of(command: &str) -> Option<String> {
    let text = command.trim();
    if let Some(rest) = text.strip_prefix('"') {
        return rest.find('"').filter(|close| *close > 0).map(|close| rest[..close].to_string());
    }
    text.to_ascii_lowercase().find(".exe").map(|at| text[..at + 4].to_string())
}

#[cfg(test)]
mod browser_tests {
    use super::*;

    #[test]
    fn the_program_is_read_out_of_an_open_command() {
        assert_eq!(executable_of(r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument %1"#).as_deref(), Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe"));
        assert_eq!(executable_of(r"C:\x\firefox.exe -osint -url %1").as_deref(), Some(r"C:\x\firefox.exe"));
        assert_eq!(executable_of(r#""""#), None);
        assert_eq!(executable_of("nothing here"), None);
    }
}

/// Plays a WAV file held in memory and returns at once. A sound already playing is cut off, so
/// pressing a speaker button again never queues up. The cue goes to one worker thread: the caller
/// (the engine loop) never waits on the sound device, and only the latest cue is played. The worker
/// pauses 15 ms after cutting a sound off, because `PlaySound` often drops a sound started in the
/// same instant as the stop (the start cue that sometimes stayed silent). A failed start is retried
/// once. `wav` must stay valid while it plays.
pub fn play_wav(wav: &'static [u8]) {
    use std::sync::{mpsc, OnceLock};
    static WORKER: OnceLock<mpsc::Sender<&'static [u8]>> = OnceLock::new();
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<&'static [u8]>();
        let _ = std::thread::Builder::new().name("cue".into()).spawn(move || {
            while let Ok(mut wav) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    wav = newer;
                }
                start_wav(wav);
            }
        });
        tx
    });
    let _ = tx.send(wav);
}

#[allow(unsafe_code)]
fn start_wav(wav: &'static [u8]) {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    let play = || {
        // SAFETY: `wav` is 'static, so it outlives the asynchronous playback.
        unsafe { PlaySoundW(wav.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT) != 0 }
    };
    // SAFETY: a null sound name stops playback.
    unsafe { PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
    std::thread::sleep(std::time::Duration::from_millis(15));
    if !play() {
        std::thread::sleep(std::time::Duration::from_millis(40));
        play();
    }
}
