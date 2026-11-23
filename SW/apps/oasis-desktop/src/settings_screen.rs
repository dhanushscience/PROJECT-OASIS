//! The Settings screen: its fields, changing one, reloading the file, and the browser extension folder.

use super::*;

/// Open the folder holding the browser extension, for "Load unpacked" in the browser.
#[tauri::command]
pub fn open_extension_folder(slot: State<'_, EngineSlot>) -> Result<(), String> {
    with_engine(&slot, |engine| {
        let folder = oasis_platform::extension_install::published_folder(engine.paths());
        if !folder.join("manifest.json").is_file() {
            return Err(
                "The extension has not been copied yet. Start OASIS with the browser bridge switched on, and keep the extensions folder next to the program.".to_string(),
            );
        }
        #[cfg(windows)]
        {
            oasis_win::shell::open(&folder.to_string_lossy(), None)
                .map_err(|code| format!("Windows could not open the folder (code {code})."))
        }
        #[cfg(not(windows))]
        {
            Err(format!("The extension is in {}", folder.display()))
        }
    })
}

/// Every simple setting, grouped in sections, with its current value, for the Settings tab.
#[derive(Serialize)]
pub(crate) struct FieldsReply {
    sections: Vec<oasis_settings::fields::SectionView>,
    /// Things worth knowing about the current choices (two roles on one button, …).
    warnings: Vec<String>,
}

pub(crate) fn field_context() -> oasis_settings::fields::FieldContext {
    oasis_settings::fields::FieldContext {
        microphones: oasis_voice::device_names(),
        speakers: oasis_voice::cue::output_names(),
        browsers: installed_browsers(),
    }
}

#[cfg(windows)]
pub(crate) fn installed_browsers() -> Vec<(String, String)> {
    use oasis_platform::windows::actions::{installed_browsers, BROWSERS};
    installed_browsers()
        .into_iter()
        .filter_map(|name| BROWSERS.iter().find(|(n, _)| *n == name).map(|(n, shown)| (n.to_string(), shown.to_string())))
        .collect()
}

#[cfg(not(windows))]
pub(crate) fn installed_browsers() -> Vec<(String, String)> {
    Vec::new()
}

#[tauri::command]
pub fn settings_fields(slot: State<'_, EngineSlot>) -> Result<FieldsReply, String> {
    with_engine(&slot, |engine| {
        let settings = engine.settings();
        Ok(FieldsReply {
            sections: oasis_settings::fields::sections(&settings, &field_context()),
            warnings: oasis_settings::fields::warnings(&settings),
        })
    })
}

/// Change one simple setting and save. The error text is meant for the person editing.
#[tauri::command]
pub fn set_setting(
    slot: State<'_, EngineSlot>,
    id: String,
    value: serde_json::Value,
) -> Result<FieldsReply, String> {
    with_engine(&slot, |engine| {
        engine
            .change(|s| oasis_settings::fields::set_field(s, &id, value, &field_context()))
            .map_err(|e| e.to_string())?;
        let settings = engine.settings();
        Ok(FieldsReply {
            sections: oasis_settings::fields::sections(&settings, &field_context()),
            warnings: oasis_settings::fields::warnings(&settings),
        })
    })
}

/// Put the overlay's colours and sizes back to the defaults.
#[tauri::command]
pub fn reset_overlay_look_setting(slot: State<'_, EngineSlot>) -> Result<FieldsReply, String> {
    with_engine(&slot, |engine| {
        engine
            .change(|s| {
                oasis_settings::fields::reset_overlay_look(s);
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        let settings = engine.settings();
        Ok(FieldsReply {
            sections: oasis_settings::fields::sections(&settings, &field_context()),
            warnings: oasis_settings::fields::warnings(&settings),
        })
    })
}

/// Read `profiles.json` again (after the C# app changed it, say).
#[tauri::command]
pub fn reload_settings(slot: State<'_, EngineSlot>) -> Result<(), String> {
    with_engine(&slot, |engine| engine.reload_settings())
}
