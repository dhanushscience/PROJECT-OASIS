//! Where the installed apps come from on Windows: Start Menu shortcuts, Start Apps (including the
//! Store), the usual install folders and the registry's App Paths. Port of the data half of
//! `Services/WindowsAppDiscovery.cs`; the rules that choose among them are in
//! `oasis_core::actions::app_resolver`.
//!
//! The Start Apps list comes from PowerShell's `Get-StartApps` and takes a second or two, so it is
//! read at OASIS start in the background ([`warm`]) and kept for ten minutes; after that the old list
//! is still used while a new one is read, so no press waits for PowerShell (`docs/APP_WINDOWS.md`
//! W3). The install folders are indexed at the same time. The Start Menu is walked at most every
//! thirty seconds. What a name resolved to is remembered ([`resolve`], W2/W4).

use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use oasis_core::actions::app_resolver::{name_matches, normalize_token, squash, AppSources, Resolved};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const START_APPS_TTL: Duration = Duration::from_secs(600);
const START_APPS_RETRY: Duration = Duration::from_secs(60);
const START_APPS_TIMEOUT: Duration = Duration::from_secs(8);
const SHORTCUTS_TTL: Duration = Duration::from_secs(30);
/// How deep the Start Menu is walked: folders of folders of shortcuts, no more.
const MAX_MENU_DEPTH: usize = 6;

type Entries = Vec<(String, String)>;

struct Cached {
    at: Instant,
    ttl: Duration,
    entries: Entries,
}

impl Cached {
    fn fresh(&self) -> bool {
        self.at.elapsed() < self.ttl
    }
}

/// The real machine, with caches that outlive one action.
#[derive(Default)]
pub struct WindowsApps {
    shortcuts: Mutex<Option<Cached>>,
    start_apps: Mutex<Option<Cached>>,
    install_index: Mutex<Option<InstallIndex>>,
}

/// The one shared instance: actions are created per press, the caches must not be.
pub fn windows_apps() -> &'static WindowsApps {
    static APPS: OnceLock<WindowsApps> = OnceLock::new();
    APPS.get_or_init(WindowsApps::default)
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn start_menu_folders() -> Vec<PathBuf> {
    [
        env_path("ProgramData").map(|p| p.join(r"Microsoft\Windows\Start Menu\Programs")),
        env_path("APPDATA").map(|p| p.join(r"Microsoft\Windows\Start Menu\Programs")),
    ]
    .into_iter()
    .flatten()
    .filter(|p| p.is_dir())
    .collect()
}

fn walk_shortcuts(dir: &Path, depth: usize, out: &mut Entries) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < MAX_MENU_DEPTH {
                walk_shortcuts(&path, depth + 1, out);
            }
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
            if let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) {
                out.push((stem, path.to_string_lossy().into_owned()));
            }
        }
    }
}

/// `Get-StartApps | ConvertTo-Json`: one object, or an array of them.
pub fn parse_start_apps(json: &str) -> Entries {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let items = match value {
        serde_json::Value::Array(items) => items,
        object @ serde_json::Value::Object(_) => vec![object],
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item.get("Name")?.as_str()?.trim();
            let id = item.get("AppID")?.as_str()?.trim();
            (!name.is_empty() && !id.is_empty()).then(|| (name.to_string(), id.to_string()))
        })
        .collect()
}

fn read_start_apps() -> Entries {
    let Ok(mut child) = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "Get-StartApps | ConvertTo-Json -Compress"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    else {
        return Vec::new();
    };
    let Some(mut stdout) = child.stdout.take() else { return Vec::new() };
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < START_APPS_TIMEOUT => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                break;
            }
        }
    }
    parse_start_apps(&reader.join().unwrap_or_default())
}

/// Each folder under the usual install roots, with the programs in it and in its `bin` folder
/// (FreeCAD 1.0\bin\freecad.exe). Slow (hundreds of folders), so built off the press path.
type InstallIndex = Vec<(String, Vec<PathBuf>)>;

fn scan_install_folders() -> InstallIndex {
    let mut index = Vec::new();
    for root in ["LOCALAPPDATA", "APPDATA", "ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(|n| env_path(n)) {
        let Ok(read) = std::fs::read_dir(&root) else { continue };
        for dir in read.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            let Some(folder) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            let mut exes = Vec::new();
            for place in [dir.clone(), dir.join("bin")] {
                let Ok(files) = std::fs::read_dir(&place) else { continue };
                exes.extend(files.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))));
            }
            if !exes.is_empty() {
                index.push((folder, exes));
            }
        }
    }
    index
}

