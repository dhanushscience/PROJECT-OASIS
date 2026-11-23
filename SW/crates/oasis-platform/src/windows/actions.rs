//! Not ported yet, marked in `docs/WINDOWS_BACKEND.md`: finding an app by fuzzy name
//! (`AppNameResolver`), Store-app discovery (`WindowsAppDiscovery`), `KnownAppResolver`'s full
//! name table, bringing a freshly launched app to the front, starting programs at normal rights
//! when OASIS runs as administrator, the clipboard for `PasteText`, and `JoinMeeting`.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use oasis_core::actions::{app_resolver, ActionBackend, MouseButton, MouseClick};
use oasis_core::shortcut::Chord;
use oasis_win::{focus, input, shell, window::WindowId};

use super::app_discovery;
use super::input::WindowsInput;
use crate::ports::InputInjector;
use crate::security_log::SecurityLog;

const SCRIPT_TIMEOUT: Duration = Duration::from_secs(60);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// The most a paste waits for the clipboard to show the text (it normally does at once).
const PASTE_READY_LIMIT: Duration = Duration::from_millis(120);
/// How long after Ctrl+V the user's own clipboard text is put back. Apps read the clipboard when
/// they handle the key; slow ones (Electron, remote desktops) take a few hundred milliseconds.
const RESTORE_AFTER: Duration = Duration::from_millis(600);

pub struct WindowsActions {
    input: WindowsInput,
    security: SecurityLog,
    allow_run_script: bool,
    /// For the smart wait: the last step brought an already open window forward, so it is ready.
    already_ready: bool,
    /// The title of the window in front before the step, so a browser tab that loads in the same
    /// window (its title changes) counts as ready.
    before_title: String,
    /// What the last launch asked for (the typed name and its launcher name), so an app that is already
    /// in front, by a friendly name like "vs code", counts as opened at once.
    targets: Vec<String>,
    /// Why the last action failed, for the user (W6); read once by `take_problem`.
    problem: Option<String>,
}

/// How long a Store app may take to show a window before the launch counts as failed.
const STORE_APP_WINDOW: Duration = Duration::from_secs(6);

impl WindowsActions {
    /// A program is running but Windows would not put it in front (W6).
    fn blocked(&mut self, name: &str) -> bool {
        let why = format!(
            "'{name}' is open, but Windows would not bring it forward. It probably runs as administrator: turn on \"Run as administrator\" in Settings > General so OASIS can control it."
        );
        self.security.write(&why);
        self.problem = Some(why);
        false
    }
}

impl WindowsActions {
    /// `allow_run_script` is the "allow RunScript actions" setting.
    pub fn new(security: SecurityLog, allow_run_script: bool) -> WindowsActions {
        WindowsActions {
            input: WindowsInput,
            security,
            allow_run_script,
            already_ready: false,
            before_title: String::new(),
            targets: Vec::new(),
            problem: None,
        }
    }
}

/// The executable that opens a page in a named browser, for the `browser|url` form. Windows finds
/// the installed program through its "App Paths" entry, so no folder is needed.
pub fn browser_executable(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_lowercase().as_str() {
        "chrome" | "google chrome" => "chrome.exe",
        "edge" | "msedge" | "microsoft edge" => "msedge.exe",
        "firefox" | "mozilla firefox" => "firefox.exe",
        "brave" | "brave browser" => "brave.exe",
        "opera" => "opera.exe",
        "vivaldi" => "vivaldi.exe",
        _ => return None,
    })
}

/// Browsers a web page can open in: `(process name, name shown in Settings)`.
pub const BROWSERS: [(&str, &str); 6] = [
    ("chrome", "Google Chrome"),
    ("msedge", "Microsoft Edge"),
    ("firefox", "Firefox"),
    ("brave", "Brave"),
    ("opera", "Opera"),
    ("vivaldi", "Vivaldi"),
];

/// Settings > Preferences > Preferred browser (W14): a [`BROWSERS`] process name, or "" for the
/// system's default. Set by the engine whenever settings load.
static PREFERRED_BROWSER: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn set_preferred_browser(name: &str) {
    *PREFERRED_BROWSER.lock().unwrap_or_else(|e| e.into_inner()) = name.trim().to_lowercase();
}

/// The [`BROWSERS`] installed on this PC (registered under App Paths), in that order.
pub fn installed_browsers() -> Vec<String> {
    let registered = oasis_win::system::app_paths();
    BROWSERS
        .iter()
        .filter(|(name, _)| {
            registered.iter().any(|(key, path)| key.eq_ignore_ascii_case(&format!("{name}.exe")) && Path::new(path).is_file())
        })
        .map(|(name, _)| name.to_string())
        .collect()
}

