use crate::core::serial::SerialManager;
use crate::ipc::common::validate_output_path;
use crate::types::Knowledge;
use modulink_core::log_analysis::{
    analyze_at_log, analyze_raw_log, parse_exported_log, LogAnalysisReport, LogFrame,
};
use tauri::State;

#[tauri::command]
pub async fn analyze_log(
    manager: State<'_, SerialManager>,
    knowledge: State<'_, Knowledge>,
    port: String,
    log_port: Option<String>,
    file_path: Option<String>,
) -> Result<LogAnalysisReport, String> {
    let (error_codes, log_patterns) = {
        let knowledge = knowledge.0.read();
        (
            knowledge.error_codes.clone(),
            knowledge.log_patterns.clone(),
        )
    };

    // 采集数据放在 async 端（快），CPU 密集的分析放 spawn_blocking（不阻塞 runtime）
    let at_frames: Vec<LogFrame> = if let Some(path) = &file_path {
        let path = validate_output_path(path, &["txt", "log"])?;
        let content = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))?;
        parse_exported_log(&content)
    } else {
        let session = manager.get(&port)?;
        session
            .history_snapshot()
            .into_iter()
            .map(|f| LogFrame {
                ts: f.ts,
                dir: f.dir,
                text: f.text,
            })
            .collect()
    };

    let raw_frames: Option<Vec<LogFrame>> = if file_path.is_none() {
        log_port.and_then(|lp| {
            manager.get(&lp).ok().map(|session| {
                session
                    .history_snapshot()
                    .into_iter()
                    .map(|f| LogFrame {
                        ts: f.ts,
                        dir: f.dir,
                        text: f.text,
                    })
                    .collect()
            })
        })
    } else {
        None
    };

    // CPU 密集分析在阻塞线程执行，避免卡住 async runtime 导致其他命令（如关串口）无响应
    let report = tauri::async_runtime::spawn_blocking(move || {
        let lookup =
            |code_type: &str, code: u32| -> Option<modulink_core::knowledge::ErrorCodeEntry> {
                error_codes.get(&format!("{code_type}:{code}")).cloned()
            };
        let mut report = LogAnalysisReport {
            at: None,
            raw: None,
        };
        if !at_frames.is_empty() {
            report.at = Some(analyze_at_log(&at_frames, &lookup));
        }
        if let Some(rf) = raw_frames {
            if !rf.is_empty() {
                report.raw = Some(analyze_raw_log(&rf, &log_patterns));
            }
        }
        report
    })
    .await
    .map_err(|e| format!("分析任务异常: {e}"))?;

    Ok(report)
}
