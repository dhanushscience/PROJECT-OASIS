//! The real parts for this computer.

use oasis_platform::paths::OasisPaths;

/// Windows: Bluetooth through `btleplug`, the foreground window and address bar through Win32 and
/// UI Automation, actions through `SendInput` and the shell, and the DPAPI signature check.
#[cfg(windows)]
pub mod windows {
    use oasis_ble::BleTransport;
    use oasis_platform::integrity::HmacSigner;
    use oasis_platform::paths::OasisPaths;
    use oasis_platform::ports::PlatformResult;
    use oasis_platform::profile_store::SettingsIntegrity;
    use oasis_platform::windows::actions::WindowsActions;
    use oasis_platform::windows::dpapi::DpapiKeyFile;
    use oasis_platform::windows::foreground::foreground_app;
    use oasis_core::actions::app_resolver::{AppSources, Resolved};
    use oasis_core::placement::{self, Desktop, DesktopWindow, PixelRect, Screen, Target, WindowPlacement};
    use oasis_platform::windows::app_discovery;
    use oasis_settings::quick_launch::{Item, ItemKind};

    use crate::quick_launch::QuickLaunchParts;

    use oasis_platform::extension_install::bundled_source_candidates;
    use oasis_platform::secrets::get_or_create_bridge_token;
    use oasis_platform::windows::dpapi::DpapiSecretStore;

    use crate::engine::{BridgeConfig, EngineConfig, EngineParts};
    use crate::voice::{VoiceMic, VoiceParts};

    /// The signature check the C# app does on `profiles.json`.
    pub fn integrity(paths: &OasisPaths) -> Box<dyn SettingsIntegrity> {
        Box::new(HmacSigner::new(
            paths.profiles_signature(),
            DpapiKeyFile::new(paths.secrets_folder().join("profile_hmac.key")),
        ))
    }

    /// The windows on this computer, for Quick Launch.
    pub struct WindowsDesktop;

    fn to_rect(r: oasis_win::desktop::Rect) -> PixelRect {
        PixelRect::new(r.left, r.top, r.right, r.bottom)
    }

    /// Every app window open now as a Quick Launch item, each on its screen and in its place: the
    /// editor's "Capture" and the spoken "save this layout" (W9). A browser window is kept as its
    /// page (the address bar is read the way website profiles read it); minimized windows and the
    /// shell's own are left out.
    pub fn capture_items() -> Vec<oasis_settings::quick_launch::ItemEdit> {
        use oasis_platform::foreground::WindowSource;
        use oasis_settings::quick_launch::ItemEdit;
        let desktop = WindowsDesktop;
        let screens = desktop.screens();
        let mut address = oasis_platform::windows::foreground::Win32Windows::default();
        let mut seen: Vec<String> = Vec::new();
        let mut items = Vec::new();
        for window in desktop.windows() {
            let process = window.process.trim().to_string();
            let skip = process.is_empty()
                || ["oasis-desktop", "applicationframehost", "searchhost", "textinputhost", "shellexperiencehost"].contains(&process.to_lowercase().as_str())
                || window.title == "Program Manager";
            let Some(now) = desktop.read(window.handle).filter(|n| !n.minimized && !skip) else { continue };
            let work = now.screen.and_then(|i| screens.get(i)).map(|s| s.work);
            let (screen, got) = match (now.screen, work) {
                (Some(i), Some(work)) => (placement::screen_spec(&screens, i), placement::capture(&now, work)),
                _ => (String::new(), placement::Captured { layout: placement::Layout::AsIs, region: None }),
            };
            let browser = oasis_core::matching::BrowserProcesses::NAMES.contains(&process.to_lowercase().as_str());
            let url = if browser { address.browser_url(window.handle as u64) } else { String::new() };
            let url = match url.trim() {
                "" => String::new(),
                u if u.contains("://") => u.to_string(),
                u => format!("https://{u}"),
            };
            // Each page opens in a window of its own, so only a second plain window of an app is "another".
            let again = url.is_empty() && seen.contains(&process.to_lowercase());
            if url.is_empty() {
                seen.push(process.to_lowercase());
            }
            let (kind, target, open_with) = if url.is_empty() { ("app", process, String::new()) } else { ("url", url, process) };
            let layout = match got.layout {
                placement::Layout::AsIs => 0,
                placement::Layout::Maximized => 1,
                placement::Layout::LeftHalf => 2,
                placement::Layout::RightHalf => 3,
            };
            items.push(ItemEdit { kind: kind.into(), target, open_with, screen, layout, region: got.region, open_another: again });
        }
        items
    }