/// In a folder whose name matches: the program named like the folder, else one whose own name matches.
fn pick_install_exe(index: &InstallIndex, token: &str) -> Option<String> {
    let stem = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    index.iter().filter(|(folder, _)| name_matches(token, folder)).find_map(|(folder, exes)| {
        let same = exes.iter().find(|e| e.parent().and_then(Path::file_name).is_some_and(|p| p.to_string_lossy() == *folder) && stem(e).eq_ignore_ascii_case(folder));
        same.or_else(|| exes.iter().find(|e| name_matches(token, &stem(e)))).map(|e| e.to_string_lossy().into_owned())
    })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What each spoken name resolved to (W2, W4): the voice check and the launch share one lookup, and
/// a name said again skips the matching. Cleared whenever the app lists are read again.
static RESOLVED: Mutex<Option<std::collections::HashMap<String, Option<Resolved>>>> = Mutex::new(None);
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// [`app_resolver::resolve`] over this machine, remembered. A result is only kept once the Start Apps
/// list is known: one made without it could miss a Store app.
pub fn resolve(lookup: &str) -> Option<Resolved> {
    let key = lookup.trim().to_lowercase();
    if let Some(hit) = lock(&RESOLVED).as_ref().and_then(|m| m.get(&key)) {
        return hit.clone();
    }
    let apps = windows_apps();
    let found = oasis_core::actions::app_resolver::resolve(lookup, apps);
    if !apps.cached_app_names(1).is_empty() {
        lock(&RESOLVED).get_or_insert_with(Default::default).insert(key, found.clone());
    }
    found
}

/// Read the app lists now, off the caller's thread (W3): at OASIS start, and when a list is out of
/// date (the old one is used meanwhile, so no press waits for PowerShell). One read at a time.
pub fn warm() {
    if REFRESHING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let apps = windows_apps();
        let mut entries = read_start_apps();
        if entries.is_empty() {
            entries = read_start_apps();
        }
        let ttl = if entries.is_empty() { START_APPS_RETRY } else { START_APPS_TTL };
        // An empty answer never replaces a list we had.
        {
            let mut guard = lock(&apps.start_apps);
            if !entries.is_empty() || guard.is_none() {
                *guard = Some(Cached { at: Instant::now(), ttl, entries });
            } else if let Some(old) = guard.as_mut() {
                old.at = Instant::now();
                old.ttl = START_APPS_RETRY;
            }
        }
        let _ = apps.start_menu_shortcuts();
        *lock(&apps.install_index) = Some(scan_install_folders());
        *lock(&RESOLVED) = None;
        REFRESHING.store(false, Ordering::SeqCst);
    });
}

/// Installed apps for the window's action field, as (name, path or Store id): Start Menu
/// shortcuts and Store apps, sorted by name, each name once (the shortcut wins), no uninstallers.
pub fn merge_installed(shortcuts: Entries, start_apps: Entries) -> Entries {
    let mut apps: Entries =
        shortcuts.into_iter().chain(start_apps).filter(|(name, _)| !name.to_lowercase().starts_with("uninstall")).collect();
    apps.sort_by_key(|(name, _)| name.to_lowercase()); // stable: a shortcut stays before its Store twin
    apps.dedup_by(|later, first| later.0.eq_ignore_ascii_case(&first.0));
    apps
}

impl WindowsApps {
    /// [`merge_installed`] over this machine. Both lists are cached; the first call can take a
    /// few seconds (`Get-StartApps`).
    pub fn installed_apps(&self) -> Entries {
        merge_installed(self.start_menu_shortcuts(), self.start_apps())
    }

    /// Installed app names for a vocabulary, from what is already known: never starts a scan.
    pub fn cached_app_names(&self, max: usize) -> Vec<String> {
        let guard = self.start_apps.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|c| c.entries.iter().take(max).map(|(n, _)| n.clone()).collect()).unwrap_or_default()
    }
}

