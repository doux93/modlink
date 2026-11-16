use crate::app_paths::AppPaths;
use crate::capability_evidence;
use crate::core::execution::{
    ExecutionDispatcher, ExecutionReport, ExecutionRequest, ExecutionSource, TaskRegistry,
};
use crate::core::serial::SerialManager;
use crate::knowledge::{
    authorize_at_commands_blocking, KnowledgeSnapshot, ManualIndex, PackValidationCache,
};
use crate::types::Knowledge;
use tauri::State;

#[tauri::command]
pub async fn execute_at(
    app: tauri::AppHandle,
    manager: State<'_, SerialManager>,
    runs: State<'_, TaskRegistry>,
    paths: State<'_, AppPaths>,
    manual_state: State<'_, ManualIndex>,
    pack_cache: State<'_, PackValidationCache>,
    knowledge: State<'_, Knowledge>,
    request: ExecutionRequest,
) -> Result<ExecutionReport, String> {
    let evidence_paths = paths.inner().clone();
    if request.source == ExecutionSource::Ai {
        let snapshot = KnowledgeSnapshot::capture(&knowledge, request.profile_id.as_deref());
        let (auth_paths, auth_state, auth_cache) = (
            paths.inner().clone(),
            manual_state.inner().clone(),
            pack_cache.inner().clone(),
        );
        let commands: Vec<String> = request.steps.iter().map(|step| step.cmd.clone()).collect();
        let port = Some(request.target.port.clone());
        let decisions = tauri::async_runtime::spawn_blocking(move || {
            authorize_at_commands_blocking(
                &auth_paths,
                &auth_state,
                &auth_cache,
                &snapshot,
                commands,
                port,
            )
        })
        .await
        // 授权本身失败必须挡住执行，不能因为查不出结论就放行。
        .map_err(|error| format!("AI 指令执行授权失败：授权检查未能完成（{error}）"))?;
        let denied: Vec<String> = decisions
            .into_iter()
            .filter(|decision| !decision.authorized)
            .map(|decision| format!("{}（{}）", decision.command, decision.reason))
            .collect();
        if !denied.is_empty() {
            return Err(format!("AI 指令执行授权失败：{}", denied.join("；")));
        }
    }
    let evidence_request = request.clone();
    let report = ExecutionDispatcher::execute(&app, &manager, &runs, request).await?;
    // 指令已经发出且拿到了响应，记账只是旁路：写盘失败不能把成功的执行报成失败。
    let record_report = report.clone();
    if let Err(error) = tauri::async_runtime::spawn_blocking(move || {
        capability_evidence::record_execution(&evidence_paths, &evidence_request, &record_report)
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()))
    {
        eprintln!("实测能力记录写入失败（不影响本次执行）: {error}");
    }
    Ok(report)
}

#[tauri::command]
pub fn stop_execution(tasks: State<'_, TaskRegistry>, execution_id: String) -> bool {
    tasks.stop(&execution_id)
}
