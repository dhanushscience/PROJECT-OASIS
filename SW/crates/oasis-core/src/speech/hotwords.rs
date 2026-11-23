//! Names the local speech model should listen for (`docs/OPEN_ANYTHING.md` stage 2, `VOICE_DECISIONS.md`
//! V6/V15): the Dictionary, and for a command the Windows places and what is on screen. Which names;
//! writing them as the model's word pieces is the engine's business (`oasis-voice` `hotwords`).

use std::collections::HashSet;

/// At most this many names per recording: every one is a chance for the model to hear a name that
/// was not said, so the list stays short.
pub const MAX_PHRASES: usize = 150;

/// The names for one recording, each once, longest lists cut at [`MAX_PHRASES`]. `dictionary` comes
/// first (the user chose those words), then `places`, then `on_screen`.
pub fn phrases(dictionary: &[String], places: &[&str], on_screen: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    dictionary
        .iter()
        .map(String::as_str)
        .chain(places.iter().copied())
        .chain(on_screen.iter().map(String::as_str))
        .map(str::trim)
        .filter(|p| p.chars().any(char::is_alphabetic) && p.chars().count() >= 3)
        .filter(|p| seen.insert(p.to_lowercase()))
        .take(MAX_PHRASES)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_short_unique_and_dictionary_first() {
        let p = phrases(&["OASIS".into(), "Kicad".into()], &["recycle bin", "oasis"], &["BIN".into(), "x".into(), "2026".into()]);
        assert_eq!(p, ["OASIS", "Kicad", "recycle bin", "BIN"], "duplicates, one-letter and digit-only names dropped");
        let many: Vec<String> = (0..400).map(|i| format!("name{i}")).collect();
        assert_eq!(phrases(&[], &[], &many).len(), MAX_PHRASES);
    }
}