impl AppSources for WindowsApps {
    fn file_exists(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn expand_env(&self, text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%') {
                Some(end) if end > 0 => match std::env::var(&after[..end]) {
                    Ok(value) => {
                        out.push_str(&value);
                        rest = &after[end + 1..];
                    }
                    Err(_) => {
                        out.push('%');
                        rest = after;
                    }
                },
                _ => {
                    out.push('%');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }

    fn start_menu_shortcuts(&self) -> Entries {
        let mut guard = self.shortcuts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = guard.as_ref().filter(|c| c.fresh()) {
            return cached.entries.clone();
        }
        let mut entries = Vec::new();
        for folder in start_menu_folders() {
            walk_shortcuts(&folder, 0, &mut entries);
        }
        *guard = Some(Cached { at: Instant::now(), ttl: SHORTCUTS_TTL, entries: entries.clone() });
        entries
    }

    fn start_apps(&self) -> Entries {
        let mut guard = self.start_apps.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = guard.as_ref().filter(|c| c.fresh()) {
            return cached.entries.clone();
        }
        // Out of date: use it as it is and read it again in the background (W3).
        if let Some(stale) = guard.as_ref().filter(|c| !c.entries.is_empty()) {
            let entries = stale.entries.clone();
            drop(guard);
            warm();
            return entries;
        }
        // Never read yet (a press before the start-up read finished): read it now.
        // An empty answer is usually a slow first PowerShell start, not a PC with no apps: ask once
        // more, or "spotify" falls through to its console helper instead of the Store app.
        let mut entries = read_start_apps();
        if entries.is_empty() {
            entries = read_start_apps();
        }
        let ttl = if entries.is_empty() { START_APPS_RETRY } else { START_APPS_TTL };
        *guard = Some(Cached { at: Instant::now(), ttl, entries: entries.clone() });
        entries
    }

    fn install_folder_exe(&self, token: &str) -> Option<String> {
        let mut guard = lock(&self.install_index);
        // Built in the background at start (W3); only a press before that pays for the scan.
        let index = guard.get_or_insert_with(scan_install_folders);
        pick_install_exe(index, token)
    }

    fn app_paths_exe(&self, token: &str) -> Option<String> {
        oasis_win::system::app_paths().into_iter().find_map(|(name, path)| {
            // Exact name only: "contains" picks helpers such as spotify_cli.exe (a console flash).
            (squash(token) == squash(&normalize_token(&name)) && Path::new(&path).is_file()).then_some(path)
        })
    }
}

/// A Store app whose id is a path under a Program Files folder (`{GUID}\Tool\tool.exe`): the real
/// file, so it can be started directly.
pub fn expand_start_app_path(app_id: &str) -> Option<String> {
    if Path::new(app_id).is_file() {
        return Some(app_id.to_string());
    }
    const PROGRAM_FILES_GUID: &str = "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}";
    let relative = app_id.strip_prefix(PROGRAM_FILES_GUID)?.trim_start_matches(['\\', '/']);
    ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|n| env_path(n))
        .map(|root| root.join(relative))
        .find(|full| full.is_file())
        .map(|full| full.to_string_lossy().into_owned())
}

/// What a `LaunchApp` name really starts on this machine: a shortcut's target file, a program
/// path, or a Store app's id. Used to refuse a spoken name that looks harmless but starts a
/// shell ("command" is the Command Prompt shortcut). Same lookup as the action runner.
pub fn launch_target(query: &str) -> Option<String> {
    let lookup = oasis_core::voice::command::apps::launch_name(query).unwrap_or(query);
    Some(match resolve(lookup)? {
        Resolved::Shortcut(lnk) => oasis_win::shell::shortcut_target(&lnk).unwrap_or(lnk),
        Resolved::Exe(path) => path,
        Resolved::Store(id) => expand_start_app_path(&id).unwrap_or(id),
    })
}

/// The files and folders the user can see: the Desktop and the folders open in File Explorer
/// (`docs/OPEN_ANYTHING.md` stage 6). Hidden and system files (`desktop.ini`) are left out, and at
/// most 500 per folder are read, so a huge Downloads folder costs a few milliseconds, not seconds.
pub fn on_screen_items() -> Vec<oasis_core::voice::command::on_screen::ScreenItem> {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
    let mut items = Vec::new();
    let (folders, selected) = oasis_win::shell::on_screen_folders();
    let is_selected = |path: &std::path::Path| selected.iter().any(|s| std::path::Path::new(s) == path);
    for (folder, front) in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten().take(500) {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.file_attributes() & HIDDEN_OR_SYSTEM != 0 {
                continue;
            }
            items.push(oasis_core::voice::command::on_screen::ScreenItem {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path().to_string_lossy().into_owned(),
                folder: meta.is_dir(),
                front,
                selected: is_selected(&entry.path()),
            });
        }
    }
    // Selected but not listed (past the 500 read per folder): still "this".
    for path in &selected {
        if !items.iter().any(|i| i.selected && std::path::Path::new(&i.path) == std::path::Path::new(path)) {
            let p = std::path::Path::new(path);
            let Some(name) = p.file_name() else { continue };
            items.push(oasis_core::voice::command::on_screen::ScreenItem {
                name: name.to_string_lossy().into_owned(),
                path: path.clone(),
                folder: p.is_dir(),
                front: true,
                selected: true,
            });
        }
    }
    items
}

