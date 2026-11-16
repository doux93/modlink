use crate::app_paths::AppPaths;
use crate::config::{load_settings, merge_json};
use crate::credentials;
use tauri::State;

#[tauri::command]
pub fn get_settings(paths: State<'_, AppPaths>) -> serde_json::Value {
    let mut settings = load_settings(&paths);
    settings["ai"]["api_key_configured"] =
        serde_json::Value::Bool(matches!(credentials::get_api_key(), Ok(Some(_))));
    settings
}

#[tauri::command]
pub fn save_settings(
    paths: State<'_, AppPaths>,
    mut settings: serde_json::Value,
) -> Result<(), String> {
    if let Some(ai) = settings.get_mut("ai").and_then(|v| v.as_object_mut()) {
        if let Some(key) = ai.remove("api_key") {
            match key {
                serde_json::Value::String(value) if !value.is_empty() => {
                    credentials::set_api_key(&value)?;
                }
                serde_json::Value::Null => credentials::clear_api_key()?,
                _ => {}
            }
        }
        ai.remove("api_key_configured");
    }
    let mut merged = load_settings(&paths);
    merge_json(&mut merged, settings);
    let json = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
    let path = paths.settings_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建设置目录失败: {e}"))?;
    }
    std::fs::write(path, json).map_err(|e| format!("写入失败: {e}"))
}
