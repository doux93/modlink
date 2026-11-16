use crate::app_paths::AppPaths;
use crate::capability_evidence;
use crate::core::execution::{
    ExecutionDispatcher, ExecutionFailurePolicy, ExecutionRequest, ExecutionSource, ExecutionStep,
    ExecutionTarget, TaskRegistry,
};
use crate::core::serial::SerialManager;
use tauri::State;

/// 默认探测指令集：覆盖常见功能域
pub(crate) fn default_probe_commands() -> Vec<&'static str> {
    vec![
        "AT+CGMR",
        "AT+MQTTCFG=?",
        "AT+MQTTCONN=?",
        "AT+MQTTSUB=?",
        "AT+MQTTPUB=?",
        "AT+MFTPCONN=?",
        "AT+MFTPCFG=?",
        "AT+MFTPSTOR=?",
        "AT+MHTTPCFG=?",
        "AT+MHTTPREQUEST=?",
        "AT+MSSLCFG=?",
        "AT+MSSLCERTWR=?",
        "AT+MGPIO=?",
        "AT+MADC=?",
        "AT+CGDCONT=?",
        "AT+CGACT=?",
        "AT+COPS=?",
        "AT+CSQ=?",
    ]
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    command: String,
    final_code: String,
    response_lines: Vec<String>,
    recorded: bool,
}

#[tauri::command]
pub async fn probe_capabilities(
    app: tauri::AppHandle,
    manager: State<'_, SerialManager>,
    runs: State<'_, TaskRegistry>,
    paths: State<'_, AppPaths>,
    port: String,
    profile_id: String,
    commands: Option<Vec<String>>,
) -> Result<Vec<ProbeResult>, String> {
    let cmds = commands.unwrap_or_else(|| {
        default_probe_commands()
            .iter()
            .map(|s| s.to_string())
            .collect()
    });
    let session_generation = manager
        .open_ports()
        .into_iter()
        .find(|session| session.port.eq_ignore_ascii_case(&port))
        .map(|session| session.generation)
        .ok_or_else(|| format!("{port} 未打开，无法开始能力探测"))?;
    let target = ExecutionTarget {
        port: port.clone(),
        session_generation,
    };
    let mut results = Vec::with_capacity(cmds.len());
    for cmd in &cmds {
        let request = ExecutionRequest {
            execution_id: None,
            target: target.clone(),
            source: ExecutionSource::ProfileDetection,
            profile_id: Some(profile_id.clone()),
            steps: vec![ExecutionStep {
                cmd: cmd.clone(),
                timeout_ms: 5000,
                delay_ms: 300,
            }],
            failure_policy: ExecutionFailurePolicy::Continue,
        };
        let evidence_request = request.clone();
        let report = ExecutionDispatcher::execute(&app, &manager, &runs, request).await;
        match report {
            Ok(report) => {
                let evidence_paths = paths.inner().clone();
                let recorded = capability_evidence::record_execution(
                    &evidence_paths,
                    &evidence_request,
                    &report,
                )
                .unwrap_or(false);
                let result = report.results.into_iter().next();
                if let Some(r) = result {
                    results.push(ProbeResult {
                        command: cmd.clone(),
                        final_code: r.final_code,
                        response_lines: r.lines,
                        recorded,
                    });
                } else {
                    results.push(ProbeResult {
                        command: cmd.clone(),
                        final_code: "NO_RESULT".into(),
                        response_lines: vec![],
                        recorded: false,
                    });
                }
            }
            Err(e) => {
                results.push(ProbeResult {
                    command: cmd.clone(),
                    final_code: "ERROR".into(),
                    response_lines: vec![e],
                    recorded: false,
                });
            }
        }
    }
    Ok(results)
}

#[tauri::command]
pub fn list_capability_observations(
    paths: State<'_, AppPaths>,
    _profile_id: Option<String>,
    _port: Option<String>,
) -> capability_evidence::CapabilityEvidenceStore {
    capability_evidence::load(&paths)
}

#[tauri::command]
pub fn get_capability_evidence_context(
    paths: State<'_, AppPaths>,
    profile_id: Option<String>,
    port: Option<String>,
) -> String {
    capability_evidence::context(&paths, profile_id.as_deref(), port.as_deref())
}
