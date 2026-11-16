//! 应用层统一 AT 执行调度。
//!
//! `SerialManager` 负责物理串口与每端口互斥；本模块负责把来自控制台、AI、
//! 诊断和压测的执行请求统一成稳定的目标快照与一致的序列语义。

use crate::core::serial::{AtResult, SerialManager};
use crate::db::{Db, ExecutionRecord};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, Manager};

static EXECUTION_SEQ: AtomicU64 = AtomicU64::new(1);

/// 单个运行中任务的控制块。
pub struct TaskControl {
    pub cancel: Arc<AtomicBool>,
    pub kind: ExecutionKind,
}

/// 统一任务注册表：所有四类任务（AT序列/诊断/压测/场景压测）启动时注册，
/// stop_execution 只需查这一张表。
pub struct TaskRegistry(pub Mutex<HashMap<String, TaskControl>>);

impl TaskRegistry {
    pub fn register(
        &self,
        execution_id: String,
        kind: ExecutionKind,
    ) -> Result<Arc<AtomicBool>, String> {
        let mut tasks = self.0.lock();
        if tasks.contains_key(&execution_id) {
            return Err(format!("任务标识重复: {execution_id}"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        tasks.insert(
            execution_id,
            TaskControl {
                cancel: cancel.clone(),
                kind,
            },
        );
        Ok(cancel)
    }

    pub fn unregister(&self, execution_id: &str) {
        self.0.lock().remove(execution_id);
    }

    pub fn stop(&self, execution_id: &str) -> bool {
        if let Some(ctrl) = self.0.lock().get(execution_id) {
            let _ = ctrl.kind; // 审计用途：未来可按 kind 记录停止日志
            ctrl.cancel.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSource {
    Console,
    QuickCommand,
    ProfileDetection,
    Ai,
    Diagnostic,
    Stress,
    ScenarioStress,
}

/// 任务类型：区分四种执行器，与触发来源（source）正交。
/// source 表示"谁触发"（控制台/AI/型号探测），kind 表示"做什么"。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    AtSequence,
    Diagnostic,
    Stress,
    ScenarioStress,
}

/// 统一的任务完成摘要。不同 kind 填充不同字段，extra 放额外指标。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSummary {
    pub planned: u32,
    pub completed: u32,
    pub passed: u32,
    pub failed: u32,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
    /// 诊断的 overall、压测的 p95_latency_ms 等额外指标
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFailurePolicy {
    Continue,
    StopOnError,
}

impl Default for ExecutionFailurePolicy {
    fn default() -> Self {
        Self::Continue
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRisk {
    ReadOnly,
    StateChange,
    Destructive,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl ExecutionRisk {
    fn rank(&self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::StateChange => 1,
            Self::Unknown => 2,
            Self::Destructive => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionTarget {
    pub port: String,
    pub session_generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionStep {
    pub cmd: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub delay_ms: u64,
}

fn default_timeout_ms() -> u64 {
    8000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    #[serde(default)]
    pub execution_id: Option<String>,
    pub target: ExecutionTarget,
    pub source: ExecutionSource,
    /// AI 来源必须携带生成卡片时绑定的画像，用于后端执行前知识授权复核。
    #[serde(default)]
    pub profile_id: Option<String>,
    pub steps: Vec<ExecutionStep>,
    #[serde(default)]
    pub failure_policy: ExecutionFailurePolicy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReport {
    pub execution_id: String,
    pub target: ExecutionTarget,
    pub source: ExecutionSource,
    pub risk: ExecutionRisk,
    pub started_at: i64,
    pub elapsed_ms: u64,
    pub stopped_on_failure: bool,
    #[serde(default)]
    pub cancelled: bool,
    pub results: Vec<AtResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionLifecycleEvent {
    execution_id: String,
    kind: ExecutionKind,
    target: ExecutionTarget,
    source: ExecutionSource,
    risk: ExecutionRisk,
    status: ExecutionStatus,
    step_count: usize,
    started_at: i64,
    elapsed_ms: u64,
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<ExecutionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    results: Option<Vec<AtResult>>,
    #[serde(default)]
    results_truncated: bool,
}

pub struct ExecutionTrace {
    execution_id: String,
    kind: ExecutionKind,
    target: ExecutionTarget,
    source: ExecutionSource,
    risk: ExecutionRisk,
    step_count: usize,
    started_at: i64,
    started: Instant,
}

impl ExecutionTrace {
    pub fn start(
        app: &tauri::AppHandle,
        execution_id: String,
        kind: ExecutionKind,
        target: ExecutionTarget,
        source: ExecutionSource,
        risk: ExecutionRisk,
        step_count: usize,
    ) -> Self {
        let trace = Self {
            execution_id,
            kind,
            target,
            source,
            risk,
            step_count,
            started_at: chrono::Utc::now().timestamp_millis(),
            started: Instant::now(),
        };
        trace.emit(app, ExecutionStatus::Running, None, None, None, false);
        trace
    }

    pub fn started_at(&self) -> i64 {
        self.started_at
    }

    pub async fn finish(
        &self,
        app: &tauri::AppHandle,
        status: ExecutionStatus,
        error: Option<String>,
    ) {
        debug_assert!(status != ExecutionStatus::Running);
        self.finish_terminal(app, status, error, None, None).await;
    }

    pub async fn finish_with_summary(
        &self,
        app: &tauri::AppHandle,
        status: ExecutionStatus,
        error: Option<String>,
        summary: ExecutionSummary,
    ) {
        debug_assert!(status != ExecutionStatus::Running);
        self.finish_terminal(app, status, error, Some(summary), None)
            .await;
    }

    pub async fn finish_with_summary_and_results(
        &self,
        app: &tauri::AppHandle,
        status: ExecutionStatus,
        error: Option<String>,
        summary: ExecutionSummary,
        results: Vec<AtResult>,
    ) {
        debug_assert!(status != ExecutionStatus::Running);
        self.finish_terminal(app, status, error, Some(summary), Some(results))
            .await;
    }

    /// Persist the complete terminal snapshot once, before emitting a bounded UI event.
    /// All executors (including failures and cancellation) share this path.
    async fn finish_terminal(
        &self,
        app: &tauri::AppHandle,
        status: ExecutionStatus,
        error: Option<String>,
        summary: Option<ExecutionSummary>,
        results: Option<Vec<AtResult>>,
    ) {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        if let Some(db) = app.try_state::<Db>() {
            let db = db.inner().clone();
            let record = ExecutionRecord {
                id: self.execution_id.clone(),
                session_id: None,
                project_id: None,
                source: enum_name(&self.source),
                kind: enum_name(&self.kind),
                status: enum_name(&status),
                started_at: self.started_at,
                finished_at: self.started_at + elapsed_ms as i64,
                elapsed_ms,
                step_count: self.step_count,
                port: Some(self.target.port.clone()),
                summary: summary
                    .as_ref()
                    .and_then(|value| serde_json::to_value(value).ok()),
                results: results
                    .as_ref()
                    .and_then(|value| serde_json::to_value(value).ok()),
                results_truncated: false,
                error: error.clone(),
            };
            if let Err(error) =
                tauri::async_runtime::spawn_blocking(move || db.save_execution(&record))
                    .await
                    .unwrap_or_else(|error| Err(error.to_string()))
            {
                eprintln!("执行记录保存失败: {error}");
                let _ = app.emit("execution://persistence-error", error);
            }
        }
        let (results, truncated) = match results {
            Some(results) => {
                let (capped, truncated) = cap_results(results);
                (Some(capped), truncated)
            }
            None => (None, false),
        };
        self.emit(app, status, error, summary, results, truncated);
    }

    fn emit(
        &self,
        app: &tauri::AppHandle,
        status: ExecutionStatus,
        error: Option<String>,
        summary: Option<ExecutionSummary>,
        results: Option<Vec<AtResult>>,
        results_truncated: bool,
    ) {
        let _ = app.emit(
            "execution://lifecycle",
            ExecutionLifecycleEvent {
                execution_id: self.execution_id.clone(),
                kind: self.kind.clone(),
                target: self.target.clone(),
                source: self.source.clone(),
                risk: self.risk.clone(),
                status,
                step_count: self.step_count,
                started_at: self.started_at,
                elapsed_ms: self.started.elapsed().as_millis() as u64,
                error,
                summary,
                results,
                results_truncated,
            },
        );
    }
}

fn enum_name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

/// 限制推送到 WebView 的结果体积：最多 50 条结果，每条最多 20 行，单行最多 512 字节。
const MAX_RESULT_ROWS: usize = 50;
const MAX_LINES_PER_RESULT: usize = 20;
const MAX_LINE_BYTES: usize = 512;
/// 最大总字节数（约 256 KiB）。
const MAX_TOTAL_BYTES: usize = 256 * 1024;

fn cap_results(mut results: Vec<AtResult>) -> (Vec<AtResult>, bool) {
    let mut truncated = false;
    let mut total_bytes = 0usize;

    if results.len() > MAX_RESULT_ROWS {
        results.truncate(MAX_RESULT_ROWS);
        truncated = true;
    }

    for r in &mut results {
        if r.lines.len() > MAX_LINES_PER_RESULT {
            r.lines.truncate(MAX_LINES_PER_RESULT);
            truncated = true;
        }
        for line in &mut r.lines {
            let line_bytes = line.len();
            if line_bytes > MAX_LINE_BYTES {
                // 向前回退到最近的 UTF-8 字符边界，避免在多字节字符中间截断导致 panic
                let mut end = MAX_LINE_BYTES;
                while end > 0 && !line.is_char_boundary(end) {
                    end -= 1;
                }
                line.truncate(end);
                line.push_str("…(截断)");
                truncated = true;
            }
            total_bytes += line.len();
        }
        total_bytes += r.request.len() + r.final_code.len();
    }

    if total_bytes > MAX_TOTAL_BYTES {
        truncated = true;
        // 逐条移除尾部直到总量达标
        while total_bytes > MAX_TOTAL_BYTES && results.len() > 1 {
            let removed = results.pop().unwrap();
            total_bytes = total_bytes.saturating_sub(
                removed.request.len()
                    + removed.final_code.len()
                    + removed.lines.iter().map(|l| l.len()).sum::<usize>(),
            );
        }
    }

    (results, truncated)
}

/// 轮询取消标志，用于 tokio::select! 中可中断等待。公共版本供 lib.rs 复用。
pub async fn wait_cancel(cancel: &Arc<AtomicBool>) {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

pub struct ExecutionDispatcher;

impl ExecutionDispatcher {
    pub async fn execute(
        app: &tauri::AppHandle,
        manager: &SerialManager,
        runs: &TaskRegistry,
        request: ExecutionRequest,
    ) -> Result<ExecutionReport, String> {
        validate_request(&request)?;
        let risk = request
            .steps
            .iter()
            .map(|step| classify_at_command(&step.cmd))
            .max_by_key(ExecutionRisk::rank)
            .unwrap_or(ExecutionRisk::Unknown);
        let execution_id = request.execution_id.clone().unwrap_or_else(|| {
            format!(
                "exec-{}-{}",
                chrono::Utc::now().timestamp_millis(),
                EXECUTION_SEQ.fetch_add(1, Ordering::Relaxed)
            )
        });
        let target = request.target.clone();
        let source = request.source.clone();
        let step_count = request.steps.len();
        let cancel = runs.register(execution_id.clone(), ExecutionKind::AtSequence)?;
        let trace = ExecutionTrace::start(
            app,
            execution_id.clone(),
            ExecutionKind::AtSequence,
            target.clone(),
            source.clone(),
            risk.clone(),
            step_count,
        );

        let result = Self::execute_inner(app, manager, request, &cancel).await;

        runs.unregister(&execution_id);

        let (results, stopped_on_failure, cancelled) = match result {
            Ok(value) => value,
            Err(error) => {
                trace
                    .finish(app, ExecutionStatus::Failed, Some(error.clone()))
                    .await;
                return Err(error);
            }
        };
        if cancelled {
            let summary = ExecutionSummary {
                planned: step_count as u32,
                completed: results.len() as u32,
                passed: results.iter().filter(|r| r.final_code == "OK").count() as u32,
                failed: results.iter().filter(|r| r.final_code != "OK").count() as u32,
                cancelled: true,
                stopped_on_failure,
                extra: None,
            };
            trace
                .finish_with_summary_and_results(
                    app,
                    ExecutionStatus::Cancelled,
                    None,
                    summary,
                    results.clone(),
                )
                .await;
            return Ok(ExecutionReport {
                execution_id,
                target,
                source,
                risk,
                started_at: trace.started_at(),
                elapsed_ms: trace.started.elapsed().as_millis() as u64,
                stopped_on_failure,
                cancelled: true,
                results,
            });
        }
        let failed_code = results
            .iter()
            .find(|result| result.final_code != "OK")
            .map(|result| result.final_code.clone());
        let status = if failed_code.is_some() {
            ExecutionStatus::Failed
        } else {
            ExecutionStatus::Succeeded
        };
        let summary = ExecutionSummary {
            planned: step_count as u32,
            completed: results.len() as u32,
            passed: results.iter().filter(|r| r.final_code == "OK").count() as u32,
            failed: results.iter().filter(|r| r.final_code != "OK").count() as u32,
            cancelled: false,
            stopped_on_failure,
            extra: None,
        };
        trace
            .finish_with_summary_and_results(
                app,
                status,
                failed_code.clone(),
                summary,
                results.clone(),
            )
            .await;
        Ok(ExecutionReport {
            execution_id,
            target,
            source,
            risk,
            started_at: trace.started_at(),
            elapsed_ms: trace.started.elapsed().as_millis() as u64,
            stopped_on_failure,
            cancelled: false,
            results,
        })
    }

    async fn execute_inner(
        app: &tauri::AppHandle,
        manager: &SerialManager,
        request: ExecutionRequest,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(Vec<AtResult>, bool, bool), String> {
        let session =
            manager.get_generation(&request.target.port, request.target.session_generation)?;
        let mut cancelled = false;
        // 排队等待串口锁期间也允许取消
        let _exclusive = tokio::select! {
            guard = session.acquire_exclusive() => guard,
            _ = wait_cancel(cancel) => {
                return Ok((Vec::new(), false, true));
            }
        };
        let mut results = Vec::with_capacity(request.steps.len());
        let mut stopped_on_failure = false;

        for step in request.steps {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let result = session
                .send_at_locked(app, step.cmd.trim(), step.timeout_ms.clamp(100, 120_000))
                .await?;
            let failed = result.final_code != "OK";
            let channel_lost = matches!(result.final_code.as_str(), "TIMEOUT" | "DISCONNECTED");
            results.push(result);
            // 指令完成后立即复查取消：用户在单条指令执行期间点停止也能及时收敛
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            if channel_lost
                || (failed && request.failure_policy == ExecutionFailurePolicy::StopOnError)
            {
                stopped_on_failure = true;
                break;
            }
            if step.delay_ms > 0 {
                // 可中断延时：取消信号立即唤醒，无需等到延时结束
                let delay = std::time::Duration::from_millis(step.delay_ms.min(60_000));
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = wait_cancel(cancel) => {
                        cancelled = true;
                        break;
                    }
                }
            }
        }
        Ok((results, stopped_on_failure, cancelled))
    }
}

pub use modulink_core::at_protocol::validate_at_command;

pub fn classify_at_command(cmd: &str) -> ExecutionRisk {
    let upper = cmd.trim().to_ascii_uppercase().replace(' ', "");
    if !upper.starts_with("AT") {
        return ExecutionRisk::Unknown;
    }
    const DESTRUCTIVE_MARKERS: &[&str] = &[
        "AT&F",
        "FACTORY",
        "FORMAT",
        "ERASE",
        "+MRST",
        "+CRST",
        "+CFUN=0",
        "+CFUN=1,1",
    ];
    if DESTRUCTIVE_MARKERS
        .iter()
        .any(|marker| upper.contains(marker))
    {
        return ExecutionRisk::Destructive;
    }
    const READ_ONLY_COMMANDS: &[&str] = &[
        "AT", "ATI", "AT+CSQ", "AT+CGMM", "AT+CGMR", "AT+CGSN", "AT+CIMI",
    ];
    if upper.ends_with('?') || READ_ONLY_COMMANDS.contains(&upper.as_str()) {
        ExecutionRisk::ReadOnly
    } else {
        ExecutionRisk::StateChange
    }
}

pub fn risk_from_execution_mode(mode: &str) -> ExecutionRisk {
    match mode {
        "readonly" => ExecutionRisk::ReadOnly,
        "mutating" => ExecutionRisk::StateChange,
        _ => ExecutionRisk::Unknown,
    }
}

fn validate_request(request: &ExecutionRequest) -> Result<(), String> {
    if request
        .execution_id
        .as_ref()
        .is_some_and(|id| id.is_empty() || id.len() > 128)
    {
        return Err("无效的执行标识".into());
    }
    if request.target.port.trim().is_empty() || request.target.session_generation == 0 {
        return Err("执行目标无效".into());
    }
    if request.steps.is_empty() || request.steps.len() > 100 {
        return Err("AT 序列长度必须在 1-100 之间".into());
    }
    for step in &request.steps {
        validate_at_command(&step.cmd)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(cmd: &str) -> ExecutionRequest {
        ExecutionRequest {
            execution_id: None,
            target: ExecutionTarget {
                port: "COM10".into(),
                session_generation: 1,
            },
            source: ExecutionSource::Console,
            profile_id: None,
            steps: vec![ExecutionStep {
                cmd: cmd.into(),
                timeout_ms: 1000,
                delay_ms: 0,
            }],
            failure_policy: ExecutionFailurePolicy::Continue,
        }
    }

    #[test]
    fn rejects_multi_command_injection() {
        assert!(validate_request(&request("AT+CSQ\r\nAT&F")).is_err());
        assert!(validate_request(&request("AT\0AT&F")).is_err());
    }

    #[test]
    fn accepts_normal_at_commands() {
        assert!(validate_request(&request("AT")).is_ok());
        assert!(validate_request(&request("AT+CGDCONT=1,\"IP\",\"cmnet\"")).is_ok());
    }

    #[test]
    fn classifies_without_treating_all_at_as_dangerous() {
        assert_eq!(classify_at_command("AT+CSQ"), ExecutionRisk::ReadOnly);
        assert_eq!(
            classify_at_command("AT+CGACT=1,1"),
            ExecutionRisk::StateChange
        );
        assert_eq!(
            classify_at_command("AT+CFUN=1,1"),
            ExecutionRisk::Destructive
        );
    }
}