    impl Desktop for WindowsDesktop {
        fn windows(&self) -> Vec<DesktopWindow> {
            oasis_win::desktop::app_windows().into_iter().map(|w| DesktopWindow { handle: w.handle, process: w.process, title: w.title }).collect()
        }
        fn foreground(&self) -> i64 {
            oasis_win::desktop::foreground()
        }
        fn screens(&self) -> Vec<Screen> {
            oasis_win::desktop::monitors().into_iter().map(|m| Screen { bounds: to_rect(m.bounds), work: to_rect(m.work), primary: m.primary }).collect()
        }
        fn read(&self, handle: i64) -> Option<WindowPlacement> {
            let p = oasis_win::desktop::read(handle)?;
            let screen = oasis_win::desktop::monitors().iter().position(|m| m.handle == p.monitor);
            Some(WindowPlacement { outer: to_rect(p.outer), visible: to_rect(p.visible), maximized: p.maximized, minimized: p.minimized, screen })
        }
        fn restore(&self, handle: i64) {
            oasis_win::desktop::restore(handle);
        }
        fn maximize(&self, handle: i64) {
            oasis_win::desktop::maximize(handle);
        }
        fn set_outer(&self, handle: i64, outer: PixelRect) {
            oasis_win::desktop::set_outer(handle, oasis_win::desktop::Rect { left: outer.left, top: outer.top, right: outer.right, bottom: outer.bottom });
        }
    }

    /// Starts Quick Launch items the way the C# launcher does: an app by the program it resolves to
    /// (with at most one folder or file to open in it), a link in the browser named or the default
    /// one, a folder in Explorer. Never a command line of its own, never a script.
    pub struct WindowsLauncher;

    /// The program a browser name stands for, or the user's default browser when no name is given.
    /// A named browser that is not installed falls back to the default, so the link still opens.
    fn browser_program(name: &str) -> Option<String> {
        if !name.trim().is_empty() {
            if let Some(exe) = oasis_platform::windows::actions::browser_executable(name) {
                return Some(exe.to_string());
            }
        }
        oasis_win::system::default_browser_exe()
    }

    fn quote(value: &str) -> String {
        format!("\"{}\"", value.replace('"', ""))
    }

    impl crate::quick_launch::Launcher for WindowsLauncher {
        fn resolve(&self, item: &Item) -> Target {
            match item.kind {
                ItemKind::Folder => Target { app_key: "explorer".into(), process_names: vec!["explorer".into()], title_hint: placement::stem(&item.target) },
                ItemKind::Url => match browser_program(&item.open_with).map(|p| placement::stem(&p)).filter(|s| !s.is_empty()) {
                    Some(browser) => Target { app_key: browser.to_lowercase(), process_names: vec![browser], title_hint: String::new() },
                    None => Target { app_key: "browser".into(), process_names: vec![], title_hint: String::new() },
                },
                ItemKind::App => {
                    let typed = placement::stem(&item.target);
                    let mut names: Vec<String> = match app_discovery::resolve(&item.target) {
                        Some(Resolved::Store(id)) => placement::store_app_names(&id),
                        Some(Resolved::Exe(path) | Resolved::Shortcut(path)) => vec![placement::stem(&path)],
                        None => vec![],
                    };
                    names.push(typed.clone());
                    names.retain(|n| !n.is_empty());
                    let mut seen: Vec<String> = Vec::new();
                    names.retain(|n| {
                        let fresh = !seen.contains(&n.to_lowercase());
                        seen.push(n.to_lowercase());
                        fresh
                    });
                    let key = names.first().map_or_else(|| typed.to_lowercase(), |n| n.to_lowercase());
                    Target { app_key: key, process_names: names, title_hint: typed }
                }
            }
        }

