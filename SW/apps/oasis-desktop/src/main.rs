//! The OASIS desktop app: a tray icon and a status window around `oasis-engine`.
//!
//! The engine runs on its own threads and reports through events; this file starts it, forwards
//! its events to the page, and owns the tray and window. Closing the window hides it (OASIS keeps
//! running in the tray); Quit in the tray menu stops it.
//!
//! It starts live (`defaults::windows::config(true)`): presses run their actions. Preview mode,
//! where presses are only shown, is a switch.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod fixes;
mod fixes_core;
mod hud;
mod overlay;
mod settings_view;
mod system;
mod models;
mod onboarding;
mod restart;
mod update;
// The commands, by screen.
mod accounts;
mod analytics;
mod apps;
mod icons;
mod overlay_window;
mod profiles;
mod quick_launch;
mod settings_screen;
mod voice;

use std::sync::Mutex;

use oasis_core::actions::{ActionKind, ActionStep, Next};
use oasis_engine::{EngineHandle, EngineStatus};
use oasis_settings::edit::{ButtonEdit, SetEdit};
use serde::{Deserialize, Serialize};
use settings_view::SettingsView;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use self::{apps::*, overlay_window::*, settings_screen::*, voice::*};

/// The running engine, or why there is none.
#[derive(Default)]
struct EngineSlot {
    handle: Mutex<Option<EngineHandle>>,
    error: Mutex<Option<String>>,
}

#[derive(Serialize)]
struct StatusReply {
    engine: Option<EngineStatus>,
    /// Why the engine could not start (no profiles, no Bluetooth, …).
    error: Option<String>,
}

#[tauri::command]
fn status(slot: State<'_, EngineSlot>) -> StatusReply {
    StatusReply {
        engine: slot
            .handle
            .lock()
            .ok()
            .and_then(|h| h.as_ref().map(EngineHandle::status)),
        error: slot.error.lock().ok().and_then(|e| e.clone()),
    }
}

/// Run `f` on the engine, or say why there is none.
fn with_engine<R>(
    slot: &EngineSlot,
    f: impl FnOnce(&EngineHandle) -> Result<R, String>,
) -> Result<R, String> {
    let guard = slot
        .handle
        .lock()
        .map_err(|_| "OASIS is busy; try again.".to_string())?;
    match guard.as_ref() {
        Some(engine) => f(engine),
        None => Err("OASIS is not running, so settings cannot be changed.".to_string()),
    }
}

/// The first time the Rust app runs it has no profiles of its own (it never reads the C# app's): start it
/// with one profile for everything, which the person then builds on.
///
/// Returns whether it made the file. A new file gets its defaults here, once, and never again:
/// keyboard keys on (so the buttons work without the OASIS interface), button 1 named for voice (it
/// is the talk key by default) and button 3 showing the desktop. Button 2 is left empty; the
/// first-run guide borrows it for its demo and hands it back empty.
#[cfg(windows)]
fn ensure_profiles(paths: &oasis_platform::paths::OasisPaths) -> bool {
    if paths.profiles().exists() {
        return false;
    }
    let Some(dir) = paths.profiles().parent().map(std::path::Path::to_path_buf) else {
        return false;
    };
    let mut settings = oasis_settings::Settings::default();
    if std::fs::create_dir_all(&dir).is_err() || settings.add_profile("Default", "", "").is_err() {
        return false;
    }
    let button = |kind: &str, value: &str, label: &str| ButtonEdit {
        kind: kind.into(),
        value: value.into(),
        label: label.into(),
        steps: Vec::new(),
    };
    // Each is tried on its own: a default that is refused must not cost the profile itself.
    let _ = settings.set_button(0, 0, 0, button(ActionKind::Shortcut.as_str(), "", "Voice"));
    let _ = settings.set_button(0, 0, 2, button(ActionKind::Shortcut.as_str(), "Win+D", "Desktop"));
    let _ = settings.set_input(&oasis_settings::input::InputSettings {
        keyboard: true,
        ..Default::default()
    });
    // Horizontal until the first-run guide asks: F1 to F3, the overlay in a row at the bottom.
    settings.choose_layout(false);
    oasis_platform::profile_store::ProfileStore::new(&dir)
        .save(&settings)
        .is_ok()
}

