use crate::app_paths::AppPaths;
use crate::core::legacy_import::{export_legacy_ini, import_legacy_ini, QuickCmdsFile};
use crate::core::serial::SerialManager;
use crate::ipc::common::validate_output_path;
use tauri::State;

#[tauri::command]
pub fn import_legacy(path: String) -> Result<QuickCmdsFile, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {e}"))?;
    let name = std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "导入配置".into());
    Ok(import_legacy_ini(&content, &name))
}

#[tauri::command]
pub fn save_text_file(path: String, content: String) -> Result<(), String> {
    if content.len() > 20 * 1024 * 1024 {
        return Err("导出内容超过 20 MiB 限制".into());
    }
    let path = validate_output_path(&path, &["txt", "log", "html"])?;
    std::fs::write(path, content).map_err(|e| format!("写入失败: {e}"))
}

static QUICK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tauri::command]
pub fn save_quickcmds(paths: State<'_, AppPaths>, data: QuickCmdsFile) -> Result<(), String> {
    let _guard = QUICK_LOCK.lock().map_err(|_| "快捷指令保存锁不可用")?;
    crate::quickcmd_file::save(&paths.quickcmds_file(), &data)
}

#[tauri::command]
pub fn load_quickcmds(paths: State<'_, AppPaths>) -> Result<Option<QuickCmdsFile>, String> {
    let _guard = QUICK_LOCK.lock().map_err(|_| "快捷指令保存锁不可用")?;
    crate::quickcmd_file::load(&paths.quickcmds_file())
}

#[tauri::command]
pub fn restore_quickcmds(paths: State<'_, AppPaths>) -> Result<QuickCmdsFile, String> {
    let _guard = QUICK_LOCK.lock().map_err(|_| "快捷指令保存锁不可用")?;
    crate::quickcmd_file::restore(&paths.quickcmds_file())
}

#[tauri::command]
pub fn export_legacy(path: String, data: QuickCmdsFile) -> Result<(), String> {
    let ini = export_legacy_ini(&data);
    let path = validate_output_path(&path, &["ini"])?;
    std::fs::write(path, ini).map_err(|e| format!("写入失败: {e}"))
}

#[tauri::command]
pub fn export_session_log(
    manager: State<'_, SerialManager>,
    port: String,
    session_generation: u64,
    path: String,
) -> Result<usize, String> {
    let path = validate_output_path(&path, &["txt", "log"])?;
    let frames = manager
        .get_generation(&port, session_generation)?
        .history_snapshot();
    let mut output = String::new();
    for frame in &frames {
        let direction = if frame.dir == "tx" { ">>" } else { "<<" };
        let data = frame.hex.as_deref().unwrap_or(&frame.text);
        output.push_str(&format!("[{}] {} {}\n", frame.ts, direction, data));
    }
    std::fs::write(path, output).map_err(|e| format!("写入失败: {e}"))?;
    Ok(frames.len())
}