        fn start(&self, item: &Item, new_window: bool) -> Result<(), String> {
            let failed = |_| "it could not be started".to_string();
            match item.kind {
                ItemKind::Folder => {
                    let path = app_discovery::windows_apps().expand_env(item.target.trim().trim_matches('"'));
                    if !std::path::Path::new(&path).is_dir() {
                        return Err("the folder was not found".into());
                    }
                    oasis_win::shell::open("explorer.exe", Some(&quote(&path))).map_err(failed)
                }
                ItemKind::Url => {
                    let url = oasis_core::actions::try_allow(&item.target, oasis_core::actions::UrlScope::Automation)
                        .ok_or("that address is not allowed")?;
                    match browser_program(&item.open_with) {
                        Some(browser) => {
                            let flag = if new_window { format!("{} ", placement::new_window_switch(&placement::stem(&browser))) } else { String::new() };
                            oasis_win::shell::open(&browser, Some(&format!("{flag}{}", quote(&url)))).map_err(failed)
                        }
                        // No browser could be worked out: let Windows open the link however it does.
                        None => oasis_win::shell::open(&url, None).map_err(failed),
                    }
                }
                ItemKind::App => {
                    let mut argument = None;
                    if !item.open_with.is_empty() {
                        let path = app_discovery::windows_apps().expand_env(item.open_with.trim().trim_matches('"'));
                        let p = std::path::Path::new(&path);
                        if !p.is_dir() && !p.is_file() {
                            return Err(format!("'{}' to open in it was not found", item.open_with));
                        }
                        argument = Some(quote(&path));
                    }
                    let resolved = app_discovery::resolve(&item.target).ok_or("the app was not found")?;
                    if matches!(resolved, Resolved::Store(_)) && argument.is_some() {
                        return Err("a Store app cannot be given a folder or file to open".into());
                    }
                    match app_discovery::start_command(&resolved) {
                        app_discovery::StartCommand::Open(file) => oasis_win::shell::open(&file, argument.as_deref()).map_err(failed),
                        app_discovery::StartCommand::Explorer(location) => oasis_win::shell::open("explorer.exe", Some(&location)).map_err(failed),
                    }
                }
            }
        }
    }

    pub fn quick_launch_parts() -> QuickLaunchParts {
        QuickLaunchParts { desktop: std::sync::Arc::new(WindowsDesktop), launcher: std::sync::Arc::new(WindowsLauncher) }
    }

    /// Opens `spotify:` addresses and brings the Spotify window forward.
    pub struct WindowsSpotifyHost;

    impl crate::spotify::Host for WindowsSpotifyHost {
        fn open(&self, uri: &str) {
            let _ = oasis_win::shell::open(uri, None);
        }

        fn focus(&self) {
            let _ = oasis_win::focus::main_windows_of("Spotify").into_iter().any(oasis_win::focus::activate);
        }
    }

