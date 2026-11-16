use crate::app_paths::AppPaths;
use crate::core::execution::{
    classify_at_command, risk_from_execution_mode, wait_cancel, ExecutionKind, ExecutionSource,
    ExecutionStatus, ExecutionSummary, ExecutionTarget, ExecutionTrace, TaskRegistry,
};
use crate::core::serial::SerialManager;
use crate::ipc::diagnostic::SessionExecutor;
use crate::knowledge::{
    authorize_flow_execution, KnowledgeSnapshot, ManualIndex, PackValidationCache,
};
use crate::types::Knowledge;
use modulink_core::stress::{
    run_stress_controlled, validate_config as validate_stress_config, StressConfig, StressReport,
    StressSample,
};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, State};

pub struct StressRuns(pub Mutex<HashMap<String, Arc<AtomicBool>>>);

#[derive(serde::Serialize, Clone)]
struct StressProgressEvent {
    run_id: String,
    port: String,
    completed: u32,
    passed: u32,
    failed: u32,
    sample: StressSample,
}

#[tauri::command]
pub async fn run_stress_test(
    app: tauri::AppHandle,
    manager: State<'_, SerialManager>,
    port: String,
    session_generation: u64,
    run_id: String,
    config: StressConfig,
    runs: State<'_, StressRuns>,
    tasks: State<'_, TaskRegistry>,
) -> Result<StressReport, String> {
    if run_id.is_empty() || run_id.len() > 128 {
        return Err("无效的压力测试运行标识".into());
    }
    validate_stress_config(&config)?;
    let session = manager.get_generation(&port, session_generation)?;
    let cancel = tasks.register(run_id.clone(), ExecutionKind::Stress)?;
    {
        let mut active = runs.0.lock();
        active.insert(run_id.clone(), cancel.clone());
    }

    let trace = ExecutionTrace::start(
        &app,
        run_id.clone(),
        ExecutionKind::Stress,
        ExecutionTarget {
            port: port.clone(),
            session_generation,
        },
        ExecutionSource::Stress,
        classify_at_command(&config.command),
        config.iterations as usize,
    );
    let exclusive = tokio::select! {
        guard = session.acquire_exclusive() => guard,
        _ = wait_cancel(&cancel) => {
            runs.0.lock().remove(&run_id);
            tasks.unregister(&run_id);
            let summary = ExecutionSummary {
                planned: config.iterations,
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
    let planned = config.iterations;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut exec = SessionExecutor {
            app,
            session,
            _exclusive: exclusive,
            armed_urc: None,
        };
        let mut last_emit = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_millis(100))
            .unwrap_or_else(std::time::Instant::now);
        run_stress_controlled(
            &config,
            &mut exec,
            &mut |sample, passed, failed| {
                let completed = passed + failed;
                if !sample.success || completed == planned || last_emit.elapsed().as_millis() >= 100
                {
                    let _ = app2.emit(
                        "stress://progress",
                        StressProgressEvent {
                            run_id: event_run_id.clone(),
                            port: event_port.clone(),
                            completed,
                            passed,
                            failed,
                            sample: sample.clone(),
                        },
                    );
                    last_emit = std::time::Instant::now();
                }
            },
            &mut || cancel.load(Ordering::Relaxed),
        )
    })
    .await;
    runs.0.lock().remove(&run_id);
    tasks.unregister(&run_id);
    let result = match result {
        Ok(result) => result,
        Err(error) => Err(error.to_string()),
    };
    match result {
        Ok(report) => {
            let (status, error) = if report.cancelled {
                (ExecutionStatus::Cancelled, None)
            } else if report.failed > 0 {
                (
                    ExecutionStatus::Failed,
                    Some(format!("压力测试失败 {} 次", report.failed)),
                )
            } else {
                (ExecutionStatus::Succeeded, None)
            };
            let summary = ExecutionSummary {
                planned: report.planned,
                completed: report.completed,
                passed: report.passed,
                failed: report.failed,
                cancelled: report.cancelled,
                stopped_on_failure: report.stopped_on_failure,
                extra: Some(serde_json::json!({
                    "p95_latency_ms": report.p95_latency_ms,
                    "avg_latency_ms": report.avg_latency_ms,
                })),
            };
            trace
                .finish_with_summary(&trace_app, status, error, summary)
                .await;
            Ok(report)
        }
        Err(error) => {
            trace
                .finish(&trace_app, ExecutionStatus::Failed, Some(error.clone()))
                .await;
            Err(error)
        }
    }
}

#[tauri::command]
pub fn cancel_stress_test(runs: State<'_, StressRuns>, run_id: String) -> bool {
    if let Some(cancel) = runs.0.lock().get(&run_id) {
        cancel.store(true, Ordering::Relaxed);
        true
    } else {
        false
    }
}

