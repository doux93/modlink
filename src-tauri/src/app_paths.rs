use std::path::{Path, PathBuf};
use tauri::Manager;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub resource_data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub user_data_dir: PathBuf,
    pub legacy_data_dir: Option<PathBuf>,
}

impl AppPaths {
    pub fn resolve(app: &tauri::AppHandle) -> Result<Self, String> {
        let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
        let cwd = std::env::current_dir().unwrap_or_default();
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let candidates = [
            resource_dir.join("data"),
            resource_dir.join("_up_").join("data"),
            cwd.join("data"),
            cwd.join("../data"),
            exe_dir.clone().unwrap_or_default().join("../../../data"),
            exe_dir.clone().unwrap_or_default().join("data"),
        ];
        let resource_data_dir = candidates
            .iter()
            .find(|p| p.join("profiles").is_dir())
            .cloned()
            .ok_or_else(|| "未找到 ModuLink 数据资源目录".to_string())?;
        let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
        let user_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&user_data_dir).map_err(|e| e.to_string())?;
        let legacy_data_dir = candidates
            .iter()
            .find(|p| {
                p.join("settings.json").is_file() || p.join("quickcmds/current.json").is_file()
            })
            .cloned();
        Ok(Self {
            resource_data_dir,
            config_dir,
            user_data_dir,
            legacy_data_dir,
        })
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn quickcmds_file(&self) -> PathBuf {
        self.user_data_dir.join("quickcmds").join("current.json")
    }

    pub fn capability_evidence_file(&self) -> PathBuf {
        self.user_data_dir.join("capability-evidence.json")
    }

    /// SQLite 数据库文件路径。结构化数据落库于此，日志和大附件以文件形式存储并记录引用。
    pub fn db_file(&self) -> PathBuf {
        self.user_data_dir.join("modulink.db")
    }
}

pub fn migrate_legacy_user_data(paths: &AppPaths) -> Result<(), String> {
    let Some(legacy) = &paths.legacy_data_dir else {
        return Ok(());
    };
    let old_settings = legacy.join("settings.json");
    if old_settings.is_file() {
        let content = std::fs::read_to_string(&old_settings).map_err(|e| e.to_string())?;
        let mut settings: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| e.to_string())?;
        let key = settings
            .pointer("/ai/api_key")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        if let Some(key) = key {
            crate::credentials::set_api_key(&key)?;
        }
        if let Some(ai) = settings.get_mut("ai").and_then(|v| v.as_object_mut()) {
            ai.remove("api_key");
        }
        let sanitized = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        if !paths.settings_file().is_file() {
            std::fs::write(paths.settings_file(), &sanitized).map_err(|e| e.to_string())?;
        }
        // Remove the plaintext secret from the legacy file after the credential write succeeds.
        std::fs::write(old_settings, sanitized).map_err(|e| e.to_string())?;
    }
    let old_quickcmds = legacy.join("quickcmds").join("current.json");
    let new_quickcmds = paths.quickcmds_file();
    if old_quickcmds.is_file() && !new_quickcmds.is_file() {
        if let Some(parent) = new_quickcmds.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(old_quickcmds, new_quickcmds).map_err(|e| e.to_string())?;
    }
    Ok(())
}
