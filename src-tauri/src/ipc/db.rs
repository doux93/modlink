//! 数据库持久化 Tauri 命令。前端通过这些命令读写 SQLite，而非 localStorage。
use crate::db::{new_id, Db, ExecutionRecord, Project};
use chrono::Utc;
use tauri::State;

#[tauri::command]
pub fn list_projects(db: State<'_, Db>) -> Result<Vec<Project>, String> {
    db.list_projects()
}

#[tauri::command]
pub fn get_project(db: State<'_, Db>, id: String) -> Result<Option<Project>, String> {
    db.get_project(&id)
}

#[tauri::command]
pub fn create_project(
    db: State<'_, Db>,
    name: String,
    customer: Option<String>,
    device_model: Option<String>,
    firmware: Option<String>,
    sim_env: Option<String>,
    notes: Option<String>,
) -> Result<Project, String> {
    let now = Utc::now().timestamp_millis();
    let project = Project {
        id: new_id(),
        name,
        customer,
        device_model,
        firmware,
        sim_env,
        notes,
        created_at: now,
        updated_at: now,
    };
    db.create_project(&project)?;
    Ok(project)
}

/// 查询最近的执行记录。limit 默认 200，与前端历史列表容量一致。
#[tauri::command]
pub fn list_executions(
    db: State<'_, Db>,
    limit: Option<u32>,
) -> Result<Vec<ExecutionRecord>, String> {
    db.list_executions(limit.unwrap_or(200))
}