#[derive(serde::Serialize, Clone)]
struct ScenarioRoundEvent {
    run_id: String,
    port: String,
    round: modulink_core::stress::ScenarioRound,
    passed: u32,
    failed: u32,
}

#[tauri::command]
pub async fn run_scenario_stress_test(
    app: tauri::AppHandle,
    manager: State<'_, SerialManager>,
    knowledge: State<'_, Knowledge>,
    paths: State<'_, AppPaths>,
    manual_state: State<'_, ManualIndex>,
    pack_cache: State<'_, PackValidationCache>,
    port: String,
    session_generation: u64,
    run_id: String,
    flow_id: String,
    profile_id: Option<String>,
    allow_unverified: Option<bool>,
    config: modulink_core::stress::ScenarioStressConfig,
    runs: State<'_, StressRuns>,
    tasks: State<'_, TaskRegistry>,
) -> Result<modulink_core::stress::ScenarioStressReport, String> {
    if run_id.is_empty() || run_id.len() > 128 {
        return Err("invalid stress run id".into());
    }
    config.validate().map_err(|e| e.to_string())?;
    let flow = {
        let knowledge = knowledge.0.read();
        let flow = knowledge
            .flows
            .iter()
            .find(|f| f.id == flow_id)
            .cloned()
            .ok_or(format!("flow not found: {flow_id}"))?;
        if let Some(profile_id) = profile_id.as_deref() {
            let profile = knowledge
                .profile(profile_id)
                .ok_or_else(|| format!("profile not found: {profile_id}"))?;
            if !modulink_core::knowledge::flow_applies(&flow, Some(profile)) {
                return Err(format!(
                    "flow {} not applicable for profile {}",
                    flow.name, profile.name
                ));
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
        config.recovery_commands.clone().unwrap_or_default(),
    )
    .await?;
    let session = manager.get_generation(&port, session_generation)?;
    let cancel = tasks.register(run_id.clone(), ExecutionKind::ScenarioStress)?;
    {
        let mut active = runs.0.lock();
        active.insert(run_id.clone(), cancel.clone());
    }
    let trace = ExecutionTrace::start(
        &app,
        run_id.clone(),
        ExecutionKind::ScenarioStress,
        ExecutionTarget {
            port: port.clone(),
            session_generation,
        },
        ExecutionSource::ScenarioStress,
        risk_from_execution_mode(&flow.execution_mode),
        flow.steps.len(),
    );
    let exclusive = tokio::select! {
        guard = session.acquire_exclusive() => guard,
        _ = wait_cancel(&cancel) => {
            runs.0.lock().remove(&run_id);
            tasks.unregister(&run_id);
            let summary = ExecutionSummary {
                planned: config.iterations,
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
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut exec = SessionExecutor {
            app,
            session,
            _exclusive: exclusive,
            armed_urc: None,
        };
        // 长稳模式轮次极快时，每轮一次 IPC 会压垮前端。以 120ms 为节流窗口：
        // 期间到达的轮次被丢弃（相当于源头降采样），趋势图只需代表性采样点。
        let mut last_emit = std::time::Instant::now();
        let emit_interval = std::time::Duration::from_millis(120);
        modulink_core::stress::run_scenario_stress(
            &flow,
            &config,
            &mut exec,
            &mut |round, passed, failed| {
                let now = std::time::Instant::now();
                if now.duration_since(last_emit) < emit_interval {
                    return;
                }
                last_emit = now;
                let _ = app2.emit(
                    "stress://round",
                    ScenarioRoundEvent {
                        run_id: event_run_id.clone(),
                        port: event_port.clone(),
                        round: round.clone(),
                        passed,
                        failed,
                    },
                );
            },
            &mut || cancel.load(Ordering::Relaxed),
        )
    })
    .await;
    runs.0.lock().remove(&run_id);
    tasks.unregister(&run_id);
    let result = match result {
        Ok(result) => result,
        Err(error) => Err(error.to_string()),
    };
    match result {
        Ok(report) => {
            let (status, error) = if report.cancelled {
                (ExecutionStatus::Cancelled, None)
            } else if report.failed > 0 {
                (
                    ExecutionStatus::Failed,
                    Some(format!("场景压测失败 {} 轮", report.failed)),
                )
            } else {
                (ExecutionStatus::Succeeded, None)
            };
            let summary = ExecutionSummary {
                planned: report.planned,
                completed: report.completed,
                passed: report.passed,
                failed: report.failed,
                cancelled: report.cancelled,
                stopped_on_failure: report.stopped_on_failure,
                extra: None,
            };
            trace
                .finish_with_summary(&trace_app, status, error, summary)
                .await;
            Ok(report)
        }
        Err(error) => {
            trace
                .finish(&trace_app, ExecutionStatus::Failed, Some(error.clone()))
                .await;
            Err(error)
        }
    }
}
