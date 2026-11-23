//! The three counters behind the daily analytics summary (docs/ANALYTICS.md, PRIVACY.md).
//!
//! Only whole numbers: words dictated, dictations made, dictations the person corrected by hand
//! afterwards. They are added up here, per day, on this PC. At most one summary per finished day
//! leaves, and at most [`MAX_PER_MONTH`] a month, which keeps 500 daily users under Aptabase's
//! free 20,000 events (500 x 31 = 15,500). When consent is off nothing is counted and the file
//! is deleted.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::telemetry::civil;

/// Summaries one install may send in a calendar month.
pub const MAX_PER_MONTH: u32 = 31;
/// Days an unsent count is kept; older ones are dropped rather than sent late.
pub(crate) const KEEP_DAYS: i64 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Words,
    Dictations,
    Corrections,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub words: u64,
    pub dictations: u64,
    pub corrections: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Stats {
    #[serde(skip)]
    path: PathBuf,
    /// Counts by day (days since 1970-01-01, local time).
    days: BTreeMap<i64, Counts>,
    /// Month number (`year * 12 + month`) and how many summaries went out in it.
    month: i64,
    month_sent: u32,
}

fn month_id(day: i64) -> i64 {
    let (y, m, _) = civil(day);
    y * 12 + m
}

impl Stats {
    /// Reads the file; a missing, damaged or older-format one starts from zero.
    pub fn load(path: PathBuf) -> Stats {
        let mut stats: Stats = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        stats.path = path;
        stats
    }

    fn save(&self) {
        if let Ok(text) = serde_json::to_string(self) {
            let _ = oasis_platform::atomic_file::write_utf8(&self.path, &text);
        }
    }

    /// Adds `n` to a counter for `day`. Does nothing unless `on` (the person's consent).
    pub fn count(&mut self, on: bool, metric: Metric, n: u64, day: i64) {
        if !on || n == 0 {
            return;
        }
        let today = self.days.entry(day).or_default();
        let field = match metric {
            Metric::Words => &mut today.words,
            Metric::Dictations => &mut today.dictations,
            Metric::Corrections => &mut today.corrections,
        };
        *field = field.saturating_add(n);
        self.save();
    }

    /// Consent is off: forget every count and delete the file.
    pub fn forget(&mut self, on: bool) {
        if on || self.days.is_empty() && !self.path.exists() {
            return;
        }
        *self = Stats { path: std::mem::take(&mut self.path), ..Stats::default() };
        let _ = std::fs::remove_file(&self.path);
    }

    /// The finished days (before `today`) with something to report, oldest first, limited by this
    /// month's allowance. Taking a day removes it, so each is sent once.
    pub fn take_finished(&mut self, on: bool, today: i64) -> Vec<(i64, Counts)> {
        if !on {
            return Vec::new();
        }
        self.days.retain(|d, _| *d > today - KEEP_DAYS);
        let month = month_id(today);
        if self.month != month {
            (self.month, self.month_sent) = (month, 0);
        }
        let room = MAX_PER_MONTH.saturating_sub(self.month_sent) as usize;
        let due: Vec<i64> = self.days.range(..today).map(|(d, _)| *d).take(room).collect();
        let out: Vec<(i64, Counts)> = due.iter().filter_map(|d| self.days.remove(d).map(|c| (*d, c))).collect();
        self.month_sent += out.len() as u32;
        if !out.is_empty() {
            self.save();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) -> Stats {
        let dir = std::env::temp_dir().join(format!("oasis-stats-{name}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s.json");
        let _ = std::fs::remove_file(&path);
        Stats::load(path)
    }

    #[test]
    fn nothing_counts_while_off() {
        let mut s = fresh("off");
        s.count(false, Metric::Words, 5, 100);
        assert!(s.days.is_empty() && !s.path.exists());
    }

    #[test]
    fn finished_days_go_once_and_today_waits() {
        let mut s = fresh("once");
        s.count(true, Metric::Words, 10, 100);
        s.count(true, Metric::Words, 5, 100);
        s.count(true, Metric::Dictations, 2, 100);
        s.count(true, Metric::Corrections, 1, 101);
        let sent = s.take_finished(true, 101);
        assert_eq!(sent, vec![(100, Counts { words: 15, dictations: 2, corrections: 0 })]);
        assert!(s.take_finished(true, 101).is_empty(), "sent once");
        assert_eq!(s.take_finished(true, 102).len(), 1, "day 101 is finished on day 102");
    }

    #[test]
    fn counts_survive_a_restart() {
        let mut s = fresh("restart");
        s.count(true, Metric::Words, 3, 100);
        let again = Stats::load(s.path.clone());
        assert_eq!(again.days.get(&100).map(|c| c.words), Some(3));
    }

    #[test]
    fn a_month_never_sends_more_than_the_limit_and_old_days_are_dropped() {
        let mut s = fresh("cap");
        for d in 19_700..19_740 {
            s.count(true, Metric::Words, 1, d);
        }
        // One calendar month (day 19_730 is in the same month as 19_740 minus 10 days): all take from one allowance.
        let today = 19_740;
        let first = s.take_finished(true, today);
        assert!(first.len() as u32 <= MAX_PER_MONTH);
        assert!(first.iter().all(|(d, _)| *d > today - KEEP_DAYS), "{first:?}");
        s.month_sent = MAX_PER_MONTH;
        s.count(true, Metric::Words, 1, today - 1);
        assert!(s.take_finished(true, today).is_empty());
    }

    #[test]
    fn turning_consent_off_deletes_everything() {
        let mut s = fresh("forget");
        s.count(true, Metric::Corrections, 4, 100);
        assert!(s.path.exists());
        s.forget(false);
        assert!(s.days.is_empty() && !s.path.exists());
        assert!(s.take_finished(false, 200).is_empty());
    }
}
