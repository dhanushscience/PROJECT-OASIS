//! The only way anything leaves this PC for analytics (docs/ANALYTICS.md, PRIVACY.md).
//!
//! One kind of event exists: `usage_summary`, built here from three whole numbers and a date. There
//! is no way to hand this module free text or other properties. It sends nothing unless the person
//! said yes to the current wording (`oasis_settings::privacy`). Events wait in a small file (at most
//! 14 days) and go to Aptabase (EU) in batches; a failed send keeps them, a refused one is dropped.
//! There is no id that lasts: every event gets its own random one.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oasis_settings::Settings;
use serde_json::{json, Value};

use crate::stats::Counts;

/// The Aptabase app key (EU region). It is public by design: it only lets events in.
pub const ANALYTICS_KEY: &str = "A-EU-1609743833";
const MAX_QUEUED: usize = 40;
const BATCH: usize = 25;
const TIMEOUT: Duration = Duration::from_secs(20);

/// True only when the person said yes to the current wording.
pub fn allowed(settings: &Settings) -> bool {
    oasis_settings::privacy::usage_allowed(settings)
}

/// Where an app key sends its events; `None` for a key this build does not know.
fn host(key: &str) -> Option<&'static str> {
    if key.starts_with("A-EU-") {
        Some("https://eu.aptabase.com/api/v0/events")
    } else if key.starts_with("A-US-") {
        Some("https://us.aptabase.com/api/v0/events")
    } else {
        None
    }
}

pub struct Telemetry {
    path: PathBuf,
    key: String,
    version: String,
    queue: Mutex<Vec<Value>>,
}

impl Telemetry {
    /// Reads the waiting events from `path` (a missing or damaged file means none).
    pub fn load(path: PathBuf, key: &str, version: &str) -> Telemetry {
        let queue: Vec<Value> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Telemetry { path, key: key.to_string(), version: version.to_string(), queue: Mutex::new(queue) }
    }

    /// Queues the summary of one finished day (`day`: days since 1970-01-01). Returns whether it
    /// was queued: it is not when consent is off or no key is set.
    pub fn track_day(&self, settings: &Settings, day: i64, counts: &Counts) -> bool {
        if host(&self.key).is_none() || !allowed(settings) {
            return false;
        }
        let (y, m, d) = civil(day);
        // A new id for every event: one per run would tie together all the days a tray app runs for.
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let event = json!({
            "timestamp": now_iso(),
            "sessionId": format!("{nanos:x}{:x}{day:x}", std::process::id()),
            "eventName": "usage_summary",
            "systemProps": { "isDebug": cfg!(debug_assertions), "osName": "Windows", "appVersion": self.version, "sdkVersion": "oasis-1" },
            "props": { "day": y * 10_000 + m * 100 + d, "words": counts.words, "dictations": counts.dictations, "corrections": counts.corrections },
        });
        let Ok(mut queue) = self.queue.lock() else { return false };
        queue.push(event);
        // PRIVACY.md: a day not sent within 14 days is dropped, here as in the counts file.
        let (oy, om, od) = civil(day - crate::stats::KEEP_DAYS);
        let oldest = oy * 10_000 + om * 100 + od;
        queue.retain(|e| e["props"]["day"].as_i64().is_some_and(|d| d > oldest));
        if queue.len() > MAX_QUEUED {
            let extra = queue.len() - MAX_QUEUED;
            queue.drain(..extra);
        }
        self.save(&queue);
        true
    }

    /// Consent is off: delete every waiting event.
    pub fn forget(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.clear();
            self.save(&queue);
        }
    }

    /// Sends what is waiting, a batch at a time, checking `still_on` before each batch so that a
    /// withdrawal during a send stops the rest (a batch already on its way cannot be recalled).
    /// A batch Aptabase refuses (a 4xx answer) is dropped, because it would never be accepted and
    /// would block the queue; a network or server failure keeps it for next time.
    /// Slow: call it from a background thread.
    pub fn flush(&self, still_on: &dyn Fn() -> bool) {
        let Some(url) = host(&self.key) else { return };
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into();
        loop {
            if !still_on() {
                self.forget();
                return;
            }
            let batch: Vec<Value> = match self.queue.lock() {
                Ok(q) if !q.is_empty() => q.iter().take(BATCH).cloned().collect(),
                _ => return,
            };
            let body = serde_json::to_vec(&batch).unwrap_or_default();
            let answer = agent.post(url).header("App-Key", &self.key).content_type("application/json").send(&body[..]);
            let done = match answer {
                Ok(r) => r.status().is_success() || r.status().is_client_error(),
                Err(_) => false,
            };
            if !done {
                return;
            }
            if let Ok(mut q) = self.queue.lock() {
                let n = batch.len().min(q.len());
                q.drain(..n);
                self.save(&q);
            }
        }
    }

    fn save(&self, queue: &[Value]) {
        if queue.is_empty() {
            let _ = std::fs::remove_file(&self.path);
        } else if let Ok(text) = serde_json::to_string(queue) {
            let _ = oasis_platform::atomic_file::write_utf8(&self.path, &text);
        }
    }

    #[cfg(test)]
    fn waiting(&self) -> Vec<Value> {
        self.queue.lock().unwrap().clone()
    }
}

