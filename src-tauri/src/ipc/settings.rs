use crate::app_paths::AppPaths;
use crate::config::{load_settings, merge_json, validate_settings};
use crate::credentials;
use tauri::State;

#[tauri::command]
pub fn get_settings(paths: State<'_, AppPaths>) -> Result<serde_json::Value, String> {
    let mut settings = load_settings(&paths.settings_file())?;
    settings["ai"]["api_key_configured"] =
        serde_json::Value::Bool(credentials::get_api_key()?.is_some_and(|key| !key.is_empty()));
    Ok(settings)
}

#[tauri::command]
pub fn save_settings(
    paths: State<'_, AppPaths>,
    settings: serde_json::Value,
) -> Result<(), String> {
    save_settings_value(&paths, settings)
}

pub(crate) fn save_settings_value(
    paths: &AppPaths,
    mut settings: serde_json::Value,
) -> Result<(), String> {
    static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SAVE_LOCK.lock().map_err(|_| "设置保存锁不可用")?;
    validate_settings(&settings)?;
    let mut key_change = None;
    if let Some(ai) = settings.get_mut("ai").and_then(|v| v.as_object_mut()) {
        if let Some(key) = ai.remove("api_key") {
            key_change = match key {
                serde_json::Value::String(value) if !value.is_empty() => Some(Some(value)),
                serde_json::Value::Null => Some(None),
                _ => None,
            };
        }
        ai.remove("api_key_configured");
    }
    let current = load_settings(&paths.settings_file())?;
    let mut merged = current.clone();
    merge_json(&mut merged, settings);
    let json = if merged != current {
        Some(serde_json::to_vec_pretty(&merged).map_err(|e| e.to_string())?)
    } else {
        None
    };
    crate::settings_commit::commit_settings(
        &paths.settings_file(),
        json.as_deref(),
        key_change,
        credentials::get_api_key,
        |key| match key {
            Some(value) => credentials::set_api_key(value),
            None => credentials::clear_api_key(),
        },
    )
}
