use crate::app_paths::AppPaths;
use crate::core::execution::{
    risk_from_execution_mode, validate_at_command, wait_cancel, ExecutionKind, ExecutionSource,
    ExecutionStatus, ExecutionSummary, ExecutionTarget, ExecutionTrace, TaskRegistry,
};
use crate::core::serial::{SerialManager, Session};
use crate::knowledge::{
    authorize_flow_execution, KnowledgeSnapshot, ManualIndex, PackValidationCache,
};
use crate::types::Knowledge;
use modulink_core::diag::{
    run_flow_controlled, AtExecutor, AtOutcome, DiagReport, StepReport, UrcPoll,
};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, State};

pub(crate) struct SessionExecutor {
    pub(crate) app: tauri::AppHandle,
    pub(crate) session: Arc<Session>,
    pub(crate) _exclusive: tokio::sync::OwnedMutexGuard<()>,
    pub(crate) armed_urc: Option<(u64, mpsc::Receiver<String>)>,
}

impl AtExecutor for SessionExecutor {
    fn execute(&mut self, cmd: &str, timeout_ms: u64) -> AtOutcome {
        if let Err(error) = validate_at_command(cmd) {
            return AtOutcome {
                lines: vec![error],
                final_code: "ERROR".into(),
            };
        }
        let r =
            tauri::async_runtime::block_on(self.session.send_at_locked(&self.app, cmd, timeout_ms));
        match r {
            Ok(res) => AtOutcome {
                lines: res.lines,
                final_code: res.final_code,
            },
            Err(e) => AtOutcome {
                lines: vec![e],
                final_code: "ERROR".into(),
            },
        }
    }

    fn arm_urc(&mut self, pattern: &str) {
        // 替换尚未清理的旧 waiter
        if let Some((id, _)) = self.armed_urc.take() {
            self.session.remove_urc_waiter(id);
        }
        self.armed_urc = self.session.arm_urc(pattern);
    }