/// Year, month (1-12) and day of a day count since 1970-01-01 (Hinnant's algorithm).
pub fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// The current time as `2026-11-23T10:15:30.000Z`, from the system clock.
fn now_iso() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (year, month, day) = civil(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consenting() -> Settings {
        let mut s = Settings::default();
        oasis_settings::privacy::choose(&mut s, true, true).unwrap();
        s
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oasis-telemetry-{name}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("q.json");
        let _ = std::fs::remove_file(&path);
        path
    }

    const COUNTS: Counts = Counts { words: 120, dictations: 9, corrections: 2 };

    #[test]
    fn nothing_is_queued_without_a_key_or_consent() {
        assert!(!Telemetry::load(temp("nokey"), "", "1").track_day(&consenting(), 20_000, &COUNTS));
        let t = Telemetry::load(temp("nocon"), "A-EU-1", "1");
        assert!(!t.track_day(&Settings::default(), 20_000, &COUNTS));
        assert!(t.waiting().is_empty());
        assert!(t.track_day(&consenting(), 20_000, &COUNTS));
        assert_eq!(t.waiting().len(), 1);
    }

    #[test]
    fn the_event_holds_only_a_date_and_the_three_numbers() {
        let t = Telemetry::load(temp("shape"), "A-EU-1", "0.1.9");
        t.track_day(&consenting(), 20_453, &COUNTS);
        let e = &t.waiting()[0];
        assert_eq!(e["eventName"], "usage_summary");
        assert_eq!(e["props"], json!({ "day": 20_251_231, "words": 120, "dictations": 9, "corrections": 2 }));
        assert_eq!(e["systemProps"]["appVersion"], "0.1.9");
    }

    #[test]
    fn withdrawing_deletes_the_waiting_events_and_they_survive_a_restart_until_then() {
        let path = temp("restart");
        let t = Telemetry::load(path.clone(), "A-EU-1", "1");
        t.track_day(&consenting(), 20_000, &COUNTS);
        assert_eq!(Telemetry::load(path.clone(), "A-EU-1", "1").waiting().len(), 1);
        t.forget();
        assert!(!path.exists() && t.waiting().is_empty());
    }

    #[test]
    fn a_flush_after_withdrawal_sends_nothing_and_clears_the_queue() {
        let path = temp("withdrawn");
        let t = Telemetry::load(path.clone(), "A-EU-1", "1");
        t.track_day(&consenting(), 20_000, &COUNTS);
        t.flush(&|| false);
        assert!(t.waiting().is_empty() && !path.exists());
    }

    #[test]
    fn the_queue_is_bounded_and_the_clock_text_is_iso() {
        let t = Telemetry::load(temp("bound"), "A-EU-1", "1");
        for d in 0..60 {
            t.track_day(&consenting(), 20_000 + d, &COUNTS);
        }
        // Only the last 14 days wait, each with its own id, so days cannot be linked.
        let waiting = t.waiting();
        assert_eq!(waiting.len(), crate::stats::KEEP_DAYS as usize);
        let ids: std::collections::HashSet<_> = waiting.iter().map(|e| e["sessionId"].to_string()).collect();
        assert_eq!(ids.len(), waiting.len(), "one id per event");
        assert!(host("A-EU-9").unwrap().contains("eu.") && host("A-SH-9").is_none() && host("").is_none());
        let now = now_iso();
        assert!(now.starts_with("20") && now.ends_with(".000Z") && now.len() == 24, "{now}");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_453), (2025, 12, 31));
    }
}
