//! "open project" when a folder called Project is on the Desktop or in an open Explorer window:
//! the files and folders the user can see, matched by name. The engine lists them (Desktop
//! folders and the folders open Explorer windows show); this only matches. `docs/OPEN_ANYTHING.md` stage 6.

use crate::matching::edit_distance::levenshtein;
use crate::matching::web_catalog::compact;

/// One file or folder the user can see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenItem {
    /// The name as shown, with extension ("Report.pdf", "Project").
    pub name: String,
    /// The full path.
    pub path: String,
    pub folder: bool,
    /// In the File Explorer window in front: what the user is looking at, so it wins a tie.
    pub front: bool,
    /// Selected in the window in front (or on the Desktop when it is in front): "open this".
    pub selected: bool,
}

impl ScreenItem {
    /// What to hand `LaunchApp`: a folder ends in `\`, which [`is_plain_open`] reads as "a folder".
    pub fn launch_value(&self) -> String {
        if self.folder && !self.path.ends_with('\\') { format!("{}\\", self.path) } else { self.path.clone() }
    }

    /// The name without its extension, for a folder the whole name ("v1.2" stays).
    pub fn stem(&self) -> &str {
        if self.folder {
            return &self.name;
        }
        self.name.rsplit_once('.').map_or(&self.name, |(stem, _)| if stem.is_empty() { &self.name } else { stem })
    }
}

/// What a spoken name found.
#[derive(Debug, PartialEq, Eq)]
pub enum Found<'a> {
    None,
    /// The name as said, exactly ("project" for `Project`). Beats an app of a similar name.
    Exact(&'a ScreenItem),
    /// The start of a name ("project" for `Project 2026`), or a close spelling.
    Close(&'a ScreenItem),
    /// Several are equally good: say more of the name.
    Many(Vec<&'a ScreenItem>),
}

/// The item `spoken` names. Tiers, best first: the whole name, the start of the name, a slip of
/// about one letter in five. Within the best tier that has anything, one item wins or it is `Many`.
pub fn find<'a>(spoken: &str, items: &'a [ScreenItem]) -> Found<'a> {
    let wanted = compact(spoken);
    if wanted.len() < 2 {
        return Found::None;
    }
    let tiers: [&dyn Fn(&ScreenItem) -> bool; 3] = [
        &|i| compact(i.stem()) == wanted || compact(&i.name) == wanted,
        &|i| wanted.len() >= 3 && compact(i.stem()).starts_with(&wanted),
        &|i| {
            let name = compact(i.stem());
            name.len() >= 5 && levenshtein(&wanted, &name) * 5 <= name.len()
        },
    ];
    for (tier, matches) in tiers.iter().enumerate() {
        let mut hits: Vec<&ScreenItem> = items.iter().filter(|i| matches(i)).collect();
        // The Desktop can also be open in an Explorer window: one item, listed twice.
        let mut seen = std::collections::HashSet::new();
        hits.retain(|i| seen.insert(i.path.to_lowercase()));
        // "open bin" with a BIN folder in the window in front means that one, not one elsewhere.
        if hits.iter().any(|i| i.front) {
            hits.retain(|i| i.front);
        }
        match hits.len() {
            0 => continue,
            1 if tier == 0 => return Found::Exact(hits[0]),
            1 => return Found::Close(hits[0]),
            _ => return Found::Many(hits),
        }
    }
    Found::None
}

/// File types that only show content when opened: a document, picture, sound or video, never a
/// program, script or shortcut. These open without the first-use question.
const PLAIN: &[&str] = &[
    "pdf", "txt", "md", "rtf", "csv", "log", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp",
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "heic", "svg", "mp3", "wav", "m4a", "flac", "ogg", "mp4", "mkv",
    "mov", "avi", "webm", "zip", "7z", "rar",
];

/// Whether a `LaunchApp` value is a folder (an absolute path ending in `\`) or a plain document.
/// Programs, scripts, shortcuts and macro-enabled documents are not.
pub fn is_plain_open(value: &str) -> bool {
    let v = value.trim();
    let absolute = v.get(1..3) == Some(":\\") || v.starts_with("\\\\");
    if !absolute || v.contains(['|', '"']) {
        return false;
    }
    if v.ends_with('\\') {
        return true;
    }
    let file = v.rsplit('\\').next().unwrap_or(v);
    file.rsplit_once('.').is_some_and(|(_, ext)| PLAIN.contains(&ext.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, folder: bool) -> ScreenItem {
        ScreenItem { name: name.into(), path: format!(r"C:\Users\me\Desktop\{name}"), folder, front: false, selected: false }
    }

    #[test]
    fn the_window_in_front_wins_a_tie() {
        let front = ScreenItem { name: "BIN".into(), path: r"G:\BIN".into(), folder: true, front: true, selected: false };
        let items = [item("bin", true), front.clone()];
        assert_eq!(find("bin", &items), Found::Exact(&front));
        let items = [item("Project old", true), item("Project new", true), ScreenItem { front: true, ..item("Project x", true) }];
        assert!(matches!(find("project", &items), Found::Close(i) if i.name == "Project x"));
    }

    #[test]
    fn names_on_screen_are_found_by_the_whole_name_its_start_or_a_slip() {
        let items = [item("Project", true), item("Report 2026.pdf", false), item("Holiday photos", true), item("setup.exe", false)];
        assert_eq!(find("project", &items), Found::Exact(&items[0]));
        assert_eq!(find("Project.", &items), Found::Exact(&items[0]));
        assert_eq!(find("report", &items), Found::Close(&items[1]), "the start of the name");
        assert_eq!(find("report 2026", &items), Found::Exact(&items[1]), "without the extension");
        assert_eq!(find("holiday fotos", &items), Found::Close(&items[2]), "a slip");
        assert_eq!(find("chrome", &items), Found::None);
        assert_eq!(find("p", &items), Found::None, "too short to mean anything");
    }

    #[test]
    fn two_equally_good_names_are_not_guessed() {
        let items = [item("Project old", true), item("Project new", true)];
        assert!(matches!(find("project", &items), Found::Many(v) if v.len() == 2));
        assert_eq!(find("project new", &items), Found::Exact(&items[1]));
    }

    #[test]
    fn only_folders_and_documents_open_without_asking() {
        assert!(is_plain_open(r"C:\Users\me\Desktop\Project\"));
        assert!(is_plain_open(r"C:\Users\me\Desktop\Report.PDF"));
        assert!(is_plain_open(r"\\server\share\notes.txt"));
        for v in [r"C:\Users\me\Desktop\setup.exe", r"C:\x\run.bat", r"C:\x\Chrome.lnk", r"C:\x\macro.docm", r"C:\x\noext", "notepad", "chrome|https://a.test", "C:\\x\\a.pdf\" & calc"] {
            assert!(!is_plain_open(v), "{v}");
        }
        assert_eq!(item("Project", true).launch_value(), r"C:\Users\me\Desktop\Project\");
    }
}
