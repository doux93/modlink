mod app_paths;
mod capability_evidence;
mod config;
mod core;
mod credentials;
mod db;
mod ipc;
mod knowledge;
#[cfg(test)]
mod knowledge_caps_tests;
#[cfg(test)]
mod regression;
mod storage;
mod types;

use crate::app_paths::{migrate_legacy_user_data, AppPaths};
use crate::core::execution::TaskRegistry;
use crate::core::serial::SerialManager;
use crate::db::Db;
use crate::knowledge::{ManualIndex, PackValidationCache};
use crate::types::Knowledge;
use modulink_core::knowledge::KnowledgeStore;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(SerialManager::default())
        .manage(crate::core::ai_requests::AiRequests::default())
        .manage(crate::ipc::diagnostic::DiagnosticRuns(Mutex::new(
            HashMap::new(),
        )))
        .manage(crate::ipc::stress::StressRuns(Mutex::new(HashMap::new())))
        .manage(TaskRegistry(Mutex::new(HashMap::new())))
        .setup(|app| {
            let paths = AppPaths::resolve(app.handle()).map_err(|e| std::io::Error::other(e))?;
            migrate_legacy_user_data(&paths).map_err(|e| std::io::Error::other(e))?;
            let db = Db::open(&paths).map_err(|e| std::io::Error::other(e))?;
            let knowledge =
                KnowledgeStore::load_with_user_dir(&paths.resource_data_dir, &paths.user_data_dir);
            app.manage(paths);
            app.manage(db);
            app.manage(Knowledge(RwLock::new(knowledge)));
            app.manage(ManualIndex(std::sync::Arc::new(RwLock::new(None))));
            app.manage(PackValidationCache(std::sync::Arc::new(RwLock::new(None))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::ipc::serial::list_serial_ports,
            crate::ipc::serial::open_serial,
            crate::ipc::serial::close_serial,
            crate::ipc::serial::open_serial_ports,
            crate::ipc::serial::serial_write,
            crate::ipc::execution::execute_at,
            crate::ipc::execution::stop_execution,
            crate::ipc::legacy::import_legacy,
            crate::ipc::legacy::export_legacy,
            crate::ipc::legacy::save_text_file,
            crate::ipc::legacy::export_session_log,
            crate::ipc::knowledge_cmds::get_knowledge,
            crate::knowledge::capabilities::get_knowledge_capabilities,
            crate::knowledge::pack::validate_knowledge_pack,
            crate::knowledge::pack::validate_and_activate_knowledge_pack,
            crate::ipc::knowledge_cmds::list_applicable_flows,
            crate::ipc::knowledge_cmds::reload_knowledge,
            crate::ipc::legacy::save_quickcmds,
            crate::ipc::legacy::load_quickcmds,
            crate::ipc::diagnostic::run_diagnostic,
            crate::ipc::diagnostic::cancel_diagnostic,
            crate::ipc::stress::run_stress_test,
            crate::ipc::stress::cancel_stress_test,
            crate::ipc::knowledge_cmds::save_usecase,
            crate::ipc::knowledge_cmds::delete_usecase,
            crate::ipc::stress::run_scenario_stress_test,
            crate::ipc::settings::get_settings,
            crate::ipc::settings::save_settings,
            crate::ipc::ai::ai_chat,
            crate::ipc::ai::prepare_ai_request,
            crate::ipc::ai::cancel_ai_request,
            crate::knowledge::manual::search_manuals,
            crate::knowledge::authorization::authorize_at_commands,
            crate::ipc::capabilities::probe_capabilities,
            crate::ipc::capabilities::list_capability_observations,
            crate::ipc::capabilities::get_capability_evidence_context,
            crate::ipc::log_analysis::analyze_log,
            crate::ipc::db::list_projects,
            crate::ipc::db::get_project,
            crate::ipc::db::create_project,
            crate::ipc::db::list_executions
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
