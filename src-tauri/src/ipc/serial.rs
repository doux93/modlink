use crate::core::serial::{OpenPortInfo, PortInfo, SerialManager};
use tauri::State;

#[tauri::command]
pub fn list_serial_ports() -> Vec<PortInfo> {
    SerialManager::list_ports()
}

#[tauri::command]
pub fn open_serial(
    app: tauri::AppHandle,
    manager: State<'_, SerialManager>,
    port: String,
    baud: u32,
) -> Result<OpenPortInfo, String> {
    manager.open(app, &port, baud)
}

#[tauri::command]
pub fn close_serial(
    manager: State<'_, SerialManager>,
    port: String,
    session_generation: u64,
) -> Result<(), String> {
    manager.close_generation(&port, session_generation)
}

#[tauri::command]
pub fn open_serial_ports(manager: State<'_, SerialManager>) -> Vec<OpenPortInfo> {
    manager.open_ports()
}

#[tauri::command]
pub async fn serial_write(
    app: tauri::AppHandle,
    manager: State<'_, SerialManager>,
    port: String,
    session_generation: u64,
    data: String,
    is_hex: bool,
    append_newline: bool,
) -> Result<(), String> {
    manager
        .get_generation(&port, session_generation)?
        .write_raw(&app, &data, is_hex, append_newline)
        .await
}