    /// Spotify with this user's saved sign-in and `spotify_oauth.json` (next to `profiles.json`,
    /// beside the program, or in a `Config` folder beside it).
    pub fn spotify_hub(paths: &OasisPaths) -> std::sync::Arc<crate::spotify::Hub> {
        use crate::spotify::{Auth, Hub, OAuthConfig};
        let mut dirs: Vec<std::path::PathBuf> = paths.profiles().parent().map(std::path::Path::to_path_buf).into_iter().chain([paths.app_data_root.clone()]).collect();
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(std::path::Path::to_path_buf)) {
            dirs.push(exe_dir.join("Config"));
            dirs.push(exe_dir);
        }
        let config = OAuthConfig::load(&dirs, std::env::var("OASIS_SPOTIFY_CLIENT_ID").ok().as_deref());
        let store: std::sync::Arc<dyn oasis_platform::ports::SecretStore> = std::sync::Arc::new(DpapiSecretStore::new(paths.secrets_folder()));
        std::sync::Arc::new(Hub { auth: std::sync::Arc::new(Auth::new(store, config)), host: std::sync::Arc::new(WindowsSpotifyHost) })
    }

    /// Meetings from the user's Google calendar, with the sign-in and client id/secret the C# app
    /// saved. The client id may be in `Meetings.GoogleClientId` of `profiles.json`, the environment,
    /// or `google_oauth.json`; the secret in the protected store, the environment or that file.
    pub fn meetings_hub(paths: &OasisPaths) -> std::sync::Arc<crate::meetings::Meetings> {
        use crate::meetings::{Google, GoogleConfig, Meetings};
        let store: std::sync::Arc<dyn oasis_platform::ports::SecretStore> = std::sync::Arc::new(DpapiSecretStore::new(paths.secrets_folder()));
        let client_id = std::fs::read_to_string(paths.profiles())
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.pointer("/Meetings/GoogleClientId").and_then(|c| c.as_str()).map(str::to_string))
            .unwrap_or_default();
        let mut dirs: Vec<std::path::PathBuf> = paths.profiles().parent().map(std::path::Path::to_path_buf).into_iter().chain([paths.app_data_root.clone()]).collect();
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(std::path::Path::to_path_buf)) {
            dirs.push(exe_dir.join("Config"));
            dirs.push(exe_dir);
        }
        let saved_secret = store.get(oasis_platform::secrets::GOOGLE_CLIENT_SECRET).ok().flatten();
        let env = (std::env::var("OASIS_GOOGLE_CLIENT_ID").ok(), std::env::var("OASIS_GOOGLE_CLIENT_SECRET").ok());
        let config = GoogleConfig::load(&client_id, saved_secret, env, &dirs);
        std::sync::Arc::new(Meetings::new(
            std::sync::Arc::new(Google::new(store, config)),
            std::sync::Arc::new(|url: &str| oasis_win::shell::open(url, None).is_ok()),
        ))
    }

    /// Fails with a sentence for the user if this computer has no Bluetooth adapter.
    pub fn parts(paths: &OasisPaths) -> PlatformResult<EngineParts<BleTransport>> {
        // The installed-app lists, read now in the background so no "open X" waits for them (W3).
        app_discovery::warm();
        Ok(EngineParts {
            transport: BleTransport::new()?,
            foreground: Box::new(foreground_app()),
            voice: Some(voice_parts(paths)),
            quick_launch: Some(quick_launch_parts()),
            spotify: Some(spotify_hub(paths)),
            meetings: Some(meetings_hub(paths)),
            buttons: Some(Box::new(oasis_platform::windows::key_buttons::KeyboardButtons::new())),
            make_backend: Box::new(|context| {
                Box::new(WindowsActions::new(
                    context.security_log,
                    context.allow_run_script,
                ))
            }),
        })
    }

    /// Push-to-talk with the real microphone and the on-device speech model.
    ///
    /// The speech DLLs are looked for in `OASIS_SHERPA_DIR`, beside the program, and in a
    /// `native` folder beside it; models are under `Models\stt` (`OASIS_STT_DIR` names another
    /// folder, for rehearsing).
    pub fn voice_parts(paths: &OasisPaths) -> VoiceParts {
        use oasis_models::{AudioClip, SttEngine};
        use oasis_settings::voice::VoiceSettings;
        use oasis_settings::Settings;
        use oasis_voice::{detect_model, Microphone, SherpaTransducer};

        struct Mic(Microphone);
        impl VoiceMic for Mic {
            fn start(&mut self) -> Result<(), String> {
                self.0.start()
            }
            fn stop(&mut self) -> AudioClip {
                self.0.stop()
            }
            fn take(&mut self) -> AudioClip {
                self.0.take()
            }
            fn level(&self) -> f32 {
                self.0.level()
            }
        }

        let stt_root = std::env::var_os("OASIS_STT_DIR")
            .filter(|v| !v.is_empty())
            .map_or_else(|| paths.stt_folder(), std::path::PathBuf::from);
        // The installer puts the default model in `Models\stt` beside the program; a model of the
        // user's own under their Models folder wins.
        let bundled_root = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("Models").join("stt")));
        let model_folder = move |settings: &Settings| -> std::path::PathBuf {
            let name = VoiceSettings::from(settings).model_folder;
            let mine = stt_root.join(&name);
            if detect_model(&mine, false).is_some() {
                return mine;
            }
            bundled_root.as_ref().map(|r| r.join(&name)).filter(|b| detect_model(b, false).is_some()).unwrap_or(mine)
        };
        let dll_folder = crate::self_check::sherpa_dll_folder;

        let focus_reader = oasis_win::uia::FieldReader::new();
        let ready_folder = model_folder.clone();
        let ready_secrets = DpapiSecretStore::new(paths.secrets_folder());
        let load_secrets = DpapiSecretStore::new(paths.secrets_folder());
        let load_local = move |settings: &Settings| -> Result<Box<dyn SttEngine>, String> {
            let layout = detect_model(&model_folder(settings), false).ok_or("The speech model folder is incomplete.")?;
            let dlls = dll_folder().ok_or("The speech engine files (sherpa-onnx-c-api.dll, onnxruntime.dll) were not found next to OASIS.")?;
            let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(4) as i32);
            let engine = SherpaTransducer::load_model(&dlls, &layout, threads, "cpu").map_err(|e| e.to_string())?;
            Ok(Box::new(engine))
        };
        VoiceParts {
            make_mic: Box::new(|settings: &Settings| Box::new(Mic(Microphone::new(&VoiceSettings::from(settings).microphone)))),
            model_ready: Box::new(move |settings: &Settings| {
                // Groq needs only its key; the local engine needs its files.
                (VoiceSettings::from(settings).use_groq && groq_key(&ready_secrets).is_some())
                    || (dll_folder().is_some() && detect_model(&ready_folder(settings), false).is_some())
            }),
            load_stt: Box::new(move |settings: &Settings| -> Result<Box<dyn SttEngine>, String> {
                let voice = VoiceSettings::from(settings);
                if !voice.use_groq {
                    return load_local(settings);
                }
                match groq_key(&load_secrets) {
                    Some(key) => Ok(Box::new(crate::groq::GroqStt {
                        key,
                        model: voice.groq_model,
                        language: voice.language,
                        prompt: crate::groq::prompt_for(&voice.dictation_glossary),
                        local: if voice.groq_fallback_to_local { load_local(settings).ok() } else { None },
                    })),
                    None if voice.groq_fallback_to_local => load_local(settings),
                    None => Err("The Groq API key is not set. Add one in Settings → Voice, or choose the speech model on this computer.".into()),
                }
            }),
            editable_focus: Box::new(move || focus_reader.editable_focus(std::time::Duration::from_millis(250))),
            running_apps: Box::new(|| {
                oasis_win::window::visible_windows()
                    .into_iter()
                    .map(|(process, title)| oasis_core::voice::command::RunningApp { process, title })
                    .collect()
            }),
            app_installed: Box::new(oasis_platform::windows::app_discovery::is_installed),
            on_screen: std::sync::Arc::new(oasis_platform::windows::app_discovery::on_screen_items),
            app_names: Box::new(|| oasis_platform::windows::app_discovery::windows_apps().cached_app_names(600)),
            pause_media: Box::new(oasis_win::media::pause_playing),
            resume_media: Box::new(oasis_win::media::resume),
        }
    }

    /// The stored Groq API key, if there is one.
    pub fn groq_key(store: &DpapiSecretStore) -> Option<String> {
        use oasis_platform::ports::SecretStore;
        store.get(oasis_platform::secrets::GROQ_API_KEY).ok().flatten().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
    }

    /// The browser-extension socket: this user's secret (shared with the C# app) and where the
    /// bundled extension may be. `None` if the secret cannot be read or created.
    fn bridge_config(paths: &OasisPaths) -> Option<BridgeConfig> {
        let token = get_or_create_bridge_token(&DpapiSecretStore::new(paths.secrets_folder())).ok()?;
        let exe = std::env::current_exe().ok()?;
        let here = std::env::current_dir().unwrap_or_default();
        let port = std::env::var("OASIS_BRIDGE_PORT").ok().and_then(|p| p.parse().ok());
        Some(BridgeConfig { token, bundled_extension: bundled_source_candidates(&exe, &here), port })
    }

    /// Config for this machine's data folder, or `None` if it has no home folder.
    pub fn config(live: bool) -> Option<EngineConfig> {
        let paths = super::detect_paths()?;
        let bridge = bridge_config(&paths);
        Some(EngineConfig { integrity: Some(integrity(&paths)), paths, live, bridge, startup_setup_after: None, hold_keys_at_start: false })
    }
}

/// The data folders for this run: the standard ones for this computer, unless the environment
/// variable `OASIS_DATA_DIR` names a folder to use instead. The override is for developing and
/// testing against a copy, so a change made in a settings window cannot touch the real files.
pub fn detect_paths() -> Option<OasisPaths> {
    match std::env::var_os("OASIS_DATA_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => Some(OasisPaths::with_roots(&dir, &dir)),
        // The standard folders, but the profiles in a folder of their own: the Rust app does not read or write
        // the C# app's `profiles.json`.
        None => OasisPaths::detect().map(|p| {
            let dir = p.app_data_root.join(oasis_platform::paths::RUST_PROFILES_FOLDER_NAME);
            p.with_profiles_dir(dir)
        }),
    }
}