/// W13: where a web page opens. The browser used last, if one is open (it takes the page as a new
/// tab in its last window); else the preferred browser, if installed; else `None`: the system's
/// default browser.
fn page_browser() -> Option<String> {
    let is_browser = |p: &str| BROWSERS.iter().any(|(name, _)| p.eq_ignore_ascii_case(name));
    let front = focus::main_windows_where(is_browser).into_iter().next();
    if let Some(process) = front.and_then(oasis_win::window::window_process_id).and_then(oasis_win::window::process_name) {
        return Some(format!("{}.exe", process.to_lowercase()));
    }
    let preferred = PREFERRED_BROWSER.lock().unwrap_or_else(|e| e.into_inner()).clone();
    installed_browsers().contains(&preferred).then(|| format!("{preferred}.exe"))
}

fn is_web_page(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// How a `LaunchApp` query is handled.
#[derive(Debug, PartialEq, Eq)]
enum Launch<'a> {
    /// `shell:` locations are opened by Explorer.
    Shell(&'a str),
    /// A program or document: focus it if its process already has a window, else start it.
    Program { query: &'a str, process: Option<String> },
}

fn plan_launch(query: &str) -> Launch<'_> {
    if query.len() >= 6 && query[..6].eq_ignore_ascii_case("shell:") {
        return Launch::Shell(query);
    }
    // explorer.exe always runs (it is the taskbar), so "bring it forward" found a hidden window and
    // nothing appeared. File Explorer always opens a new window, as its taskbar icon does.
    if oasis_core::voice::command::apps::canonical_app(&app_resolver::normalize_token(query)) == Some("explorer") {
        return Launch::Shell("");
    }
    // "C:\...\chrome.exe" and "chrome.exe" name a process; a document or URL-like text does not.
    if Path::new(query).is_dir() {
        return Launch::Shell(query);
    }
    let process = Path::new(query)
        .file_stem()
        .filter(|_| query.to_ascii_lowercase().ends_with(".exe"))
        .map(|s| s.to_string_lossy().into_owned());
    Launch::Program { query, process }
}

/// Do `names` (typed names or program paths) match the program of a running window?
pub(super) fn runs_as(names: &[String], process: &str) -> bool {
    let process = app_resolver::squash(process);
    names.iter().any(|n| process.len() >= 3 && app_resolver::squash(&app_resolver::normalize_token(n)) == process)
}

/// Every name a spoken app may go by: as said, by its launcher name ("vs code" is "Visual Studio Code"),
/// and the program it resolves to (`Code.exe`). For finding its windows.
pub(super) fn program_names(query: &str) -> Vec<String> {
    let lookup = oasis_core::voice::command::apps::launch_name(query).unwrap_or(query);
    let mut names = vec![query.to_string(), lookup.to_string()];
    if let Some(resolved) = app_discovery::resolve(lookup) {
        names.extend(match &resolved {
            app_resolver::Resolved::Shortcut(lnk) => shell::shortcut_target(lnk),
            app_resolver::Resolved::Exe(path) => Some(path.clone()),
            app_resolver::Resolved::Store(id) => app_discovery::expand_start_app_path(id),
        });
    }
    names
}

/// What bringing a program forward came to (W6).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Forward {
    /// No window of it is open.
    NotRunning,
    /// It is in front now (or was).
    Front,
    /// It is running, but Windows would not put it in front: usually it runs as administrator.
    Blocked,
}

/// Bring every window of a running program named by `names` forward, restored if minimised, the
/// one used last on top. Does nothing when one of them is already in front.
fn bring_forward<'a>(names: impl Iterator<Item = &'a String>) -> Forward {
    let names: Vec<String> = names.cloned().collect();
    let windows: Vec<WindowId> = focus::main_windows_where(|p| runs_as(&names, p));
    if windows.is_empty() {
        return Forward::NotRunning;
    }
    if oasis_win::window::foreground_window().is_some_and(|f| windows.contains(&f)) {
        return Forward::Front;
    }
    // Back to front, so the first (front-most) ends up on top.
    if windows.into_iter().rev().fold(false, |any, w| focus::activate(w) | any) {
        Forward::Front
    } else {
        Forward::Blocked
    }
}

/// One timing line in the security log: how long the request took to reach the point named (step 0
/// of `docs/APP_WINDOWS.md`). No window titles, no text, only the app name asked for.
fn security_note(security: &SecurityLog, app: &str, what: &str, since: Instant) {
    security.write(&format!("Launch timing: {app} {what} after {} ms.", since.elapsed().as_millis()));
}

