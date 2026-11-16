use crate::app_paths::AppPaths;
use crate::knowledge::ManualIndex;
use crate::types::Knowledge;
use modulink_core::diag::{infer_execution_mode, validate_flow, DiagFlow};
use modulink_core::knowledge::KnowledgeStore;
use tauri::State;

#[tauri::command]
pub fn get_knowledge(knowledge: State<'_, Knowledge>) -> KnowledgeStore {
    knowledge.0.read().clone()
}

/// 返回对指定画像适用的流程 id 列表（前端下拉过滤用）。
/// 判定逻辑与 run_diagnostic 共用 modulink-core::knowledge::flow_applies，
/// 保证"能看到的一定能跑"。
#[tauri::command]
pub fn list_applicable_flows(
    knowledge: State<'_, Knowledge>,
    profile_id: Option<String>,
) -> Vec<String> {
    let knowledge = knowledge.0.read();
    let profile = profile_id.as_deref().and_then(|id| knowledge.profile(id));
    knowledge
        .flows
        .iter()
        .filter(|f| modulink_core::knowledge::flow_applies(f, profile))
        .map(|f| f.id.clone())
        .collect()
}

/// 热重载数据目录（FAE 改完 YAML 点一下刷新即生效——答辩演示点）
#[tauri::command]
pub fn reload_knowledge(
    paths: State<'_, AppPaths>,
    knowledge: State<'_, Knowledge>,
    manual_index: State<'_, ManualIndex>,
) -> KnowledgeStore {
    let store = KnowledgeStore::load_with_user_dir(&paths.resource_data_dir, &paths.user_data_dir);
    *knowledge.0.write() = store.clone();
    *manual_index.0.write() = None;
    store
}

/// 人工用例 id 校验：仅允许 my_ 前缀的安全字符，防止路径穿越。
fn usecase_filename(flow_id: &str) -> Result<std::path::PathBuf, String> {
    let safe = regex::Regex::new(r"^my_[A-Za-z0-9_-]+$").unwrap();
    if !safe.is_match(flow_id) {
        return Err(format!("usecase id must match my_* pattern: {flow_id}"));
    }
    Ok(std::path::PathBuf::from(format!("{flow_id}.yaml")))
}

#[tauri::command]
pub fn save_usecase(
    paths: State<'_, AppPaths>,
    knowledge: State<'_, Knowledge>,
    manual_index: State<'_, ManualIndex>,
    mut flow: DiagFlow,
) -> Result<(), String> {
    // 保存意味着流程内容可能已被修改，必须重新复核；影响范围由最终指令重新推断。
    flow.verified = false;
    flow.execution_mode = "unknown".into();
    flow.execution_mode = infer_execution_mode(&flow);
    let warnings = validate_flow(&flow);
    if !warnings.is_empty() {
        return Err(format!(
            "usecase validation failed: {}",
            warnings.join("; ")
        ));
    }
    let name = usecase_filename(&flow.id)?;
    // 用户用例存用户数据目录：安装目录可能只读，且 resource 副本会被
    // 构建前清理（clean-stale-resources）删除——绝不能放在那里
    let dir = paths.user_data_dir.join("usecases");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create dir failed: {e}"))?;
    let yaml = serde_yaml::to_string(&flow).map_err(|e| format!("serialize failed: {e}"))?;
    std::fs::write(dir.join(&name), yaml).map_err(|e| format!("write failed: {e}"))?;
    let store = KnowledgeStore::load_with_user_dir(&paths.resource_data_dir, &paths.user_data_dir);
    *knowledge.0.write() = store;
    *manual_index.0.write() = None;
    Ok(())
}

#[tauri::command]
pub fn delete_usecase(
    paths: State<'_, AppPaths>,
    knowledge: State<'_, Knowledge>,
    manual_index: State<'_, ManualIndex>,
    flow_id: String,
) -> Result<(), String> {
    let name = usecase_filename(&flow_id)?;
    let path = paths.user_data_dir.join("usecases").join(&name);
    if !path.is_file() {
        return Err(format!("usecase file not found: {flow_id}"));
    }
    std::fs::remove_file(&path).map_err(|e| format!("delete failed: {e}"))?;
    let store = KnowledgeStore::load_with_user_dir(&paths.resource_data_dir, &paths.user_data_dir);
    *knowledge.0.write() = store;
    *manual_index.0.write() = None;
    Ok(())
}
