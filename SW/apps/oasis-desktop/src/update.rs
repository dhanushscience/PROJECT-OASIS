//! Updates from GitHub Releases (Tauri updater). The app reads `latest.json` from the public
//! releases repo set in tauri.conf.json, checks the installer's signature against the public key
//! there, and on Windows runs the new installer, which closes this copy and starts the new one.
//! It checks at start and twice a day; installing waits for the person to press Install.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

const CHECK_EVERY: Duration = Duration::from_secs(12 * 60 * 60);

/// What the Settings card shows. Sent as the `update-status` event whenever it changes.
#[derive(Clone, Default, Serialize)]
pub struct UpdateInfo {
    current: String,
    /// The newer version on offer, if any.
    available: Option<String>,
    notes: Option<String>,
    /// When the last check finished (seconds since 1970).
    checked: Option<u64>,
    error: Option<String>,
    checking: bool,
    /// Download progress 0–100 while installing.
    progress: Option<u8>,
}

#[derive(Default)]
pub struct UpdateState(Mutex<UpdateInfo>);

fn change(app: &AppHandle, f: impl FnOnce(&mut UpdateInfo)) -> UpdateInfo {
    let state = app.state::<UpdateState>();
    let mut info = state.0.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut info);
    let out = info.clone();
    drop(info);
    let _ = app.emit("update-status", &out);
    out
}

/// Starts the background checks; call once from setup.
pub fn start(app: &AppHandle) {
    let version = app.package_info().version.to_string();
    change(app, |i| i.current = version);
    let app = app.clone();
    std::thread::spawn(move || {
        // The themed pop-up (UpdatePrompt.svelte) listens to `update-status` and asks the person.
        loop {
            tauri::async_runtime::block_on(check(&app));
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

async fn find(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>, String> {
    app.updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())
}

async fn check(app: &AppHandle) -> UpdateInfo {
    change(app, |i| i.checking = true);
    let found = find(app).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    change(app, |i| {
        i.checking = false;
        i.checked = Some(now);
        match found {
            Ok(update) => {
                i.error = None;
                i.available = update.as_ref().map(|u| u.version.clone());
                i.notes = update.and_then(|u| u.body);
            }
            Err(why) => i.error = Some(why),
        }
    })
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateInfo {
    app.state::<UpdateState>()
        .0
        .lock()
        .map(|i| i.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> UpdateInfo {
    check(&app).await
}

/// Downloads and installs the update on offer, then restarts into it. On Windows the installer
/// takes over and this process ends inside `install`.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let result = download_and_install(&app).await;
    if let Err(why) = &result {
        change(&app, |i| {
            i.progress = None;
            i.error = Some(why.clone());
        });
    }
    result
}

async fn download_and_install(app: &AppHandle) -> Result<(), String> {
    let update = find(app).await?.ok_or("There is no update to install.")?;
    change(app, |i| i.progress = Some(0));
    let (mut got, mut shown) = (0u64, 0u8);
    let bytes = update
        .download(
            |chunk, total| {
                got += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let pct = (got * 100 / total).min(100) as u8;
                    if pct != shown {
                        shown = pct;
                        change(app, |i| i.progress = Some(pct));
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // Release the pad cleanly before the installer replaces the program.
    crate::stop_engine(app);
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}