/// W7: a program just started opens its window a moment later, often behind the one in use (Windows
/// refuses focus to a new process). Wait for it, off the caller's thread, and bring it forward.
fn front_when_it_opens(names: Vec<String>, security: SecurityLog, since: Instant) {
    std::thread::spawn(move || {
        // Every 25 ms for up to 8 s: the window comes forward the moment it exists.
        for _ in 0..320 {
            if bring_forward(names.iter()) != Forward::NotRunning {
                security.write(&format!("Launch timing: {} window in front {} ms after the request.", names[0], since.elapsed().as_millis()));
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        // Not an error for the user (a tray app has no window), but the log says what happened.
        security.write(&format!("Launch: {} was started, but no window of it appeared within 8 seconds.", names.first().map_or("the app", String::as_str)));
    });
}

/// The File Explorer windows open now, front to back (the desktop's "Program Manager" among them,
/// which is never new, so never brought forward).
fn explorer_windows() -> Vec<WindowId> {
    focus::main_windows_where(|p| runs_as(&["explorer".to_string()], p))
}

/// A place or folder opened by voice can appear behind the window in use (Windows refuses focus to
/// Explorer here), so "close this" then meant the desktop. Wait for the new Explorer window and
/// bring it forward, off the caller's thread.
fn front_new_explorer(before: Vec<WindowId>, security: SecurityLog, since: Instant) {
    std::thread::spawn(move || {
        // Every 25 ms for up to 2 s.
        for _ in 0..80 {
            std::thread::sleep(Duration::from_millis(25));
            if let Some(new) = explorer_windows().into_iter().find(|w| !before.contains(w)) {
                focus::activate(new);
                security.write(&format!("Launch timing: Explorer window in front {} ms after the request.", since.elapsed().as_millis()));
                return;
            }
        }
        // shortcut: Windows 11 may open the folder as a tab of a window already open, so there is no
        // new window; the front-most Explorer window is then the one that got the tab. Upgrade to
        // matching the tab's folder (IShellWindows) if this picks the wrong window in testing.
        if let Some(top) = explorer_windows().into_iter().find(|w| !oasis_win::window::is_shell_window(*w)) {
            focus::activate(top);
        }
    });
}

/// Does the typed name (a friendly name, a program name, a path) name this window? Compared without
/// spaces, case or `.exe`, against the program's name and the window title.
fn names_window(target: &str, process: &str, title: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    let stem = Path::new(target)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (t, p, w) = (norm(&stem), norm(process), norm(title));
    t.len() >= 3 && (p.contains(&t) || (p.len() >= 3 && t.contains(&p)) || w.contains(&t))
}

impl ActionBackend for WindowsActions {
    fn send_shortcut(&mut self, chord: &Chord) -> bool {
        self.input.send_shortcut(chord).is_ok()
    }

    fn open_url(&mut self, url: &str) -> bool {
        // A web page goes to the browser in use as a new tab (W13); anything else (mailto:,
        // ms-settings:) to whatever Windows opens it with.
        if is_web_page(url) {
            if let Some(exe) = page_browser() {
                if shell::open(&exe, Some(&format!("\"{url}\""))).is_ok() {
                    return true;
                }
            }
        }
        shell::open(url, None).is_ok()
    }

    fn launch_browser_url(&mut self, browser: &str, url: &str) -> bool {
        browser_executable(browser).is_some_and(|exe| shell::open(exe, Some(url)).is_ok())
    }

    fn launch_app(&mut self, query: &str) -> bool {
        self.problem = None;
        let began = Instant::now();
        // "browser" (said as "my browser", "the internet"): the one in use, else the preferred one,
        // else whichever opens web links (W14).
        let default_browser = (query == "browser").then(|| page_browser().or_else(oasis_win::shell::default_browser)).flatten();
        let query = default_browser.as_deref().unwrap_or(query);
        self.targets = vec![query.to_string()];
        if let Some(launcher) = oasis_core::voice::command::apps::launch_name(query) {
            self.targets.push(launcher.to_string());
        }
        match plan_launch(query) {
            Launch::Shell(location) => {
                // A folder opens in Explorer under the folder's own name: any new titled window counts.
                self.targets.clear();
                let before = explorer_windows();
                let opened = shell::open("explorer.exe", Some(location).filter(|l| !l.is_empty())).is_ok();
                if opened {
                    front_new_explorer(before, self.security.clone(), began);
                }
                opened
            }
            Launch::Program { query, process } => {
                // A friendly name ("spotify", "vs code") is looked up on this machine: Start Menu,
                // Store apps, install folders, the registry. A spoken name goes by its launcher
                // name first ("vs code" is "Visual Studio Code").
                let lookup = oasis_core::voice::command::apps::launch_name(query).unwrap_or(query);
                // W1: a running copy comes forward (restored if minimised) instead of a second start.
                let mut names: Vec<String> = [Some(query.to_string()), Some(lookup.to_string()), process].into_iter().flatten().collect();
                match bring_forward(names.iter()) {
                    Forward::Front => {
                        self.already_ready = true;
                        security_note(&self.security, &names[0], "already open, brought forward", began);
                        return true;
                    }
                    Forward::Blocked => return self.blocked(&names[0]),
                    Forward::NotRunning => {}
                }
                match app_discovery::resolve(lookup) {
                    Some(resolved) => {
                        // The program the name resolves to ("vs code" -> Code.exe) may be running too.
                        let exe = match &resolved {
                            app_resolver::Resolved::Shortcut(lnk) => shell::shortcut_target(lnk),
                            app_resolver::Resolved::Exe(path) => Some(path.clone()),
                            app_resolver::Resolved::Store(id) => app_discovery::expand_start_app_path(id),
                        };
                        match exe.as_ref().map_or(Forward::NotRunning, |e| bring_forward(std::iter::once(e))) {
                            Forward::Front => {
                                self.already_ready = true;
                                return true;
                            }
                            Forward::Blocked => return self.blocked(&names[0]),
                            Forward::NotRunning => {}
                        }
                        names.extend(exe);
                        match app_discovery::start_command(&resolved) {
                            app_discovery::StartCommand::Open(target) => {
                                let started = shell::open(&target, None).is_ok();
                                if started {
                                    security_note(&self.security, &names[0], "started", began);
                                    front_when_it_opens(names, self.security.clone(), began);
                                } else {
                                    self.problem = Some(format!("Windows could not start '{}'.", names[0]));
                                }
                                started
                            }
                            // A Store app: Explorer answers "ok" whether or not the app starts, so
                            // the only proof is its window (W6).
                            app_discovery::StartCommand::Explorer(location) => {
                                if shell::open("explorer.exe", Some(&location)).is_err() {
                                    self.problem = Some(format!("Windows could not start '{}'.", names[0]));
                                    return false;
                                }
                                let started = Instant::now();
                                while started.elapsed() < STORE_APP_WINDOW {
                                    if bring_forward(names.iter()) != Forward::NotRunning {
                                        self.already_ready = true;
                                        return true;
                                    }
                                    std::thread::sleep(Duration::from_millis(25));
                                }
                                self.problem = Some(format!(
                                    "'{}' did not open: Windows started it but no window appeared within {} seconds.",
                                    names[0],
                                    STORE_APP_WINDOW.as_secs()
                                ));
                                false
                            }
                        }
                    }
                    None => shell::open(query, None).is_ok(),
                }
            }
        }
    }

    fn window_action(&mut self, kind: &str, value: &str) -> bool {
        self.problem = None;
        match super::window_commands::run(kind, value) {
            Ok(()) => true,
            Err(why) => {
                self.problem = Some(why);
                false
            }
        }
    }

    fn take_problem(&mut self) -> Option<String> {
        self.problem.take()
    }

    fn close_app(&mut self, process: &str) -> bool {
        oasis_win::window::close_program(process) > 0
    }

    fn front_window(&mut self) -> Option<isize> {
        self.already_ready = false;
        // Set again by a LaunchApp step; an OpenUrl step or a folder has no app to wait for.
        self.targets.clear();
        let front = oasis_win::window::foreground_window();
        self.before_title = front
            .map(oasis_win::window::window_title)
            .unwrap_or_default();
        front.map(|w| w.0)
    }

    /// Ready when a different window is in front, or the same window has a new title (a page that
    /// loaded in a browser already in front). Then a short pause, so it takes the next keys.
    fn wait_until_ready(&mut self, before: Option<isize>, limit: Duration) -> bool {
        if std::mem::take(&mut self.already_ready) {
            return true;
        }
        let started = Instant::now();
        while started.elapsed() < limit {
            if let Some(front) = oasis_win::window::foreground_window() {
                // The app asked for is already the window in front (named however it was typed).
                let in_front = oasis_win::window::foreground_window().is_some_and(|w| {
                    let title = oasis_win::window::window_title(w);
                    let process = oasis_win::window::window_process_id(w)
                        .and_then(oasis_win::window::process_name)
                        .unwrap_or_default();
                    self.targets
                        .iter()
                        .any(|t| names_window(t, &process, &title))
                });
                // The app asked for is in front, or (for a page or a document, which has no app name
                // to look for) another window with a title is. A starting program shows untitled
                // windows first; keys sent to one of those are lost.
                let title = oasis_win::window::window_title(front);
                let other_window = (Some(front.0) != before || title != self.before_title) && !title.trim().is_empty();
                // An app whose window cannot be told by its name still counts after a while.
                let app_unmatched = started.elapsed() >= Duration::from_secs(3);
                if in_front || (other_window && (self.targets.is_empty() || app_unmatched)) {
                    // Ready for the next step's keys when the program is (W5); 400 ms is only the limit.
                    focus::wait_input_idle(front, Duration::from_millis(400));
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn run_script(&mut self, script: &str) {
        let (script, security) = (script.trim().to_string(), self.security.clone());
        std::thread::spawn(move || run_powershell(&script, "RunScript", &security));
    }

    fn run_autohotkey(&mut self, script: &str) -> bool {
        let Some(exe) = autohotkey_exe() else {
            self.security
                .write("AutoHotkey step skipped: AutoHotkey v2 is not installed for all users.");
            return false;
        };
        let (script, security) = (script.to_string(), self.security.clone());
        // `*` reads the script from standard input (never a temp file, as for PowerShell);
        // /CP65001 reads it as UTF-8. Errors still show AutoHotkey's own dialog, which says the line.
        std::thread::spawn(move || {
            run_hidden(&exe, &["/CP65001", "*"], &script, "AutoHotkey", &security)
        });
        true
    }

    fn mouse_click(&mut self, click: MouseClick) -> bool {
        let button = match click.button {
            MouseButton::Left => input::MouseButton::Left,
            MouseButton::Right => input::MouseButton::Right,
            MouseButton::Middle => input::MouseButton::Middle,
        };
        input::click_at(click.x, click.y, button).is_ok()
    }

    /// Pastes the text: puts it on the clipboard, then sends Ctrl+V. The whole text lands at once,
    /// however long it is. If the clipboard cannot be used, the text is typed instead so nothing
    /// that was said is lost. Text the user had copied is put back afterwards (step 7).
    fn paste_text(&mut self, text: &str) -> bool {
        use oasis_win::clipboard;
        let previous = clipboard::get_text();
        if clipboard::set_text(text).is_err() {
            return self.input.type_text(text).is_ok();
        }
        let ours = clipboard::sequence();
        // Setting the clipboard is immediate; the old fixed 120 ms wait is only a limit now.
        let expected = String::from_utf16_lossy(&clipboard::clipboard_units(text));
        let started = Instant::now();
        while clipboard::get_text().is_none_or(|t| expected.trim_end_matches('\0') != t) && started.elapsed() < PASTE_READY_LIMIT {
            std::thread::sleep(Duration::from_millis(10));
        }
        let pasted = Chord::parse("Ctrl+V").is_ok_and(|chord| self.input.send_shortcut(&chord).is_ok());
        if let Some(previous) = previous.filter(|_| pasted) {
            // The app reads the clipboard when it handles Ctrl+V, a moment later; only then is the
            // user's text put back, and only if nothing else was copied meanwhile.
            std::thread::spawn(move || {
                std::thread::sleep(RESTORE_AFTER);
                if clipboard::sequence() == ours {
                    let _ = clipboard::set_text(&previous);
                }
            });
        }
        pasted
    }

    fn join_meeting(&mut self) -> bool {
        false
    }

    fn system(&mut self, command: &str) -> bool {
        match command {
            "lock" => shell::open("rundll32.exe", Some("user32.dll,LockWorkStation")).is_ok(),
            "theme-dark" | "theme-light" => {
                let light = if command == "theme-light" { "1" } else { "0" };
                let (script, security) = (THEME_SCRIPT.replace("LIGHT", light), self.security.clone());
                std::thread::spawn(move || run_powershell(&script, "Theme change", &security));
                true
            }
            "radio:wifi:on" | "radio:wifi:off" | "radio:bluetooth:on" | "radio:bluetooth:off" | "radio:airplane:on" | "radio:airplane:off" => {
                let mut parts = command.split(':').skip(1);
                let (device, on) = (parts.next().unwrap_or(""), parts.next() == Some("on"));
                // Airplane mode "on" means every radio off.
                let (kinds, state) = match (device, on) {
                    ("wifi", _) => ("'WiFi'", on),
                    ("bluetooth", _) => ("'Bluetooth'", on),
                    _ => ("'WiFi','Bluetooth','MobileBroadband','FM'", !on),
                };
                let script = RADIO_SCRIPT.replace("KINDS", kinds).replace("STATE", if state { "On" } else { "Off" });
                let security = self.security.clone();
                std::thread::spawn(move || run_powershell(&script, "Radio change", &security));
                true
            }
            "chime:1" | "chime:4" => {
                let beeps = if command == "chime:1" { 1 } else { 4 };
                std::thread::spawn(move || {
                    for _ in 0..beeps {
                        oasis_win::system::beep();
                        std::thread::sleep(Duration::from_millis(900));
                    }
                });
                true
            }
            c if c.strip_prefix("sound:").and_then(oasis_core::actions::sounds::wav_cached).is_some() => {
                if let Some(wav) = oasis_core::actions::sounds::wav_cached(&c["sound:".len()..]) {
                    oasis_win::system::play_wav(wav); // in the background: the next step does not wait
                }
                true
            }
            c if c.strip_prefix("brightness:").is_some_and(is_brightness_arg) => {
                let arg = &c["brightness:".len()..];
                let (script, security) = (BRIGHTNESS_SCRIPT.replace("ARG", arg), self.security.clone());
                std::thread::spawn(move || run_powershell(&script, "Brightness change", &security));
                true
            }
            c if c.strip_prefix("volume:").is_some_and(is_brightness_arg) => oasis_win::audio::set_volume(&c["volume:".len()..]),
            _ => false,
        }
    }

    fn pause(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn run_script_allowed(&self) -> bool {
        self.allow_run_script
    }

    fn security_log(&mut self, message: &str) {
        self.security.write(message);
    }
}

/// Sets the app and system theme (LIGHT is 0 or 1), then tells running programs, which is what
/// makes it switch live.
const THEME_SCRIPT: &str = r#"
$p = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
Set-ItemProperty $p AppsUseLightTheme LIGHT
Set-ItemProperty $p SystemUsesLightTheme LIGHT
Add-Type -Namespace O -Name W -MemberDefinition "[DllImport(""user32.dll"", CharSet=CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, UIntPtr w, string l, uint f, uint t, out UIntPtr r);"
$r = [UIntPtr]::Zero
[O.W]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, "ImmersiveColorSet", 2, 1000, [ref]$r) | Out-Null
"#;

/// Sets the built-in screen's brightness: ARG is a level (`70`) or a change (`+10`, `-10`).
/// External monitors do not answer this call, so on a desktop PC nothing happens.
const BRIGHTNESS_SCRIPT: &str = r#"
$arg = "ARG"
$now = [int](Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness | Select-Object -First 1).CurrentBrightness
$target = if ($arg -match '^[+-]') { $now + [int]$arg } else { [int]$arg }
$target = [Math]::Max(0, [Math]::Min(100, $target))
Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{ Timeout = 1; Brightness = $target } | Out-Null
"#;

/// Switches radios through the Windows radio API (no administrator needed). KINDS and STATE are
/// filled in from a fixed list, never from spoken text.
const RADIO_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, $type) { $t = $asTask.MakeGenericMethod($type).Invoke($null, @($op)); $t.Wait(-1) | Out-Null; $t.Result }
[Windows.Devices.Radios.Radio, Windows.System.Devices, ContentType = WindowsRuntime] | Out-Null
[Windows.Devices.Radios.RadioAccessStatus, Windows.System.Devices, ContentType = WindowsRuntime] | Out-Null
$access = Await ([Windows.Devices.Radios.Radio]::RequestAccessAsync()) ([Windows.Devices.Radios.RadioAccessStatus])
if ($access -ne 'Allowed') { exit 1 }
$radios = Await ([Windows.Devices.Radios.Radio]::GetRadiosAsync()) ([System.Collections.Generic.IReadOnlyList[Windows.Devices.Radios.Radio]])
foreach ($r in $radios) {
    if (@(KINDS) -contains [string]$r.Kind) { Await ($r.SetStateAsync('STATE')) ([Windows.Devices.Radios.RadioAccessStatus]) | Out-Null }
}
"#;

/// Switches the Bluetooth radio on, in the background (the Settings screen's "Turn on Bluetooth").
/// The pad connector keeps retrying, so the OASIS interface is found a few seconds later.
pub fn turn_on_bluetooth(security: &SecurityLog) {
    let script = RADIO_SCRIPT.replace("KINDS", "'Bluetooth'").replace("STATE", "On");
    let security = security.clone();
    std::thread::spawn(move || run_powershell(&script, "Radio change", &security));
}

/// The same check the action runner makes, repeated because this text goes into a script.
fn is_brightness_arg(arg: &str) -> bool {
    let digits = arg.strip_prefix(['+', '-']).unwrap_or(arg);
    (1..=3).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
}

/// AutoHotkey v2, only from the all-users install under Program Files. Changing a file there
/// needs administrator rights, so another program cannot swap in its own "AutoHotkey64.exe";
/// a copy on PATH or in a per-user folder could be, so those are never used.
fn autohotkey_exe() -> Option<std::path::PathBuf> {
    let root = Path::new(&std::env::var_os("ProgramFiles")?).join("AutoHotkey");
    ["v2\\AutoHotkey64.exe", "v2\\AutoHotkey32.exe"]
        .iter()
        .map(|p| root.join(p))
        .find(|p| p.is_file())
}

/// Run `script` hidden, waiting up to a minute.
fn run_powershell(script: &str, what: &str, security: &SecurityLog) {
    run_hidden(
        Path::new("powershell.exe"),
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "-",
        ],
        script,
        what,
        security,
    );
}

/// Run `program` with `script` on its standard input, waiting up to a minute, then stopping it.
///
/// The script goes in on standard input, never through a temp file: when OASIS runs as
/// administrator, another program could swap a temp file between writing and running it.
fn run_hidden(program: &Path, args: &[&str], script: &str, what: &str, security: &SecurityLog) {
    use std::io::Write;
    use std::os::windows::process::CommandExt;

    let result = (|| -> std::io::Result<()> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        // `-Command -` runs the last statement only once a blank line follows it; without one,
        // everything since the previous blank line is silently dropped. Dropping stdin sends EOF.
        let mut stdin = child.stdin.take().expect("stdin is piped");
        stdin.write_all(script.as_bytes())?;
        stdin.write_all(b"\n\n")?;
        drop(stdin);
        let started = Instant::now();
        while child.try_wait()?.is_none() && started.elapsed() < SCRIPT_TIMEOUT {
            std::thread::sleep(Duration::from_millis(100));
        }
        if child.try_wait()?.is_none() {
            // Past the limit: stop it, or it would run on unseen after we stop waiting.
            security.write(&format!("{what} timed out and was stopped."));
            child.kill()?;
            child.wait()?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        security.write(&format!("{what} failed: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::actions::{run_action, run_single, ActionOutcome, ActionStep, ButtonAction};

    #[test]
    fn only_plain_numbers_reach_the_brightness_script() {
        for ok in ["70", "+10", "-5"] {
            assert!(is_brightness_arg(ok), "{ok}");
        }
        for bad in ["", "+", "1000", "7;calc", "$(calc)", "5 6"] {
            assert!(!is_brightness_arg(bad), "{bad}");
        }
        assert!(!BRIGHTNESS_SCRIPT.replace("ARG", "70").contains("ARG"));
    }

    #[test]
    fn the_radio_script_has_its_values_filled_in() {
        let script = RADIO_SCRIPT.replace("KINDS", "'WiFi'").replace("STATE", "Off");
        assert!(!script.contains("KINDS") && !script.contains("STATE"));
        assert!(script.contains("@('WiFi')") && script.contains("SetStateAsync('Off')"));
    }

    #[test]
    fn the_theme_script_has_its_value_filled_in_and_the_right_key() {
        let script = THEME_SCRIPT.replace("LIGHT", "0");
        assert!(!script.contains("LIGHT"));
        assert!(script.contains(r"Themes\Personalize"));
        assert!(script.contains("AppsUseLightTheme 0") && script.contains("SystemUsesLightTheme 0"));
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("oasis-actions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// On the real desktop, by hand: `cargo test -p oasis-platform the_step_after_opening -- --ignored`.
    /// Chrome is opened (or brought forward), then Ctrl+T must land in it.
    #[test]
    #[ignore = "drives the real desktop and Chrome"]
    fn the_step_after_opening_chrome_lands_in_chrome() {
        use oasis_core::actions::{run_steps, ActionStep};
        let dir = std::env::temp_dir().join("oasis-step-after-open");
        let _ = std::fs::create_dir_all(&dir);
        let mut backend = actions(&dir, false);
        let step = |kind: &str, value: &str| ActionStep { kind: kind.into(), value: value.into(), ..Default::default() };
        // Something else in front first, as when the user presses the pad from another program.
        let _ = shell::open("notepad.exe", None);
        std::thread::sleep(Duration::from_millis(1500));
        let started = Instant::now();
        // OASIS_TEST_BROWSER=msedge tests a browser that is not running yet (a cold start).
        let browser = std::env::var("OASIS_TEST_BROWSER").unwrap_or_else(|_| "chrome".into());
        let result = run_steps(&mut backend, &[step("LaunchApp", &browser), step("Shortcut", "Ctrl+T")], 0);
        let took = started.elapsed();
        std::thread::sleep(Duration::from_millis(700));
        let title = oasis_win::window::foreground_window().map(oasis_win::window::window_title).unwrap_or_default();
        let _ = std::process::Command::new("taskkill").args(["/IM", "notepad.exe", "/F"]).output();
        println!("result {:?} in {took:?}, front: {title:?}", result.failure_message);
        assert!(title.to_lowercase().starts_with("new tab"), "Ctrl+T did not reach {browser}: front is {title:?}");
    }

    fn actions(dir: &Path, allow_scripts: bool) -> WindowsActions {
        WindowsActions::new(SecurityLog::at(dir.join("security.log")), allow_scripts)
    }

    #[test]
    fn a_friendly_name_matches_the_window_it_opened() {
        assert!(names_window("spotify", "Spotify", "Song - Artist"));
        assert!(names_window(
            "vs code",
            "Code",
            "main.rs - Visual Studio Code"
        ));
        assert!(names_window(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            "chrome",
            "New Tab"
        ));
        assert!(names_window("Visual Studio Code", "Code", "x"));
        assert!(!names_window("spotify", "chrome", "YouTube"));
        assert!(
            !names_window("ab", "abc", "abc"),
            "a very short name matches too much"
        );
    }

    #[test]
    fn browser_names_map_to_their_program() {
        assert_eq!(browser_executable("Chrome"), Some("chrome.exe"));
        assert_eq!(browser_executable(" Microsoft Edge "), Some("msedge.exe"));
        assert_eq!(browser_executable("netscape"), None);
    }

    #[test]
    fn launch_queries_are_classified() {
        assert!(is_web_page("https://www.google.com/search?q=x") && is_web_page("HTTP://a.test"));
        assert!(!is_web_page("mailto:a@b.test") && !is_web_page("ms-settings:display"));
        let names = ["Spotify".to_string(), r"C:\Apps\Visual Studio Code\Code.exe".to_string()];
        assert!(runs_as(&names, "spotify") && runs_as(&names, "Code"));
        assert!(!runs_as(&names, "spotifyhelper") && !runs_as(&names, "co"));
        assert_eq!(plan_launch("shell:Downloads"), Launch::Shell("shell:Downloads"));
        assert_eq!(plan_launch("SHELL:AppsFolder"), Launch::Shell("SHELL:AppsFolder"));
        assert_eq!(plan_launch("explorer"), Launch::Shell(""), "File Explorer always opens a window");
        assert_eq!(plan_launch(r"C:\Windows\explorer.exe"), Launch::Shell(""));
        assert_eq!(
            plan_launch(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            Launch::Program {
                query: r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                process: Some("chrome".into())
            }
        );
        assert_eq!(
            plan_launch("notepad.exe"),
            Launch::Program { query: "notepad.exe", process: Some("notepad".into()) }
        );
        // A document or a bare name is started, never matched to a running process.
        assert_eq!(plan_launch(r"C:\notes\todo.txt"), Launch::Program { query: r"C:\notes\todo.txt", process: None });
    }

    #[test]
    fn a_blocked_script_is_logged_and_not_run() {
        let dir = temp_dir("blocked");
        let mut a = actions(&dir, false);
        assert!(!run_single(&mut a, "RunScript", "Set-Content x y"));
        let log = std::fs::read_to_string(dir.join("security.log")).unwrap();
        assert!(log.contains("Blocked RunScript"));
    }

    #[test]
    fn an_allowed_script_really_runs_hidden() {
        let dir = temp_dir("script");
        let marker = dir.join("ran.txt");
        let mut a = actions(&dir, true);
        // Ends on a block with no trailing newline: the case `-Command -` drops without a blank line.
        let script = format!("$v = 'ok'\nif ($true) {{\n\n    Set-Content -Path '{}' -Value $v\n}}", marker.display());
        assert!(run_single(&mut a, "RunScript", &script));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(marker.exists(), "the script did not run (the C# argument list would fail here)");
    }

    #[test]
    fn a_blocked_url_is_logged_and_nothing_opens() {
        let dir = temp_dir("url");
        let mut a = actions(&dir, false);
        assert!(!run_single(&mut a, "OpenUrl", "javascript:alert(1)"));
        assert!(std::fs::read_to_string(dir.join("security.log")).unwrap().contains("Blocked OpenUrl"));
    }

    #[test]
    fn an_automation_runs_for_real_with_only_harmless_steps() {
        let dir = temp_dir("auto");
        let mut a = actions(&dir, false);
        let action = ButtonAction {
            kind: "Automation".into(),
            value: String::new(),
            // F24 is bound to nothing; the delay proves pauses really wait.
            steps: vec![
                ActionStep {
                    kind: "Shortcut".into(),
                    value: "F24".into(),
                    ..Default::default()
                },
                ActionStep {
                    kind: "Delay".into(),
                    value: "120".into(),
                    ..Default::default()
                },
                ActionStep {
                    kind: "Shortcut".into(),
                    value: "F24".into(),
                    ..Default::default()
                },
            ],
        };
        let started = Instant::now();
        let ActionOutcome::Automation(result) = run_action(&mut a, &action, 0) else {
            panic!("expected an automation")
        };
        assert!(result.success, "{:?}", result.failure_message);
        assert!(started.elapsed() >= Duration::from_millis(120));
    }
}
