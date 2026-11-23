//! The Windows Settings pages that can be opened by name. Port of `SettingsPages.cs`. A fixed
//! list: the `OpenSettings` action opens only an address from here, so a spoken phrase (or an
//! imported button) can never turn into an arbitrary `ms-settings:` or command line.

const PAGES: &[(&[&str], &str)] = &[
    (&["settings", "windows", "home", "system"], "ms-settings:"),
    (&["wifi", "wireless", "wifi network"], "ms-settings:network-wifi"),
    (&["network", "internet", "network and internet", "ethernet"], "ms-settings:network"),
    (&["bluetooth", "bluetooth and devices", "devices"], "ms-settings:bluetooth"),
    (&["display", "screen", "brightness", "resolution", "monitor"], "ms-settings:display"),
    (&["night light", "nightlight", "blue light"], "ms-settings:nightlight"),
    (&["sound", "audio", "volume", "speakers"], "ms-settings:sound"),
    (&["notification", "notifications"], "ms-settings:notifications"),
    (&["focus", "do not disturb", "quiet hours", "focus assist"], "ms-settings:quiethours"),
    (&["battery", "battery saver"], "ms-settings:batterysaver"),
    (&["power", "power and sleep", "sleep"], "ms-settings:powersleep"),
    (&["storage", "disk space"], "ms-settings:storagesense"),
    (&["apps", "installed apps", "apps and features"], "ms-settings:appsfeatures"),
    (&["default apps", "defaults"], "ms-settings:defaultapps"),
    (&["startup", "startup apps"], "ms-settings:startupapps"),
    (&["update", "windows update", "updates"], "ms-settings:windowsupdate"),
    (&["privacy", "privacy and security"], "ms-settings:privacy"),
    (&["camera", "webcam"], "ms-settings:privacy-webcam"),
    (&["microphone", "mic"], "ms-settings:privacy-microphone"),
    (&["personalization", "personalisation", "background", "wallpaper", "theme", "themes"], "ms-settings:personalization"),
    (&["color", "colors", "colour", "colours", "accent color"], "ms-settings:colors"),
    (&["taskbar"], "ms-settings:taskbar"),
    (&["lock screen"], "ms-settings:lockscreen"),
    (&["account", "accounts", "your info"], "ms-settings:yourinfo"),
    (&["date and time", "time", "date", "clock", "time and date"], "ms-settings:dateandtime"),
    (&["language", "region", "language and region"], "ms-settings:regionlanguage"),
    (&["keyboard", "typing"], "ms-settings:typing"),
    (&["mouse", "touchpad", "trackpad"], "ms-settings:mousetouchpad"),
    (&["accessibility", "ease of access"], "ms-settings:easeofaccess"),
    (&["about", "about this pc", "pc info", "device info", "specs"], "ms-settings:about"),
    (&["vpn"], "ms-settings:network-vpn"),
    (&["airplane", "airplane mode", "flight mode"], "ms-settings:network-airplanemode"),
    (&["hotspot", "mobile hotspot"], "ms-settings:network-mobilehotspot"),
    (&["printer", "printers", "printers and scanners"], "ms-settings:printers"),
    (&["game", "games", "gaming", "game bar"], "ms-settings:gaming-gamebar"),
    (&["multitasking", "snap", "snap layouts"], "ms-settings:multitasking"),
    (&["clipboard"], "ms-settings:clipboard"),
    (&["troubleshoot", "troubleshooting"], "ms-settings:troubleshoot"),
    (&["recovery"], "ms-settings:recovery"),
    (&["project", "projection", "second screen", "external display"], "ms-settings:project"),
];

fn flatten(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The page for a spoken name ("night light", "bluetooth"), if one is listed.
pub fn resolve(spoken: &str) -> Option<&'static str> {
    let wanted = flatten(spoken);
    if wanted.is_empty() {
        return None;
    }
    PAGES.iter().find(|(names, _)| names.iter().any(|n| flatten(n) == wanted)).map(|(_, uri)| *uri)
}

/// Every name a page goes by, with its address: for "did you mean …?" (`docs/OPEN_ANYTHING.md` stage 3).
pub fn names() -> impl Iterator<Item = (&'static str, &'static str)> {
    PAGES.iter().flat_map(|(names, uri)| names.iter().map(move |n| (*n, *uri)))
}

/// True for an address on the list. This is what `OpenSettings` checks before opening anything.
pub fn is_known(uri: &str) -> bool {
    PAGES.iter().any(|(_, u)| *u == uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_and_only_listed_addresses_are_known() {
        assert_eq!(resolve("Night Light"), Some("ms-settings:nightlight"));
        assert_eq!(resolve("wi-fi"), Some("ms-settings:network-wifi"));
        assert_eq!(resolve("the moon"), None);
        assert_eq!(resolve(""), None);
        assert!(is_known("ms-settings:bluetooth"));
        assert!(!is_known("ms-settings:bluetooth --evil"));
        assert!(!is_known("cmd.exe"));
    }
}
