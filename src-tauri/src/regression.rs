//! 真机回归测试：需要物理 AT 设备，默认 `cargo test -- --ignored` 运行。
//!
//! 通过 `REGRESSION_PORT` / `REGRESSION_PORT_2` 指定两口设备（默认 COM52 / COM113）。
//! 覆盖：执行 AT、断开重连代际失效、切换串口、取消压测。
#![cfg(test)]
use crate::core::execution::{
    ExecutionDispatcher, ExecutionFailurePolicy, ExecutionRequest, ExecutionSource, ExecutionStep,
    ExecutionTarget, TaskRegistry,
};
use crate::core::serial::SerialManager;
use crate::ipc::diagnostic::SessionExecutor;
use modulink_core::stress::{run_stress_controlled, StressConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Listener;

fn port() -> String {
    std::env::var("REGRESSION_PORT").unwrap_or_else(|_| "COM52".into())
}

fn port2() -> String {
    std::env::var("REGRESSION_PORT_2").unwrap_or_else(|_| "COM113".into())
}

fn app_handle() -> tauri::AppHandle {
    tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build app")
        .handle()
        .clone()
}

async fn open(manager: &SerialManager, app: &tauri::AppHandle, port: &str) -> u64 {
    manager
        .open(app.clone(), port, 115200)
        .unwrap_or_else(|e| panic!("打开 {port} 失败: {e}"))
        .generation
}

fn req(target: ExecutionTarget, cmds: &[&str], stop: bool) -> ExecutionRequest {
    ExecutionRequest {
        execution_id: None,
        target,
        source: ExecutionSource::Console,
        profile_id: None,
        steps: cmds
            .iter()
            .map(|c| ExecutionStep {
                cmd: c.to_string(),
                timeout_ms: 5000,
                delay_ms: 0,
            })
            .collect(),
        failure_policy: if stop {
            ExecutionFailurePolicy::StopOnError
        } else {
            ExecutionFailurePolicy::Continue
        },
    }
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LifecycleMirror {
    execution_id: String,
    status: String,
    step_count: usize,
    error: Option<String>,
}

fn collect_lifecycle(app: &tauri::AppHandle) -> Arc<Mutex<Vec<LifecycleMirror>>> {
    let store = Arc::new(Mutex::new(Vec::new()));
    let s = store.clone();
    app.listen("execution://lifecycle", move |event| {
        if let Ok(v) = serde_json::from_str::<LifecycleMirror>(event.payload()) {
            s.lock().unwrap().push(v);
        }
    });
    store
}

fn assert_lifecycle(
    store: &Arc<Mutex<Vec<LifecycleMirror>>>,
    execution_id: &str,
    final_status: &str,
) {
    let events = store.lock().unwrap();
    let running = events
        .iter()
        .find(|event| event.execution_id == execution_id && event.status == "running")
        .unwrap_or_else(|| panic!("缺少 running 事件: {events:?}"));
    assert!(running.step_count > 0, "执行规模应大于 0: {running:?}");
    assert!(
        running.error.is_none(),
        "running 事件不应携带错误: {running:?}"
    );

    let finished = events
        .iter()
        .find(|event| event.execution_id == execution_id && event.status == final_status)
        .unwrap_or_else(|| panic!("缺少 {final_status} 事件: {events:?}"));
    if final_status == "succeeded" {
        assert!(
            finished.error.is_none(),
            "成功事件不应携带错误: {finished:?}"
        );
    } else if final_status == "failed" {
        assert!(finished.error.is_some(), "失败事件应携带错误: {finished:?}");
    }
}

#[tokio::test]
#[ignore = "需要真实 AT 设备"]
async fn execute_at_basic_and_record_consistency() {
    let app = app_handle();
    let manager = SerialManager::default();
    let port = port();
    let gen = open(&manager, &app, &port).await;
    let runs = TaskRegistry(parking_lot::Mutex::new(std::collections::HashMap::new()));
    let store = collect_lifecycle(&app);

    let target = ExecutionTarget {
        port: port.clone(),
        session_generation: gen,
    };
    let report = ExecutionDispatcher::execute(
        &app,
        &manager,
        &runs,
        req(target.clone(), &["AT", "AT+CSQ"], false),
    )
    .await
    .expect("execute_at");

    assert_eq!(report.results.len(), 2, "应返回两条结果");
    assert_eq!(report.results[0].final_code, "OK", "AT 应回 OK");
    assert_eq!(report.results[1].final_code, "OK", "AT+CSQ 应回 OK");
    assert!(!report.stopped_on_failure);

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_lifecycle(&store, &report.execution_id, "succeeded");

    let failed = ExecutionDispatcher::execute(
        &app,
        &manager,
        &TaskRegistry(parking_lot::Mutex::new(std::collections::HashMap::new())),
        req(target, &["AT+ZZZNONEXIST=1"], false),
    )
    .await
    .expect("设备错误响应仍应返回执行报告");
    assert_ne!(failed.results[0].final_code, "OK");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_lifecycle(&store, &failed.execution_id, "failed");

    manager.close(&port).ok();
}

#[tokio::test]
#[ignore = "需要真实 AT 设备"]
async fn generation_invalidated_on_reconnect() {
    let app = app_handle();
    let manager = SerialManager::default();
    let port = port();

    let gen1 = open(&manager, &app, &port).await;
    manager.close(&port).expect("close");
    // 设备需要重新枚举/可打开；短暂等待端口释放
    tokio::time::sleep(Duration::from_millis(300)).await;
    let gen2 = open(&manager, &app, &port).await;
    assert_ne!(gen1, gen2, "重连后代际应自增");

    let stale = ExecutionTarget {
        port: port.clone(),
        session_generation: gen1,
    };
    let err = ExecutionDispatcher::execute(
        &app,
        &manager,
        &TaskRegistry(parking_lot::Mutex::new(std::collections::HashMap::new())),
        req(stale, &["AT"], false),
    )
    .await
    .expect_err("旧代际应被拒绝");
    assert!(err.contains("会话已变化"), "错误信息应说明代际失效: {err}");

    let fresh = ExecutionTarget {
        port: port.clone(),
        session_generation: gen2,
    };
    let report = ExecutionDispatcher::execute(
        &app,
        &manager,
        &TaskRegistry(parking_lot::Mutex::new(std::collections::HashMap::new())),
        req(fresh, &["AT"], false),
    )
    .await
    .expect("新代际应可用");
    assert_eq!(report.results[0].final_code, "OK");

    manager.close(&port).ok();
}

#[tokio::test]
#[ignore = "需要真实 AT 设备"]
async fn switch_ports_independent() {
    let app = app_handle();
    let manager = SerialManager::default();
    let p1 = port();
    let p2 = port2();
    let g1 = open(&manager, &app, &p1).await;
    let g2 = open(&manager, &app, &p2).await;
    let runs = TaskRegistry(parking_lot::Mutex::new(std::collections::HashMap::new()));
    assert_ne!(g1, g2, "两口代际应不同");

    let r1 = ExecutionDispatcher::execute(
        &app,
        &manager,
        &runs,
        req(
            ExecutionTarget {
                port: p1.clone(),
                session_generation: g1,
            },
            &["AT"],
            false,
        ),
    )
    .await
    .expect("p1 execute");
    let r2 = ExecutionDispatcher::execute(
        &app,
        &manager,
        &runs,
        req(
            ExecutionTarget {
                port: p2.clone(),
                session_generation: g2,
            },
            &["AT"],
            false,
        ),
    )
    .await
    .expect("p2 execute");
    assert_eq!(r1.results[0].final_code, "OK", "p1 应回 OK");
    assert_eq!(r2.results[0].final_code, "OK", "p2 应回 OK");

    manager.close(&p1).ok();
    manager.close(&p2).ok();
}

#[tokio::test]
#[ignore = "需要真实 AT 设备"]
async fn stress_cancel_releases_lock() {
    let app = app_handle();
    let manager = SerialManager::default();
    let port = port();
    let gen = open(&manager, &app, &port).await;
    let session = manager.get(&port).expect("get session");

    let cancel = Arc::new(AtomicBool::new(false));
    let exclusive = session.acquire_exclusive().await;
    let config = StressConfig {
        command: "AT".into(),
        iterations: 1000,
        interval_ms: 100,
        timeout_ms: 3000,
        expected_pattern: None,
        stop_on_failure: false,
    };

    let cancel2 = cancel.clone();
    let app_for_exec = app.clone();
    let session_clone = session.clone();
    let join = tokio::task::spawn_blocking(move || {
        let mut exec = SessionExecutor {
            app: app_for_exec,
            session: session_clone,
            _exclusive: exclusive,
            armed_urc: None,
        };
        run_stress_controlled(&config, &mut exec, &mut |_, _, _| {}, &mut || {
            cancel2.load(Ordering::Relaxed)
        })
    });

    tokio::time::sleep(Duration::from_millis(800)).await;
    cancel.store(true, Ordering::Relaxed);
    let report = join.await.expect("join").expect("stress report");
    assert!(report.cancelled, "压测应被取消");
    assert!(
        report.completed < report.planned,
        "取消后不应跑完全部: completed={} planned={}",
        report.completed,
        report.planned
    );

    drop(session);
    // 锁应已释放：execute_at 能继续获得互斥锁并下发指令
    let target = ExecutionTarget {
        port: port.clone(),
        session_generation: gen,
    };
    let r = ExecutionDispatcher::execute(
        &app,
        &manager,
        &TaskRegistry(parking_lot::Mutex::new(std::collections::HashMap::new())),
        req(target, &["AT"], false),
    )
    .await
    .expect("取消后 execute_at");
    assert_eq!(r.results[0].final_code, "OK", "取消后串口应可用");

    manager.close(&port).ok();
}