/// Whether `query` names a program really on this machine, for a spoken "open X"
/// (`docs/VOICE_DECISIONS.md` V4). Unlike [`launch_target`] it does not accept the lookup's last
/// guess, a bare `X.exe`, unless that program is on the PATH; so "open the door" finds nothing.
/// `browser` is the default browser.
pub fn is_installed(query: &str) -> bool {
    if query == "browser" {
        return oasis_win::shell::default_browser().is_some();
    }
    let lookup = oasis_core::voice::command::apps::launch_name(query).unwrap_or(query);
    match resolve(lookup) {
        Some(Resolved::Shortcut(_) | Resolved::Store(_)) => true,
        Some(Resolved::Exe(path)) if path.contains(['\\', '/']) => std::path::Path::new(&path).is_file(),
        Some(Resolved::Exe(name)) => on_path(&name),
        None => false,
    }
}

/// A program file of this name in one of the PATH folders.
fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|dirs| std::env::split_paths(&dirs).any(|dir| dir.join(name).is_file()))
}

/// What to hand the shell for a resolved app: a program path, a shortcut path, or
/// `explorer.exe` with the Store app's `shell:AppsFolder` address.
#[derive(Debug, PartialEq, Eq)]
pub enum StartCommand {
    Open(String),
    Explorer(String),
}

