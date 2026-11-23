//! Names as the local model's own word pieces, for sherpa-onnx hotwords: one line per name
//! ("▁Re cy cle ▁B in"). sherpa-onnx uses a line that starts with "▁" and has only pieces from
//! `tokens.txt` as it is, so no `bpe.vocab` is needed. Which names: `oasis_core::speech::hotwords`.

use std::collections::HashSet;

/// One phrase as word pieces from `vocab`, longest piece first at each point, or `None` when a
/// character has no piece. Words start with "▁", as in the model's tokens.
pub fn encode(phrase: &str, vocab: &HashSet<String>) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for word in phrase.split_whitespace() {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        if word.is_empty() {
            continue;
        }
        let text: Vec<char> = std::iter::once('▁').chain(word.chars()).collect();
        let mut at = 0;
        while at < text.len() {
            let piece = (at + 1..=text.len()).rev().find_map(|end| vocab.get(&text[at..end].iter().collect::<String>()))?;
            out.push(piece.as_str());
            at += piece.chars().count();
        }
    }
    (!out.is_empty()).then(|| out.join(" "))
}

/// The hotwords text for sherpa-onnx: one line per spelling the model may write (as written, lower
/// case, and Title Case), each as word pieces. Phrases with a character the model has no piece for are left out.
pub fn lines(phrases: &[String], vocab: &HashSet<String>) -> String {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for phrase in phrases {
        let title: String = phrase
            .split_whitespace()
            .map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ");
        for spelling in [phrase.clone(), phrase.to_lowercase(), title] {
            if let Some(line) = encode(&spelling, vocab).filter(|l| seen.insert(l.clone())) {
                out.push(line);
            }
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(pieces: &[&str]) -> HashSet<String> {
        pieces.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn a_name_becomes_the_longest_pieces_the_model_has() {
        let v = vocab(&["▁re", "▁rec", "y", "cy", "cle", "c", "l", "e", "▁b", "▁bin", "in", "▁R", "▁B", "ecy"]);
        assert_eq!(encode("recycle bin", &v).as_deref(), Some("▁rec y cle ▁bin"));
        assert_eq!(encode("Recycle Bin", &v).as_deref(), Some("▁R ecy cle ▁B in"));
        assert_eq!(encode("(Bin!)", &v).as_deref(), Some("▁B in"), "punctuation around a word is dropped");
        assert_eq!(encode("zz", &v), None, "a character with no piece: left out");
        assert_eq!(encode("  ", &v), None);
    }

    #[test]
    fn each_spelling_is_one_line_and_unencodable_names_are_skipped() {
        let v = vocab(&["▁rec", "y", "cle", "▁bin", "▁R", "ecy", "▁B", "in"]);
        let text = lines(&["Recycle Bin".into(), "recycle bin".into(), "Ünïcode".into()], &v);
        assert_eq!(text, "▁R ecy cle ▁B in\n▁rec y cle ▁bin");
    }
}