#[cfg(windows)]
fn start_engine(app: &AppHandle, hold_keys: bool) -> Result<EngineHandle, String> {
    use oasis_engine::{defaults, start};

    let mut config =
        defaults::windows::config(true).ok_or("No home folder found on this computer.")?;
    // The first-run guide is about to open: the keyboard buttons wait for it.
    config.hold_keys_at_start = hold_keys;
    // "Start with OASIS": after sign-in Windows is still busy, so wait longer; a restart OASIS made
    // itself has the desk arranged already.
    let args = oasis_core::startup::LaunchArgs::parse(std::env::args().skip(1));
    config.startup_setup_after =
        (!args.restart).then(|| std::time::Duration::from_secs(if args.tray { 12 } else { 3 }));
    let parts = defaults::windows::parts(&config.paths).map_err(|e| e.to_string())?;
    let events = app.clone();
    start(config, parts, move |event| {
        // An alarm should be seen: bring the window forward.
        if matches!(
            event,
            oasis_engine::EngineEvent::Reminder { .. }
                | oasis_engine::EngineEvent::MeetingReminder { .. }
        ) {
            show_window(&events);
        }
        if let oasis_engine::EngineEvent::Transcript { text, typed: true } = &event {
            fixes::watch(events.clone(), text.clone());
        }
        analytics::on_engine_event(&event);
        onboarding::on_engine_event(&events, &event);
        hud::on_engine_event(&events, &event);
        // 20 times a second while recording, for the pill's bars only: an event name of its own, so
        // the other windows (whose `engine-event` listeners ask for the status each time) are not woken.
        if matches!(event, oasis_engine::EngineEvent::VoiceLevel { .. }) {
            let _ = events.emit("voice-level", &event);
            return;
        }
        let _ = events.emit("engine-event", &event);
    })
    .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn start_engine(_: &AppHandle, _: bool) -> Result<EngineHandle, String> {
    Err("The desktop app is only built for Windows so far.".into())
}

/// The window's contents grow with the window: on a big screen everything is drawn larger instead of
/// the same controls sitting in empty space. 1 up to a 1600 x 900 window (in screen
/// points), at most 1.5. Page zoom, so the page lays itself out as usual in fewer, larger pixels.
fn fit_zoom(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else {
        return;
    };
    if size.width == 0 || size.height == 0 {
        return; // minimised
    }
    let (w, h) = (
        f64::from(size.width) / scale,
        f64::from(size.height) / scale,
    );
    let _ = window.set_zoom((w / 1600.0).min(h / 900.0).clamp(1.0, 1.5));
}

/// Bring OASIS forward: the welcome window while it is open (a second launch, the tray, a reminder),
/// otherwise the main window.
fn show_window(app: &AppHandle) {
    if app.get_webview_window(onboarding::WELCOME).is_some() || onboarding::welcome_due(app) {
        onboarding::show_welcome(app);
    } else {
        show_main(app);
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Stop the engine so the pad is released cleanly.
fn stop_engine(app: &AppHandle) {
    if let Some(mut engine) = app
        .state::<EngineSlot>()
        .handle
        .lock()
        .ok()
        .and_then(|mut h| h.take())
    {
        engine.stop();
    }
}

/// Stop the engine first, then leave.
fn quit(app: &AppHandle) {
    stop_engine(app);
    app.exit(0);
}

/// Keeps the single-copy lock for as long as the app runs.
struct StartupHold(#[allow(dead_code)] Mutex<system::Startup>);

fn main() {
    oasis_win::power::opt_out_of_throttling();
    // A Rust panic is written to crash.log (with the calls that led to it); without this it leaves no trace.
    if let Some(paths) = oasis_platform::paths::OasisPaths::detect() {
        oasis_platform::crash_log::CrashLog::at(paths.crash_log()).install_panic_hook();
    }
    let startup = match system::begin() {
        system::Begin::Run(startup) => startup,
        system::Begin::Exit => return,
    };
    let startup_cell = Mutex::new(Some(startup));
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(EngineSlot::default())
        .manage(update::UpdateState::default())
        .manage(models::ModelsState::default())
        .manage(restart::RestartState::default())
        .invoke_handler(tauri::generate_handler![
            status,
            profiles::settings_view,
            profiles::save_button,
            profiles::set_typing,
            profiles::press_button,
            profiles::restore_caps_lock,
            profiles::sound_list,
            profiles::play_sound,
            profiles::save_profile,
            profiles::save_profile_look,
            profiles::save_input,
            profiles::add_profile,
            profiles::delete_profile,
            profiles::move_profile,
            profiles::set_multi_config,
            profiles::add_config,
            profiles::update_config,
            profiles::delete_config,
            overlay_window::overlay_look,
            overlay_window::overlay_relayout,
            voice::learn_answer,
            overlay_window::move_overlay,
            settings_screen::open_extension_folder,
            voice::history_list,
            voice::history_audio,
            voice::history_delete,
            voice::history_delete_all,
            voice::history_export_dataset,
            voice::open_history_folder,
            settings_screen::settings_fields,
            voice::groq_key_status,
            accounts::spotify_status,
            accounts::spotify_connect,
            accounts::spotify_disconnect,
            accounts::meetings_status,
            accounts::meetings_connect,
            accounts::meetings_disconnect,
            accounts::meeting_reminder_action,
            voice::set_groq_key,
            voice::test_groq_key,
            settings_screen::set_setting,
            settings_screen::reset_overlay_look_setting,
            system::system_status,
            system::set_start_with_windows,
            system::set_run_as_administrator,
            system::restart_oasis,
            settings_screen::reload_settings,
            apps::installed_apps,
            apps::running_apps,
            apps::capture_focus,
            quick_launch::quick_setups,
            quick_launch::save_quick_setup,
            quick_launch::delete_quick_setup,
            quick_launch::quick_screens,
            quick_launch::capture_windows,
            quick_launch::run_quick_setup,
            icons::set_profile_icon,
            icons::profile_icons,
            icons::app_icon,
            icons::site_icon,
            profiles::try_button,
            profiles::profile_use_counts,
            update::update_status,
            restart::restart_pending,
            restart::engine_busy,
            models::models_status,
            models::models_choose,
            models::models_pause,
            models::models_resume,
            models::models_download_anyway,
            models::models_check_again,
            models::models_remove,
            update::check_for_update,
            analytics::privacy_status,
            analytics::privacy_choose,
            analytics::open_privacy_policy,
            update::install_update,
            onboarding::onboarding_state,
            onboarding::onboarding_event,
            onboarding::onboarding_prepare_press,
            onboarding::onboarding_try_press,
            onboarding::onboarding_check_mic,
            onboarding::onboarding_layout,
            onboarding::onboarding_open_mic_settings,
            onboarding::bluetooth_turn_on,
            onboarding::onboarding_test_groq,
            onboarding::onboarding_open_groq_keys
        ])
        .manage(overlay::MoveMode::default())
        .manage(onboarding::OnboardingState::default())
        .setup(move |app| {
            // Before any window is shown: is this a first run, and so the welcome window's turn?
            #[cfg(windows)]
            let created = oasis_engine::defaults::detect_paths().is_some_and(|p| ensure_profiles(&p));
            #[cfg(not(windows))]
            let created = false;
            let guide = onboarding::begin(app.handle(), created);
            if let Some(mut startup) = startup_cell.lock().ok().and_then(|mut s| s.take()) {
                let handle = app.handle().clone();
                system::watch_show_requests(&mut startup, move || show_window(&handle));
                // A start that shows nothing (the tray, "start with Windows") opens the welcome the
                // first time OASIS is asked for: `show_window` does that.
                if startup.show_window {
                    show_window(app.handle());
                }
                app.manage(StartupHold(Mutex::new(startup)));
                fit_zoom(app.handle());
            }
            let slot = app.state::<EngineSlot>();
            match start_engine(app.handle(), guide.hold) {
                Ok(engine) => *slot.handle.lock().expect("fresh lock") = Some(engine),
                Err(why) => *slot.error.lock().expect("fresh lock") = Some(why),
            }
            onboarding::engine_ready(app.handle());
            if let Err(why) = onboarding::create_panel(app.handle()) {
                eprintln!("The guide's caption panel could not be made: {why}");
            }

            let show = MenuItem::with_id(app, "show", "Show OASIS", true, None::<&str>)?;
            // The menu opens just above the pointer, so its last item is the easiest to hit by accident:
            // a separator keeps Quit apart and the label says what it stops.
            let quit_item = MenuItem::with_id(app, "quit", "Quit OASIS (pad and voice stop)", true, None::<&str>)?;
            let move_item =
                MenuItem::with_id(app, "move_overlay", "Move overlay", true, None::<&str>)?;
            let gap = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&show, &move_item, &gap, &quit_item])?;
            app.manage(MoveItem(move_item.clone()));

            let saved = slot.handle.lock().ok().and_then(|h| {
                h.as_ref()
                    .and_then(|e| oasis_settings::overlay::overlay_position(&e.settings()))
            });
            let dock = slot
                .handle
                .lock()
                .ok()
                .and_then(|h| {
                    h.as_ref()
                        .map(|e| oasis_settings::overlay::overlay_appearance(&e.settings()).dock)
                })
                .unwrap_or_else(|| "right".to_string());
            overlay::create(app.handle(), saved, &dock)?;
            hud::create(app.handle())?;
            voice::watch_microphones(app.handle());
            #[cfg(windows)]
            follow_the_screen_in_use(app.handle().clone());
            let mut tray = TrayIconBuilder::new()
                .tooltip("OASIS")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "show" => show_window(app),
                    "move_overlay" => {
                        toggle_overlay_move(app);
                    }
                    "quit" => quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            update::start(app.handle());
            analytics::start(app.handle());
            models::start(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // The welcome window really closes, and the application takes its place.
            WindowEvent::CloseRequested { .. } if window.label() == onboarding::WELCOME => {
                onboarding::send(window.app_handle(), oasis_core::onboarding::Event::PopupClosed);
                show_main(window.app_handle());
            }
            // Back from the browser: the guide goes on.
            WindowEvent::Focused(true) if window.label() == "main" => {
                onboarding::send(window.app_handle(), oasis_core::onboarding::Event::MainFocused);
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                fit_zoom(window.app_handle())
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("the OASIS app could not start");
}