    fn poll_urc(&mut self, slice_ms: u64) -> UrcPoll {
        let (id, rx) = match self.armed_urc.as_ref() {
            Some(v) => v,
            None => return UrcPoll::Gone,
        };
        // 先非阻塞 drain（URC 可能在 execute 期间已到达）
        match rx.try_recv() {
            Ok(line) => {
                self.armed_urc.take();
                UrcPoll::Got(line)
            }
            Err(mpsc::TryRecvError::Empty) => {
                match rx.recv_timeout(Duration::from_millis(slice_ms)) {
                    Ok(line) => {
                        self.armed_urc.take();
                        UrcPoll::Got(line)
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => UrcPoll::Pending,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        self.session.remove_urc_waiter(*id);
                        self.armed_urc.take();
                        UrcPoll::Gone
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.session.remove_urc_waiter(*id);
                self.armed_urc.take();
                UrcPoll::Gone
            }
        }
    }

    fn disarm_urc(&mut self) {
        if let Some((id, _)) = self.armed_urc.take() {
            self.session.remove_urc_waiter(id);
        }
    }
}

pub struct DiagnosticRuns(pub Mutex<HashMap<String, Arc<AtomicBool>>>);

#[derive(serde::Serialize, Clone)]
struct DiagnosticStepEvent {
    run_id: String,
    port: String,
    flow_id: String,
    step: StepReport,
}

/// 运行诊断流程；每步完成通过 `diag://step` 事件实时推送给前端
#[tauri::command]
pub async fn run_diagnostic(
    app: tauri::AppHandle,
    manager: State<'_, SerialManager>,
    knowledge: State<'_, Knowledge>,
    paths: State<'_, AppPaths>,
    manual_state: State<'_, ManualIndex>,
    pack_cache: State<'_, PackValidationCache>,
    port: String,
    session_generation: u64,
    flow_id: String,
    run_id: String,
    profile_id: Option<String>,
    allow_unverified: Option<bool>,
    runs: State<'_, DiagnosticRuns>,
    tasks: State<'_, TaskRegistry>,
) -> Result<DiagReport, String> {
    if run_id.is_empty() || run_id.len() > 128 {
        return Err("无效的诊断运行标识".into());
    }
    let flow = {
        let knowledge = knowledge.0.read();
        let flow = knowledge
            .flows
            .iter()
            .find(|f| f.id == flow_id)
            .cloned()
            .ok_or(format!("诊断流程不存在: {flow_id}"))?;
        if let Some(profile_id) = profile_id.as_deref() {
            let profile = knowledge
                .profile(profile_id)
                .ok_or_else(|| format!("模组画像不存在: {profile_id}"))?;
            // 适用性判定统一在 modulink-core::knowledge::flow_applies（前端下拉
            // 过滤经 list_applicable_flows 走同一实现，杜绝前后端双写不一致）
            if !modulink_core::knowledge::flow_applies(&flow, Some(profile)) {
                return Err(format!("流程 {} 不适用于画像 {}", flow.name, profile.name));
            }
        }
        flow
    };
    let snapshot = KnowledgeSnapshot::capture(&knowledge, profile_id.as_deref());
    let flow = authorize_flow_execution(
        paths.inner().clone(),
        manual_state.inner().clone(),
        pack_cache.inner().clone(),
        snapshot,
        flow,
        port.clone(),
        allow_unverified.unwrap_or(false),
        Vec::new(),
    )
    .await?;
    let session = manager.get_generation(&port, session_generation)?;
    let cancel = tasks.register(run_id.clone(), ExecutionKind::Diagnostic)?;
    {
        let mut active = runs.0.lock();
        active.insert(run_id.clone(), cancel.clone());
    }
    let trace = ExecutionTrace::start(
        &app,
        run_id.clone(),
        ExecutionKind::Diagnostic,
        ExecutionTarget {
            port: port.clone(),
            session_generation,
        },
        ExecutionSource::Diagnostic,
        risk_from_execution_mode(&flow.execution_mode),
        flow.steps.len(),
    );
    let exclusive = tokio::select! {
        guard = session.acquire_exclusive() => guard,
        _ = wait_cancel(&cancel) => {
            runs.0.lock().remove(&run_id);
            tasks.unregister(&run_id);
            let summary = ExecutionSummary {
                planned: flow.steps.len() as u32,
                completed: 0,
                passed: 0,
                failed: 0,
                cancelled: true,
                stopped_on_failure: false,
                extra: None,
            };
            trace.finish_with_summary(&app, ExecutionStatus::Cancelled, None, summary).await;
            return Err("CANCELLED".into());
        }
    };
    let trace_app = app.clone();
    let app2 = app.clone();
    let event_run_id = run_id.clone();
    let event_port = port.clone();
    let event_flow_id = flow_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut exec = SessionExecutor {
            app: app.clone(),
            session,
            _exclusive: exclusive,
            armed_urc: None,
        };
        run_flow_controlled(
            &flow,
            &mut exec,
            &mut |step| {
                let _ = app2.emit(
                    "diag://step",
                    DiagnosticStepEvent {
                        run_id: event_run_id.clone(),
                        port: event_port.clone(),
                        flow_id: event_flow_id.clone(),
                        step: step.clone(),
                    },
                );
            },
            &mut || cancel.load(Ordering::Relaxed),
        )
    })
    .await;
    runs.0.lock().remove(&run_id);
    tasks.unregister(&run_id);
    match result {
        Ok(report) => {
            let (status, error) = if report.cancelled {
                (ExecutionStatus::Cancelled, None)
            } else if report.overall == "fail" {
                (ExecutionStatus::Failed, Some("诊断结论：失败".into()))
            } else {
                (ExecutionStatus::Succeeded, None)
            };
            let summary = ExecutionSummary {
                planned: report.steps.len() as u32,
                completed: report.steps.iter().filter(|s| s.result != "skip").count() as u32,
                passed: report.steps.iter().filter(|s| s.result == "pass").count() as u32,
                failed: report.steps.iter().filter(|s| s.result == "fail").count() as u32,
                cancelled: report.cancelled,
                stopped_on_failure: false,
                extra: Some(serde_json::json!({ "overall": &report.overall })),
            };
            trace
                .finish_with_summary(&trace_app, status, error, summary)
                .await;
            Ok(report)
        }
        Err(error) => {
            let error = error.to_string();
            trace
                .finish(&trace_app, ExecutionStatus::Failed, Some(error.clone()))
                .await;
            Err(error)
        }
    }
}

#[tauri::command]
pub fn cancel_diagnostic(runs: State<'_, DiagnosticRuns>, run_id: String) -> bool {
    if let Some(cancel) = runs.0.lock().get(&run_id) {
        cancel.store(true, Ordering::Relaxed);
        true
    } else {
        false
    }
}
