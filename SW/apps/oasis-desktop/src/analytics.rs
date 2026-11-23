//! Anonymous analytics for the desktop app (docs/ANALYTICS.md, PRIVACY.md).
//!
//! Three whole numbers are counted on this PC (words dictated, dictations made, dictations the
//! person corrected by hand), and the finished days go out as one small summary each through
//! `oasis_engine::telemetry`. Nothing is counted or sent unless the person said yes to the
//! current wording; the counting path reads one flag and takes no other lock.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use oasis_engine::stats::{Metric, Stats};
use oasis_engine::telemetry::{allowed, Telemetry, ANALYTICS_KEY};
use oasis_settings::Settings;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::{with_engine, EngineSlot};

static TELEMETRY: OnceLock<Telemetry> = OnceLock::new();
static STATS: OnceLock<Mutex<Stats>> = OnceLock::new();
/// Whether the person said yes to the current wording. Counting reads only this.
static ON: AtomicBool = AtomicBool::new(false);
const FIRST_SEND_AFTER: Duration = Duration::from_secs(30);
const SEND_EVERY: Duration = Duration::from_secs(3 * 60 * 60);
/// The full policy, on the website (a page not linked from the home page; docs/ANALYTICS.md).
const POLICY_URL: &str = "https://dhanushscience.github.io/PROJECT-OASIS/privacy.html";

fn settings(app: &AppHandle) -> Option<Settings> {
    let slot = app.state::<EngineSlot>();
    let guard = slot.handle.lock().ok()?;
    guard.as_ref().map(|e| e.settings())
}

/// Today as a day count (local time), so a day turns over at the person's midnight.
fn today() -> i64 {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    (unix + i64::from(oasis_win::system::utc_offset_seconds())).div_euclid(86_400)
}

/// Applies the person's answer: sets the flag and, when it is off, deletes what was kept.
fn apply(settings: &Settings) {
    let on = allowed(settings);
    ON.store(on, Ordering::SeqCst);
    if on {
        return;
    }
    if let Some(stats) = STATS.get() {
        if let Ok(mut st) = stats.lock() {
            st.forget(false);
        }
    }
    if let Some(t) = TELEMETRY.get() {
        t.forget();
    }
}

/// Starts the sender: after 30 seconds and every 3 hours, finished days go out as summaries.
pub fn start(app: &AppHandle) {
    let Some(paths) = oasis_engine::defaults::detect_paths() else { return };
    let profiles = paths.profiles();
    let version = app.package_info().version.to_string();
    let telemetry = TELEMETRY.get_or_init(|| Telemetry::load(profiles.with_file_name("telemetry_queue.json"), ANALYTICS_KEY, &version));
    let stats = STATS.get_or_init(|| Mutex::new(Stats::load(profiles.with_file_name("usage_stats.json"))));
    if let Some(s) = settings(app) {
        apply(&s);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_SEND_AFTER);
        loop {
            if let Some(s) = settings(&app) {
                apply(&s);
                let finished = stats.lock().map(|mut st| st.take_finished(allowed(&s), today())).unwrap_or_default();
                for (day, counts) in finished {
                    telemetry.track_day(&s, day, &counts);
                }
                telemetry.flush(&|| settings(&app).is_some_and(|s| allowed(&s)));
            }
            std::thread::sleep(SEND_EVERY);
        }
    });
}

/// Adds to one of the three counters. Free when the person has not said yes.
pub fn count(metric: Metric, n: u64) {
    if !ON.load(Ordering::SeqCst) {
        return;
    }
    if let Some(stats) = STATS.get() {
        if let Ok(mut st) = stats.lock() {
            st.count(true, metric, n, today());
        }
    }
}

/// A dictation that was typed is one dictation and its words. Nothing else the engine reports is counted.
pub fn on_engine_event(event: &oasis_engine::EngineEvent) {
    if let oasis_engine::EngineEvent::Transcript { text, typed: true } = event {
        count(Metric::Dictations, 1);
        count(Metric::Words, text.split_whitespace().count() as u64);
    }
}

#[derive(Serialize)]
pub struct PrivacyStatus {
    /// They have answered the current wording, yes or no.
    answered: bool,
    /// Sending is on.
    usage: bool,
}

fn status(settings: &Settings) -> PrivacyStatus {
    PrivacyStatus { answered: oasis_settings::privacy::answered(settings), usage: allowed(settings) }
}

#[tauri::command]
pub fn privacy_status(slot: State<'_, EngineSlot>) -> Result<PrivacyStatus, String> {
    with_engine(&slot, |engine| Ok(status(&engine.settings())))
}

/// Saves the answer. Turning it on needs `adult` (the 18-or-older confirmation).
#[tauri::command]
pub fn privacy_choose(slot: State<'_, EngineSlot>, usage: bool, adult: bool) -> Result<PrivacyStatus, String> {
    with_engine(&slot, |engine| {
        engine.change(|s| oasis_settings::privacy::choose(s, usage, adult)).map_err(|e| e.to_string())?;
        let settings = engine.settings();
        apply(&settings);
        Ok(status(&settings))
    })
}

/// Opens the privacy policy in the browser.
#[tauri::command]
pub fn open_privacy_policy() -> Result<(), String> {
    #[cfg(windows)]
    {
        oasis_win::shell::open(POLICY_URL, None).map_err(|code| format!("Windows could not open the page (code {code}). It is at {POLICY_URL}"))
    }
    #[cfg(not(windows))]
    {
        Err(format!("The privacy policy is at {POLICY_URL}"))
    }
}
