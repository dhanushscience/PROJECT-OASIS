//! # oasis-settings
//!
//! The `profiles.json` document the C# app wrote (schema v2). Typed where the Rust build needs
//! a field today; every other field is kept as raw JSON and written back unchanged, so
//! porting a section later never loses what a user already has, and the C# and Rust builds
//! can share one file while the port is in progress (DECISIONS D11).
//!
//! | Module | C# origin | Role |
//! |---|---|---|
//! | [`model`] | `SettingsSnapshot` (partly) | `Settings`, `Profile`, `ConfigProfile`, `Button` |
//! | [`migrate`] | `SettingsMigrationPipeline`, `V0ToV1…`, `V1ToV2…` | Older files → current schema |
//! | [`schema`] | `SettingsSchema`, `IntegrityLoadPolicy` | Constants and the persist rule |
//! | [`matcher`] | `Services/ProfileMatcher.cs` | Which profile and config apply to the foreground app |
//! | [`arbiter_bridge`] | `Services/ArbiterSettingsBridge.cs` | Gesture settings → the arbiter config |
//! | [`edit`] | new | Validated edits for a settings window (buttons, profile names and matches) |
//! | [`overlay`] | `OverlayAppearanceSettings`, `OverlayWindow` position | The overlay's look and saved position, checked for a web page |
//! | [`fields`] | new | The simple settings, each described once (label, place in the file, limits, default) |
//! | [`input`] | new | Which ways of pressing the buttons are on (pad, keyboard) and the keyboard keys |
//! | [`meetings`] | new | Meeting reminders and how early a meeting can be joined |
//! | [`quick_launch`] | `QuickLaunchModel.cs`, `QuickLaunchService.cs` (reading) | The Quick Launch setups |
//! | [`system`] | new | `StartWithWindows` and `RunAsAdministrator` |
//! | [`voice`] | `VoiceSettings` | The voice settings this build uses (push-to-talk, microphone, model) |
//! | [`voice_commands`] | `VoiceSettings`, `VoiceLexicon`, `IntentTrustedTargetStore` | Spoken-command settings, vocabulary, approved targets |
//! | [`youtube`] | `Services/OverlayService.cs` (rules) | YouTube and browser-bridge settings, when the YouTube gesture is live |
//! | `profile_ext` | `Services/ProfileExtensions.cs` | Read-only profile helpers and button → action conversion (private) |

pub mod arbiter_bridge;
pub mod edit;
pub mod fields;
pub mod input;
pub mod privacy;
pub mod matcher;
pub mod meetings;
pub mod migrate;
pub mod model;
pub mod overlay;
pub mod quick_launch;
pub mod system;
pub mod voice;
pub mod voice_commands;
pub mod youtube;
mod profile_ext;
pub mod schema;

pub use model::{Button, ConfigProfile, Profile, Settings};
pub use schema::{may_persist, CURRENT_SCHEMA};

#[derive(Debug)]
pub enum SettingsError {
    /// Not valid JSON, or not the expected shape.
    Parse(String),
    /// Written by a newer build. Refusing to load keeps us from rewriting it and dropping
    /// fields we do not know.
    TooNew { found: u32 },
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Parse(why) => write!(f, "Settings file could not be read: {why}"),
            SettingsError::TooNew { found } => write!(
                f,
                "Settings were saved by a newer OASIS (schema {found}); this build understands up to {CURRENT_SCHEMA}."
            ),
        }
    }
}

impl std::error::Error for SettingsError {}

/// Result of loading a file.
#[derive(Debug)]
pub struct Loaded {
    pub settings: Settings,
    /// The schema the file had before migration, if it was older than current.
    pub migrated_from: Option<u32>,
    /// A Google client secret that was sitting in the file. It is blanked in `settings`; the
    /// caller must put it in the secret store or it is lost.
    pub extracted_google_client_secret: Option<String>,
}

/// Parse and, if needed, migrate. A missing `SchemaVersion` means schema 0.
pub fn load(text: &str) -> Result<Loaded, SettingsError> {
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|e| SettingsError::Parse(e.to_string()))?;
    if !root.is_object() {
        return Err(SettingsError::Parse("the root must be a JSON object".into()));
    }
    let found = migrate::read_version(&root);
    if found > CURRENT_SCHEMA {
        return Err(SettingsError::TooNew { found });
    }
    let migrated = migrate::apply(root).map_err(SettingsError::Parse)?;
    let settings: Settings = serde_json::from_value(migrated.root)
        .map_err(|e| SettingsError::Parse(e.to_string()))?;
    Ok(Loaded {
        settings,
        migrated_from: (found < CURRENT_SCHEMA).then_some(found),
        extracted_google_client_secret: migrated.extracted_google_client_secret,
    })
}

/// Indented JSON, like the C# writer.
pub fn to_json(settings: &Settings) -> String {
    serde_json::to_string_pretty(settings).expect("settings are always serialisable")
}
