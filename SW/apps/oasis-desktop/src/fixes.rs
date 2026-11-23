//! After a dictation is typed, watch the field it went into for a little while. If the person
//! retypes a misheard word by hand ("keycat" → "KiCad"), offer it to the dictionary: the HUD (bottom middle)
//! shows it with a short countdown and saves it unless told not to. The loop itself is in
//! `fixes_core`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use oasis_win::uia::FieldReader;
use tauri::AppHandle;

use crate::fixes_core;

const READ_WAIT: Duration = Duration::from_millis(800);

/// Bumped by every dictation, so a newer one ends the watch on the older.
static WATCH: AtomicU64 = AtomicU64::new(0);

/// The focused field's text, with Windows line ends read as plain new lines.
fn read() -> Option<String> {
    static READER: OnceLock<FieldReader> = OnceLock::new();
    let text = READER
        .get_or_init(FieldReader::new)
        .focused_text(READ_WAIT)?;
    Some(text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// Start watching for hand fixes of `dictated`, just typed into the focused field.
pub fn watch(app: AppHandle, dictated: String) {
    let ticket = WATCH.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = thread::Builder::new()
        .name("oasis-fixes".into())
        .spawn(move || {
            let mut counted = false;
            fixes_core::run(
                &dictated,
                fixes_core::REAL,
                &|| WATCH.load(Ordering::SeqCst) == ticket,
                &read,
                &oasis_win::desktop::foreground,
                &mut |fixes| {
                    if !std::mem::replace(&mut counted, true) {
                        crate::analytics::count(oasis_engine::stats::Metric::Corrections, 1);
                    }
                    offer(&app, fixes)
                },
                &note,
            );
        });
}

/// One line in `fixes.log` (beside the other logs) about why a watch ended or did what it did. It
/// never holds what was said.
fn note(line: &str) {
    use std::io::Write;
    let Some(paths) = oasis_platform::paths::OasisPaths::detect() else {
        return;
    };
    let path = paths.app_data_root.join("fixes.log");
    // Keep it small: start over once it grows past 100 KB.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 100_000) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let app = oasis_win::window::foreground_window()
            .and_then(oasis_win::window::window_process_id)
            .and_then(oasis_win::window::process_name)
            .unwrap_or_else(|| "?".into());
        let _ = writeln!(file, "{app}: {line}");
    }
}

/// Show the words on the HUD (bottom middle), which counts down and answers with `learn_answer`.
fn offer(app: &AppHandle, fixes: Vec<(String, String)>) {
    crate::hud::offer_learn(app, fixes);
}