pub fn start_command(resolved: &Resolved) -> StartCommand {
    match resolved {
        Resolved::Exe(path) | Resolved::Shortcut(path) => StartCommand::Open(path.clone()),
        Resolved::Store(id) => match expand_start_app_path(id) {
            Some(file) => StartCommand::Open(file),
            None => StartCommand::Explorer(format!("shell:AppsFolder\\{id}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_index_finds_a_program_by_its_folder_or_its_own_name() {
        let index: InstallIndex = vec![
            ("KiCad".into(), vec![PathBuf::from(r"C:\PF\KiCad\bin\kicad.exe"), PathBuf::from(r"C:\PF\KiCad\bin\pcbnew.exe")]),
            ("FreeCAD 1.0".into(), vec![PathBuf::from(r"C:\PF\FreeCAD 1.0\bin\FreeCAD.exe"), PathBuf::from(r"C:\PF\FreeCAD 1.0\bin\FreeCADCmd.exe")]),
            ("Zed".into(), vec![PathBuf::from(r"C:\PF\Zed\uninstall.exe"), PathBuf::from(r"C:\PF\Zed\Zed.exe")]),
        ];
        assert_eq!(pick_install_exe(&index, "kicad").as_deref(), Some(r"C:\PF\KiCad\bin\kicad.exe"));
        assert_eq!(pick_install_exe(&index, "free cad").as_deref(), Some(r"C:\PF\FreeCAD 1.0\bin\FreeCAD.exe"));
        assert_eq!(pick_install_exe(&index, "zed").as_deref(), Some(r"C:\PF\Zed\Zed.exe"), "the program named like its folder, not the uninstaller");
        assert_eq!(pick_install_exe(&index, "photoshop"), None);
    }

    #[test]
    fn a_name_is_resolved_once_and_then_remembered() {
        // The machine's own lists; slow the first time, a map lookup after.
        let first = resolve("notepad");
        let started = Instant::now();
        assert_eq!(resolve("notepad"), first);
        if !windows_apps().cached_app_names(1).is_empty() {
            assert!(started.elapsed() < Duration::from_millis(5), "{:?}", started.elapsed());
        }
    }

    #[test]
    fn start_apps_json_is_read_for_one_app_or_many() {
        let many = r#"[{"Name":"Spotify","AppID":"SpotifyAB.SpotifyMusic!Spotify"},{"Name":"Notepad","AppID":"Microsoft.WindowsNotepad!App"},{"Name":"","AppID":"x"}]"#;
        assert_eq!(parse_start_apps(many).len(), 2, "a nameless entry is skipped");
        let one = r#"{"Name":"Only","AppID":"id"}"#;
        assert_eq!(parse_start_apps(one), [("Only".to_string(), "id".to_string())]);
        assert!(parse_start_apps("").is_empty() && parse_start_apps("not json").is_empty() && parse_start_apps("42").is_empty());
    }

    #[test]
    fn environment_variables_are_expanded_and_unknown_ones_left_alone() {
        let apps = WindowsApps::default();
        std::env::set_var("OASIS_TEST_VAR", "value");
        assert_eq!(apps.expand_env(r"%OASIS_TEST_VAR%\x"), r"value\x");
        assert_eq!(apps.expand_env("%NO_SUCH_VARIABLE_XYZ%\\x"), "%NO_SUCH_VARIABLE_XYZ%\\x");
        assert_eq!(apps.expand_env("100%"), "100%");
        assert_eq!(apps.expand_env("plain"), "plain");
    }

    #[test]
    fn a_store_app_id_that_is_a_file_is_started_directly_and_others_go_through_explorer() {
        let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        assert_eq!(start_command(&Resolved::Store(exe.clone())), StartCommand::Open(exe));
        assert_eq!(
            start_command(&Resolved::Store("Pkg.App!App".into())),
            StartCommand::Explorer(r"shell:AppsFolder\Pkg.App!App".into())
        );
        assert_eq!(start_command(&Resolved::Shortcut(r"C:\x.lnk".into())), StartCommand::Open(r"C:\x.lnk".into()));
    }

    #[test]
    fn this_machines_start_menu_is_readable_and_has_shortcuts() {
        let shortcuts = WindowsApps::default().start_menu_shortcuts();
        assert!(!shortcuts.is_empty(), "no Start Menu shortcuts found");
        assert!(shortcuts.iter().all(|(name, path)| !name.is_empty() && path.to_lowercase().ends_with(".lnk")));
    }

    #[test]
    fn a_known_program_resolves_on_this_machine() {
        // Windows always has Notepad; it resolves to a Store app, a shortcut or notepad.exe.
        let found = oasis_core::actions::app_resolver::resolve("notepad", windows_apps());
        assert!(found.is_some());
    }

    #[test]
    fn installed_apps_are_sorted_once_each_without_uninstallers() {
        let entry = |n: &str, p: &str| (n.to_string(), p.to_string());
        let shortcuts = vec![entry("Spotify", r"C:\menu\Spotify.lnk"), entry("Uninstall Foo", r"C:\menu\u.lnk"), entry("Figma", r"C:\menu\Figma.lnk")];
        let store = vec![entry("spotify", "SpotifyAB.SpotifyMusic!Spotify"), entry("Calculator", "Microsoft.WindowsCalculator!App")];
        assert_eq!(
            merge_installed(shortcuts, store),
            [entry("Calculator", "Microsoft.WindowsCalculator!App"), entry("Figma", r"C:\menu\Figma.lnk"), entry("Spotify", r"C:\menu\Spotify.lnk")]
        );
    }

    #[test]
    fn this_machine_lists_its_installed_apps() {
        let apps = windows_apps().installed_apps();
        assert!(apps.iter().any(|(name, _)| name.eq_ignore_ascii_case("notepad")), "{} apps, no Notepad", apps.len());
    }

    #[test]
    fn a_harmless_sounding_name_that_starts_a_shell_is_seen_through() {
        use oasis_core::voice::command::safety::names_a_shell;
        // Every Windows has the "Command Prompt" shortcut; "command" matches it by name.
        for name in ["command", "command prompt", "windows powershell"] {
            let target = launch_target(name);
            assert!(target.as_deref().is_some_and(names_a_shell), "{name} -> {target:?}");
        }
        let notepad = launch_target("notepad");
        assert!(!notepad.as_deref().is_some_and(names_a_shell), "{notepad:?}");
    }

    #[test]
    fn a_spoken_name_counts_as_installed_only_when_it_is_really_here() {
        let started = Instant::now();
        let items = on_screen_items();
        eprintln!("on screen: {} items ({} in front) from {:?} in {:?}", items.len(), items.iter().filter(|i| i.front).count(), oasis_win::shell::on_screen_folders().0, started.elapsed());
        assert!(!oasis_win::shell::on_screen_folders().0.is_empty(), "the Desktop is always there");
        assert!(is_installed("notepad"), "Windows always has Notepad");
        assert!(is_installed("cmd"), "a program on the PATH");
        assert!(!is_installed("the door for me"), "the lookup's last guess is not a program");
        assert!(!is_installed("zzqx nonexistent tool"));
        assert_eq!(is_installed("browser"), oasis_win::shell::default_browser().is_some());
    }
}
