//! The Windows places that can be opened by name: This PC, Recycle Bin, Downloads, … They are
//! Explorer locations (`shell:` addresses), not programs, so the app lookup never finds them. A
//! fixed list, like `settings_pages`: a spoken phrase can only open an address from here.
//! See `docs/OPEN_ANYTHING.md`.

use crate::matching::web_catalog::compact;

/// `(display name, spoken names, address)`. Names include common speech-to-text slips.
const PLACES: &[(&str, &[&str], &str)] = &[
    ("THIS PC", &["this pc", "my pc", "this computer", "my computer", "computer"],"shell:MyComputerFolder"),
    ("RECYCLE BIN", &[
        "recycle bin", "recycling bin", "recycle", "recycled bin", "recycle been", "recycle pin", "recycle ben",
        "recycle bins", "bin", "trash", "trash can", "trash bin", "rubbish bin", "rubbish", "deleted files",
        "deleted items", "dustbin", "waste bin",
    ], "shell:RecycleBinFolder"),
    ("DOWNLOADS", &["downloads", "download", "my downloads", "downloaded files"], "shell:Downloads"),
    ("DOCUMENTS", &["documents", "document", "my documents", "docs"], "shell:Personal"),
    ("DESKTOP", &["desktop", "desktop folder"], "shell:Desktop"),
    ("PICTURES", &["pictures", "photos", "my pictures", "my photos", "images"], "shell:My Pictures"),
    ("MUSIC", &["music", "my music"], "shell:My Music"),
    ("VIDEOS", &["videos", "video", "my videos"], "shell:My Video"),
    ("CONTROL PANEL", &["control panel", "controlpanel", "control panels"], "shell:ControlPanelFolder"),
    ("NETWORK", &["network", "network folder", "network places"], "shell:NetworkPlacesFolder"),
    ("STARTUP FOLDER", &["startup folder", "start up folder"], "shell:Startup"),
];

/// The display name and address for a spoken place ("the trash", "my computer"), if listed.
/// A trailing "folder" or "window" is ignored ("downloads folder").
pub fn resolve(spoken: &str) -> Option<(&'static str, &'static str)> {
    let lookup = |text: &str| {
        let key = compact(text);
        PLACES.iter().find(|(_, names, _)| !key.is_empty() && names.iter().any(|n| compact(n) == key)).map(|(name, _, addr)| (*name, *addr))
    };
    let text = spoken.trim();
    let bare = [" folder", " window"].into_iter().find_map(|s| text.strip_suffix(s)).unwrap_or(text);
    let bare_no_the = ["the ", "my "].into_iter().find_map(|p| bare.strip_prefix(p)).unwrap_or(bare);
    lookup(text).or_else(|| lookup(bare)).or_else(|| lookup(bare_no_the)).or_else(|| recycle(bare_no_the))
}

/// "recycle then", "recycle bit", "recycle bank": speech hears "recycle" well and "bin" badly, so
/// "recycle" plus at most one more word is the bin. ("empty recycle bin" never gets here: it is not
/// an open phrase, and `safety.rs` blocks it.)
fn recycle(text: &str) -> Option<(&'static str, &'static str)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    (words.len() <= 2 && words.first().is_some_and(|w| w.starts_with("recycl"))).then_some(("RECYCLE BIN", "shell:RecycleBinFolder"))
}

/// Every spoken name, with the place's display name and address: for "did you mean …?".
pub fn names() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    PLACES.iter().flat_map(|(name, spoken, addr)| spoken.iter().map(move |s| (*s, *name, *addr)))
}

/// True for an address on the list.
pub fn is_known(address: &str) -> bool {
    PLACES.iter().any(|(_, _, a)| a.eq_ignore_ascii_case(address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_places_resolve_and_only_listed_addresses_are_known() {
        for said in ["recycle bin", "Recycle Bin", "the trash", "recycling bin", "recycle been", "the recycle bin folder", "bin", "recycle then", "recycle bit", "recycle bank", "recycle pin", "recycled bean", "the recycle"] {
            assert_eq!(resolve(said).map(|p| p.1), Some("shell:RecycleBinFolder"), "{said}");
        }
        for said in ["this pc", "This PC", "this p.c.", "my computer", "the computer", "this computer"] {
            assert_eq!(resolve(said).map(|p| p.1), Some("shell:MyComputerFolder"), "{said}");
        }
        assert_eq!(resolve("downloads folder"), Some(("DOWNLOADS", "shell:Downloads")));
        assert_eq!(resolve("my documents").map(|p| p.1), Some("shell:Personal"));
        assert_eq!(resolve("control panel").map(|p| p.1), Some("shell:ControlPanelFolder"));
        assert_eq!(resolve("chrome"), None);
        assert_eq!(resolve("recycle my old phone today"), None, "a sentence about recycling is not the bin");
        assert_eq!(resolve(""), None);
        assert_eq!(resolve("folder"), None);
        assert!(is_known("shell:RecycleBinFolder"));
        assert!(!is_known("shell:RecycleBinFolder & calc"));
        assert!(!is_known("shell:AppsFolder"));
    }
}
