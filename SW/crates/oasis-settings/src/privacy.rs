//! The one analytics consent, kept in `profiles.json` under `Privacy`. Off until the person says yes.
//!
//! Two values: `Usage` (their choice) and `AskedVersion` (the wording they answered). Sending is
//! allowed only when both hold, so changing the wording (bump [`CONSENT_VERSION`]) stops sending
//! and asks again, even for someone who said yes before.

use serde_json::{json, Value};

use crate::edit::EditError;
use crate::model::Settings;

/// Bump this whenever the notice or the data collected changes.
pub const CONSENT_VERSION: u32 = 2;

fn stored<'a>(settings: &'a Settings, key: &str) -> Option<&'a Value> {
    settings.extra.get("Privacy")?.get(key)
}

/// Whether the person has answered the current wording (yes or no).
pub fn answered(settings: &Settings) -> bool {
    stored(settings, "AskedVersion").and_then(Value::as_u64) == Some(u64::from(CONSENT_VERSION))
}

/// Whether sending is allowed: they said yes, to the current wording. Anything odd means no.
pub fn usage_allowed(settings: &Settings) -> bool {
    answered(settings) && stored(settings, "Usage").and_then(Value::as_bool) == Some(true)
}

/// Records the answer. Turning it on needs the person to have confirmed they are 18 or older;
/// turning it off, or saying no, never does. Leftovers of older wording are removed.
pub fn choose(settings: &mut Settings, usage: bool, adult: bool) -> Result<(), EditError> {
    if usage && !adult {
        return Err(EditError::Invalid("Please confirm that you are 18 or older to turn this on.".into()));
    }
    settings.extra.insert("Privacy".into(), json!({ "Usage": usage, "AskedVersion": CONSENT_VERSION }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_and_unanswered_by_default() {
        let s = Settings::default();
        assert!(!answered(&s) && !usage_allowed(&s));
    }

    #[test]
    fn yes_needs_the_age_confirmation_and_no_never_does() {
        let mut s = Settings::default();
        assert!(choose(&mut s, true, false).is_err());
        assert!(!answered(&s), "a refused answer changes nothing");
        choose(&mut s, false, false).unwrap();
        assert!(answered(&s) && !usage_allowed(&s));
        choose(&mut s, true, true).unwrap();
        assert!(usage_allowed(&s));
    }

    #[test]
    fn a_yes_to_older_wording_does_not_count_and_old_keys_are_cleared() {
        let mut s = Settings::default();
        s.extra.insert("Privacy".into(), json!({ "Usage": true, "Asked": true }));
        assert!(!usage_allowed(&s) && !answered(&s));
        s.extra.insert("Privacy".into(), json!({ "Usage": true, "AskedVersion": CONSENT_VERSION - 1 }));
        assert!(!usage_allowed(&s));
        choose(&mut s, false, false).unwrap();
        assert_eq!(s.extra["Privacy"], json!({ "Usage": false, "AskedVersion": CONSENT_VERSION }));
    }

    #[test]
    fn wrong_types_mean_no() {
        let mut s = Settings::default();
        s.extra.insert("Privacy".into(), json!({ "Usage": "yes", "AskedVersion": CONSENT_VERSION }));
        assert!(!usage_allowed(&s));
    }
}
