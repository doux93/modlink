//! Repeated AT command runner with latency and failure statistics.

use crate::diag::{AtExecutor, AtOutcome};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StressConfig {
    pub command: String,
    pub iterations: u32,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    #[serde(default)]
    pub expected_pattern: Option<String>,
    #[serde(default)]
    pub stop_on_failure: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StressSample {
    pub index: u32,
    pub success: bool,
    pub final_code: String,
    pub elapsed_ms: u64,
    pub lines: Vec<String>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StressReport {
    pub command: String,
    pub planned: u32,
    pub completed: u32,
    pub passed: u32,
    pub failed: u32,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
    pub elapsed_ms: u64,
    pub min_latency_ms: u64,
    pub max_latency_ms: u64,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: u64,
    pub failures: Vec<StressSample>,
    pub failures_truncated: bool,
}

pub fn validate_config(config: &StressConfig) -> Result<Option<Regex>, String> {
    if config.command.trim().is_empty() {
        return Err("AT 指令不能为空".into());
    }
    if config.command.contains('\n') || config.command.contains('\r') {
        return Err("压力测试每轮只允许发送一条 AT 指令".into());
    }
    if !(1..=100_000).contains(&config.iterations) {
        return Err("测试次数必须在 1 到 100000 之间".into());
    }
    if !(100..=120_000).contains(&config.timeout_ms) {
        return Err("单次超时必须在 100 到 120000 ms 之间".into());
    }
    if config.interval_ms > 60_000 {
        return Err("发送间隔不能超过 60000 ms".into());
    }
    config
        .expected_pattern
        .as_deref()
        .filter(|pattern| !pattern.trim().is_empty())
        .map(Regex::new)
        .transpose()
        .map_err(|e| format!("成功条件正则无效: {e}"))
}

fn evaluate(
    index: u32,
    outcome: AtOutcome,
    expected: Option<&Regex>,
    elapsed_ms: u64,
) -> StressSample {
    let final_ok = outcome.final_code == "OK";
    let response = outcome.lines.join("\n");
    let pattern_ok = expected.is_none_or(|pattern| pattern.is_match(&response));
    let success = final_ok && pattern_ok;
    let error = if !final_ok {
        format!("终结码为 {}", outcome.final_code)
    } else if !pattern_ok {
        "响应未匹配成功条件".into()
    } else {
        String::new()
    };
    StressSample {
        index,
        success,
        final_code: outcome.final_code,
        elapsed_ms,
        lines: outcome.lines,
        error,
    }
}

fn percentile95(values: &mut [u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let index = ((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    values[index]
}

pub fn run_stress_controlled(
    config: &StressConfig,
    exec: &mut dyn AtExecutor,
    on_sample: &mut dyn FnMut(&StressSample, u32, u32),
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<StressReport, String> {
    let expected = validate_config(config)?;
    let started = Instant::now();
    let mut passed = 0;
    let mut failed = 0;
    let mut cancelled = false;
    let mut stopped_on_failure = false;
    let mut latencies = Vec::with_capacity(config.iterations as usize);
    let mut failures = Vec::new();
    let mut failures_truncated = false;

    for index in 1..=config.iterations {
        if should_cancel() {
            cancelled = true;
            break;
        }
        let command_started = Instant::now();
        let outcome = exec.execute(config.command.trim(), config.timeout_ms);
        let sample = evaluate(
            index,
            outcome,
            expected.as_ref(),
            command_started.elapsed().as_millis() as u64,
        );
        latencies.push(sample.elapsed_ms);
        if sample.success {
            passed += 1;
        } else {
            failed += 1;
            if failures.len() < 100 {
                failures.push(sample.clone());
            } else {
                failures_truncated = true;
            }
        }
        on_sample(&sample, passed, failed);
        if !sample.success && config.stop_on_failure {
            stopped_on_failure = true;
            break;
        }

        if index < config.iterations && config.interval_ms > 0 {
            let mut remaining = config.interval_ms;
            while remaining > 0 {
                if should_cancel() {
                    cancelled = true;
                    break;
                }
                let slice = remaining.min(100);
                exec.sleep_ms(slice);
                remaining -= slice;
            }
            if cancelled {
                break;
            }
        }
    }

    let completed = passed + failed;
    let min_latency_ms = latencies.iter().copied().min().unwrap_or(0);
    let max_latency_ms = latencies.iter().copied().max().unwrap_or(0);
    let avg_latency_ms = if latencies.is_empty() {
        0.0
    } else {
        latencies.iter().sum::<u64>() as f64 / latencies.len() as f64
    };
    let p95_latency_ms = percentile95(&mut latencies);
    Ok(StressReport {
        command: config.command.trim().into(),
        planned: config.iterations,
        completed,
        passed,
        failed,
        cancelled,
        stopped_on_failure,
        elapsed_ms: started.elapsed().as_millis() as u64,
        min_latency_ms,
        max_latency_ms,
        avg_latency_ms,
        p95_latency_ms,
        failures,
        failures_truncated,
    })
}

// ---------------------------------------------------------------- 场景压测 (L2/L3)

use crate::diag::{DiagFlow, DiagReport};

/// 失败原因归一化枚举——失败聚类按 (step_id, error_kind) 分桶。
/// 归一化避免 "+CME ERROR: 148" 与 "+CME ERROR:  148" 被分成两类。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub enum StressErrorKind {
    Timeout,
    CmeError(String),
    CmsError(String),
    PlainError,
    UrcMissing,
    IoError,
    Other,
}

impl StressErrorKind {
    /// 从步骤结果（result + reason + raw）推断失败类别。
    fn from_step(step: &crate::diag::StepReport) -> Self {
        let combined = format!("{} {} {}", step.result, step.reason, step.raw.join(" "));
        let upper = combined.to_uppercase();
        if upper.contains("URC") || step.reason.contains("URC") {
            return StressErrorKind::UrcMissing;
        }
        if upper.contains("TIMEOUT") || step.reason.contains("超时") {
            return StressErrorKind::Timeout;
        }
        if let Some(code) = extract_error_code(&combined, "+CME ERROR") {
            return StressErrorKind::CmeError(code);
        }
        if let Some(code) = extract_error_code(&combined, "+CMS ERROR") {
            return StressErrorKind::CmsError(code);
        }
        if upper.contains("ERROR") {
            return StressErrorKind::PlainError;
        }
        StressErrorKind::Other
    }

    pub fn as_str(&self) -> String {
        match self {
            StressErrorKind::Timeout => "超时".into(),
            StressErrorKind::CmeError(c) => format!("CME错误({})", c),
            StressErrorKind::CmsError(c) => format!("CMS错误({})", c),
            StressErrorKind::PlainError => "ERROR".into(),
            StressErrorKind::UrcMissing => "URC未上报".into(),
            StressErrorKind::IoError => "串口IO错误".into(),
            StressErrorKind::Other => "其他".into(),
        }
    }
}

fn extract_error_code(text: &str, prefix: &str) -> Option<String> {
    let idx = text.to_uppercase().find(prefix)?;
    let rest = &text[idx + prefix.len()..];
    let rest = rest.trim_start_matches(|c: char| c == ' ' || c == ':' || c == '\t');
    let code: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if code.is_empty() {
        None
    } else {
        Some(code)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioStressConfig {
    pub iterations: u32,
    #[serde(default = "default_scenario_interval")]
    pub interval_ms: u64,
    /// continue | stop | recover（默认 recover）
    #[serde(default = "default_failure_policy")]
    pub failure_policy: String,
    /// None=自动推断关闭类指令；Some(vec)=手动指定；Some(空vec)=不恢复
    #[serde(default)]
    pub recovery_commands: Option<Vec<String>>,
    #[serde(default = "default_recovery_timeout")]
    pub recovery_timeout_ms: u64,
    /// warn 是否算成功（默认 true）
    #[serde(default = "default_warn_as_pass")]
    pub warn_as_pass: bool,
    /// 最大时长秒（0=按 iterations 终止；长稳模式用）
    #[serde(default)]
    pub max_duration_sec: u64,
}

fn default_scenario_interval() -> u64 {
    200
}
fn default_failure_policy() -> String {
    "recover".into()
}
fn default_recovery_timeout() -> u64 {
    2000
}
fn default_warn_as_pass() -> bool {
    true
}

impl ScenarioStressConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.iterations == 0 && self.max_duration_sec == 0 {
            return Err("iterations 和 max_duration_sec 不能同时为 0".into());
        }
        if self.iterations > 100_000 {
            return Err("测试次数不能超过 100000".into());
        }
        if self.interval_ms > 60_000 {
            return Err("轮间隔不能超过 60000 ms".into());
        }
        match self.failure_policy.as_str() {
            "continue" | "stop" | "recover" => Ok(()),
            other => Err(format!("未知失败策略: {other}")),
        }
    }
}

/// 单轮紧凑样本——成功轮只存聚合，失败轮存每步明细供聚类。
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioRound {
    pub index: u32,
    pub success: bool,
    pub overall: String,
    pub elapsed_ms: u64,
    pub steps: Vec<ScenarioStepSample>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioStepSample {
    pub id: String,
    pub title: String,
    pub result: String,
    pub final_code: String,
    pub elapsed_ms: u64,
    pub error_kind: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailureCluster {
    pub step_id: String,
    pub step_title: String,
    pub error_kind: String,
    pub count: u32,
    pub sample_rounds: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepLatencyStat {
    pub step_id: String,
    pub step_title: String,
    pub count: u32,
    pub min_ms: u64,
    pub max_ms: u64,
    pub avg_ms: f64,
    pub p95_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioStressReport {
    pub flow_id: String,
    pub flow_name: String,
    pub planned: u32,
    pub completed: u32,
    pub passed: u32,
    pub failed: u32,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
    pub time_limit_reached: bool,
    pub elapsed_ms: u64,
    pub min_latency_ms: u64,
    pub max_latency_ms: u64,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: u64,
    pub step_stats: Vec<StepLatencyStat>,
    pub failure_clusters: Vec<FailureCluster>,
    pub failures: Vec<ScenarioRound>,
    pub failures_truncated: bool,
    pub recovery_attempts: u32,
    pub recovery_failures: u32,
}

/// 自动推断恢复指令：取流程里最后一个关闭类指令。
pub fn infer_recovery_commands(flow: &DiagFlow) -> Vec<String> {
    let close_prefixes = [
        "AT+MIPCLOSE",
        "AT+NSOCL",
        "AT+MQTTDISC",
        "AT+MQTTCLOSE",
        "AT+CGACT=0",
        "AT+CIPCLOSE",
        "AT+HTTPTERM",
        "AT+FTPCLOSE",
    ];
    let mut found = Vec::new();
    for step in &flow.steps {
        for cmd in &step.send {
            let upper = cmd.trim().to_uppercase();
            if close_prefixes.iter().any(|p| upper.starts_with(p)) {
                found.push(cmd.trim().to_string());
            }
        }
    }
    found.into_iter().last().into_iter().collect()
}

fn run_recovery(commands: &[String], exec: &mut dyn AtExecutor, timeout_ms: u64) -> (u32, u32) {
    let mut attempts = 0u32;
    let mut failures = 0u32;
    for cmd in commands {
        if cmd.trim().is_empty() {
            continue;
        }
        attempts += 1;
        let outcome = exec.execute(cmd.trim(), timeout_ms);
        if outcome.final_code != "OK" {
            failures += 1;
        }
    }
    (attempts, failures)
}

fn percentile(values: &mut [u64], p: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let idx = ((values.len() as f64 * p).ceil() as usize).saturating_sub(1);
    values[idx.min(values.len() - 1)]
}

/// 场景压测主入口。复用 run_flow_controlled 跑每一轮，外层套循环 + 失败恢复 + 取消。
pub fn run_scenario_stress(
    flow: &DiagFlow,
    config: &ScenarioStressConfig,
    exec: &mut dyn AtExecutor,
    on_round: &mut dyn FnMut(&ScenarioRound, u32, u32),
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<ScenarioStressReport, String> {
    config.validate()?;
    let recovery: Vec<String> = match &config.recovery_commands {
        Some(cmds) => cmds.clone(),
        None => infer_recovery_commands(flow),
    };

    let started = Instant::now();
    let max_iterations = if config.iterations == 0 {
        u32::MAX
    } else {
        config.iterations
    };

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut cancelled = false;
    let mut stopped_on_failure = false;
    let mut time_limit_reached = false;
    let mut round_latencies: Vec<u64> = Vec::new();
    let mut failures: Vec<ScenarioRound> = Vec::new();
    let mut failures_truncated = false;
    let mut step_latencies: std::collections::HashMap<String, (String, Vec<u64>)> =
        std::collections::HashMap::new();
    let mut clusters: std::collections::HashMap<(String, String), (u32, Vec<u32>)> =
        std::collections::HashMap::new();
    let mut recovery_attempts = 0u32;
    let mut recovery_failures = 0u32;

    let mut index = 0u32;
    while index < max_iterations {
        if should_cancel() {
            cancelled = true;
            break;
        }
        if config.max_duration_sec > 0 && started.elapsed().as_secs() >= config.max_duration_sec {
            time_limit_reached = true;
            break;
        }
        index += 1;

        let round_started = Instant::now();
        let report: DiagReport =
            crate::diag::run_flow_controlled(flow, exec, &mut |_| {}, should_cancel);
        let round_elapsed = round_started.elapsed().as_millis() as u64;

        let success = report.overall == "pass" || (config.warn_as_pass && report.overall == "warn");

        for step in &report.steps {
            if step.result == "skip" {
                continue;
            }
            let entry = step_latencies
                .entry(step.id.clone())
                .or_insert_with(|| (step.title.clone(), Vec::new()));
            entry.1.push(step.elapsed_ms);
        }

        if !success {
            failed += 1;
            let mut step_samples = Vec::new();
            for step in &report.steps {
                if step.result != "fail" {
                    continue;
                }
                let kind = StressErrorKind::from_step(step);
                let final_code = step
                    .raw
                    .iter()
                    .rev()
                    .find(|l| {
                        let u = l.to_uppercase();
                        u == "OK"
                            || u == "ERROR"
                            || u.starts_with("+CME ERROR")
                            || u.starts_with("+CMS ERROR")
                            || u == "TIMEOUT"
                    })
                    .cloned()
                    .unwrap_or_default();
                let kind_str = match &kind {
                    StressErrorKind::Timeout => "Timeout".into(),
                    StressErrorKind::CmeError(c) => format!("CmeError({})", c),
                    StressErrorKind::CmsError(c) => format!("CmsError({})", c),
                    StressErrorKind::PlainError => "PlainError".into(),
                    StressErrorKind::UrcMissing => "UrcMissing".into(),
                    StressErrorKind::IoError => "IoError".into(),
                    StressErrorKind::Other => "Other".into(),
                };
                let key = (step.id.clone(), kind_str.clone());
                let entry = clusters.entry(key).or_insert((0, Vec::new()));
                entry.0 += 1;
                if entry.1.len() < 5 {
                    entry.1.push(index);
                }
                step_samples.push(ScenarioStepSample {
                    id: step.id.clone(),
                    title: step.title.clone(),
                    result: step.result.clone(),
                    final_code,
                    elapsed_ms: step.elapsed_ms,
                    error_kind: kind_str,
                    reason: step.reason.clone(),
                });
            }
            // 延迟统计独立于失败明细收集——明细截断(100条)后统计不失真
            round_latencies.push(round_elapsed);
            let round = ScenarioRound {
                index,
                success: false,
                overall: report.overall.clone(),
                elapsed_ms: round_elapsed,
                steps: step_samples,
            };
            if failures.len() < 100 {
                failures.push(round.clone());
            } else {
                failures_truncated = true;
            }
            on_round(&round, passed, failed);

            if config.failure_policy == "recover" && !recovery.is_empty() {
                let (a, f) = run_recovery(&recovery, exec, config.recovery_timeout_ms);
                recovery_attempts += a;
                recovery_failures += f;
            }
            if config.failure_policy == "stop" {
                stopped_on_failure = true;
                break;
            }
        } else {
            passed += 1;
            round_latencies.push(round_elapsed);
            let round = ScenarioRound {
                index,
                success: true,
                overall: report.overall.clone(),
                elapsed_ms: round_elapsed,
                steps: Vec::new(),
            };
            on_round(&round, passed, failed);
        }

        if !cancelled && !stopped_on_failure {
            let more = if config.iterations == 0 {
                true
            } else {
                index < config.iterations
            };
            if more && config.interval_ms > 0 {
                let mut remaining = config.interval_ms;
                while remaining > 0 {
                    if should_cancel() {
                        cancelled = true;
                        break;
                    }
                    let slice = remaining.min(100);
                    exec.sleep_ms(slice);
                    remaining -= slice;
                }
            }
        }
        if cancelled {
            break;
        }
    }

    let completed = passed + failed;
    let mut step_stats: Vec<StepLatencyStat> = step_latencies
        .into_iter()
        .map(|(id, (title, mut times))| StepLatencyStat {
            step_id: id,
            step_title: title,
            count: times.len() as u32,
            min_ms: times.iter().copied().min().unwrap_or(0),
            max_ms: times.iter().copied().max().unwrap_or(0),
            avg_ms: if times.is_empty() {
                0.0
            } else {
                times.iter().sum::<u64>() as f64 / times.len() as f64
            },
            p95_ms: percentile(&mut times, 0.95),
        })
        .collect();
    let order: std::collections::HashMap<String, usize> = flow
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.clone(), i))
        .collect();
    step_stats.sort_by_key(|s| order.get(&s.step_id).copied().unwrap_or(usize::MAX));

    let mut failure_clusters: Vec<FailureCluster> = clusters
        .into_iter()
        .map(|((step_id, kind), (count, rounds))| {
            let title = flow
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .map(|s| s.title.clone())
                .unwrap_or_default();
            FailureCluster {
                step_id,
                step_title: title,
                error_kind: kind,
                count,
                sample_rounds: rounds,
            }
        })
        .collect();
    failure_clusters.sort_by(|a, b| b.count.cmp(&a.count));

    // round_latencies 已含成功与失败全部轮次（失败明细另行截断存储）
    let mut all_latencies = round_latencies;
    let min_latency_ms = all_latencies.iter().copied().min().unwrap_or(0);
    let max_latency_ms = all_latencies.iter().copied().max().unwrap_or(0);
    let avg_latency_ms = if all_latencies.is_empty() {
        0.0
    } else {
        all_latencies.iter().sum::<u64>() as f64 / all_latencies.len() as f64
    };
    let p95_latency_ms = percentile(&mut all_latencies, 0.95);

    // 时长模式（iterations==0）下 planned 取实际完成轮数，避免
    // 时间到/取消时 planned 比 completed 多算一轮。
    let planned = if config.iterations == 0 {
        completed
    } else {
        config.iterations
    };
    Ok(ScenarioStressReport {
        flow_id: flow.id.clone(),
        flow_name: flow.name.clone(),
        planned,
        completed,
        passed,
        failed,
        cancelled,
        stopped_on_failure,
        time_limit_reached,
        elapsed_ms: started.elapsed().as_millis() as u64,
        min_latency_ms,
        max_latency_ms,
        avg_latency_ms,
        p95_latency_ms,
        step_stats,
        failure_clusters,
        failures,
        failures_truncated,
        recovery_attempts,
        recovery_failures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct MockExec {
        outcomes: VecDeque<AtOutcome>,
    }

    impl AtExecutor for MockExec {
        fn execute(&mut self, _cmd: &str, _timeout_ms: u64) -> AtOutcome {
            self.outcomes.pop_front().unwrap_or(AtOutcome {
                lines: vec!["+CSQ: 20,0".into()],
                final_code: "OK".into(),
            })
        }
        fn sleep_ms(&mut self, _ms: u64) {}
    }

    fn config(iterations: u32) -> StressConfig {
        StressConfig {
            command: "AT+CSQ".into(),
            iterations,
            interval_ms: 0,
            timeout_ms: 1000,
            expected_pattern: Some(r"\+CSQ:".into()),
            stop_on_failure: false,
        }
    }

    #[test]
    fn counts_successes_and_failures() {
        let mut exec = MockExec {
            outcomes: VecDeque::from(vec![
                AtOutcome {
                    lines: vec!["+CSQ: 20,0".into()],
                    final_code: "OK".into(),
                },
                AtOutcome {
                    lines: vec![],
                    final_code: "TIMEOUT".into(),
                },
            ]),
        };
        let report =
            run_stress_controlled(&config(2), &mut exec, &mut |_, _, _| {}, &mut || false).unwrap();
        assert_eq!((report.completed, report.passed, report.failed), (2, 1, 1));
        assert_eq!(report.failures[0].index, 2);
    }

    #[test]
    fn stops_on_first_failure() {
        let mut cfg = config(10);
        cfg.stop_on_failure = true;
        let mut exec = MockExec {
            outcomes: VecDeque::from(vec![AtOutcome {
                lines: vec![],
                final_code: "ERROR".into(),
            }]),
        };
        let report =
            run_stress_controlled(&cfg, &mut exec, &mut |_, _, _| {}, &mut || false).unwrap();
        assert_eq!(report.completed, 1);
        assert!(report.stopped_on_failure);
    }

    #[test]
    fn cancellation_is_observed_before_next_iteration() {
        let mut exec = MockExec {
            outcomes: VecDeque::new(),
        };
        let mut checks = 0;
        let report = run_stress_controlled(&config(10), &mut exec, &mut |_, _, _| {}, &mut || {
            checks += 1;
            checks > 1
        })
        .unwrap();
        assert_eq!(report.completed, 1);
        assert!(report.cancelled);
    }

    #[test]
    fn rejects_invalid_pattern() {
        let mut cfg = config(1);
        cfg.expected_pattern = Some("[".into());
        assert!(validate_config(&cfg).is_err());
    }
    use crate::diag::{DiagStep, EvalRule};

    fn pass_rule() -> EvalRule {
        EvalRule {
            when: Some("final == 'OK'".into()),
            default: false,
            result: "pass".into(),
            detail: None,
            reason: None,
            suggestion: None,
            collect: Vec::new(),
            abort: false,
        }
    }

    fn fail_rule() -> EvalRule {
        EvalRule {
            when: None,
            default: true,
            result: "fail".into(),
            detail: None,
            reason: None,
            suggestion: None,
            collect: Vec::new(),
            abort: false,
        }
    }

    fn one_step_flow() -> DiagFlow {
        DiagFlow {
            schema: "modulink/diagflow@1".into(),
            id: "stress_t".into(),
            name: "stress test".into(),
            applies_to: Vec::new(),
            kind: "usecase".into(),
            category: String::new(),
            source: String::new(),
            verified: true,
            execution_mode: "readonly".into(),
            steps: vec![DiagStep {
                id: "s1".into(),
                title: "probe".into(),
                send: vec!["AT".into()],
                timeout_ms: 1000,
                parse: None,
                retry: None,
                optional: false,
                expect_urc: None,
                evaluate: vec![pass_rule(), fail_rule()],
            }],
        }
    }

    fn scenario_cfg(n: u32) -> ScenarioStressConfig {
        ScenarioStressConfig {
            iterations: n,
            interval_ms: 0,
            failure_policy: "continue".into(),
            recovery_commands: Some(Vec::new()),
            recovery_timeout_ms: 1000,
            warn_as_pass: true,
            max_duration_sec: 0,
        }
    }

    #[test]
    fn infer_recovery_picks_last_close_command() {
        let mk = |cmd: &str| DiagStep {
            id: "s".into(),
            title: "s".into(),
            send: vec![cmd.into()],
            timeout_ms: 1000,
            parse: None,
            retry: None,
            optional: false,
            expect_urc: None,
            evaluate: vec![pass_rule(), fail_rule()],
        };
        let flow = DiagFlow {
            schema: "modulink/diagflow@1".into(),
            id: "t".into(),
            name: "t".into(),
            applies_to: Vec::new(),
            kind: "usecase".into(),
            category: String::new(),
            source: String::new(),
            verified: true,
            execution_mode: "mutating".into(),
            steps: vec![mk("AT+MIPOPEN=0,\"TCP\""), mk("AT+MIPCLOSE=0")],
        };
        assert_eq!(
            infer_recovery_commands(&flow),
            vec!["AT+MIPCLOSE=0".to_string()]
        );
    }

    #[test]
    fn scenario_stress_counts_pass_fail() {
        let flow = one_step_flow();
        let mut exec = MockExec {
            outcomes: VecDeque::from(vec![
                AtOutcome {
                    lines: vec![],
                    final_code: "OK".into(),
                },
                AtOutcome {
                    lines: vec![],
                    final_code: "ERROR".into(),
                },
                AtOutcome {
                    lines: vec![],
                    final_code: "OK".into(),
                },
            ]),
        };
        let report = run_scenario_stress(
            &flow,
            &scenario_cfg(3),
            &mut exec,
            &mut |_, _, _| {},
            &mut || false,
        )
        .unwrap();
        assert_eq!(report.completed, 3);
        assert_eq!(report.passed, 2);
        assert_eq!(report.failed, 1);
        assert!(!report.cancelled);
    }

    #[test]
    fn scenario_stress_failure_cluster() {
        let flow = one_step_flow();
        let mut exec = MockExec {
            outcomes: VecDeque::from(vec![
                AtOutcome {
                    lines: vec![],
                    final_code: "ERROR".into(),
                },
                AtOutcome {
                    lines: vec![],
                    final_code: "ERROR".into(),
                },
            ]),
        };
        let report = run_scenario_stress(
            &flow,
            &scenario_cfg(2),
            &mut exec,
            &mut |_, _, _| {},
            &mut || false,
        )
        .unwrap();
        assert_eq!(report.failed, 2);
        assert_eq!(report.failure_clusters.len(), 1);
        assert_eq!(report.failure_clusters[0].count, 2);
        assert_eq!(report.failure_clusters[0].step_id, "s1");
    }

    #[test]
    fn scenario_stress_stop_policy() {
        let mut cfg = scenario_cfg(10);
        cfg.failure_policy = "stop".into();
        let flow = one_step_flow();
        let mut exec = MockExec {
            outcomes: VecDeque::from(vec![AtOutcome {
                lines: vec![],
                final_code: "ERROR".into(),
            }]),
        };
        let report =
            run_scenario_stress(&flow, &cfg, &mut exec, &mut |_, _, _| {}, &mut || false).unwrap();
        assert_eq!(report.completed, 1);
        assert!(report.stopped_on_failure);
    }

    #[test]
    fn scenario_stress_cancel_observed() {
        let flow = one_step_flow();
        let mut exec = MockExec {
            outcomes: VecDeque::from(vec![AtOutcome {
                lines: vec![],
                final_code: "OK".into(),
            }]),
        };
        let report = run_scenario_stress(
            &flow,
            &scenario_cfg(10),
            &mut exec,
            &mut |_, _, _| {},
            &mut || true,
        )
        .unwrap();
        assert!(report.cancelled);
        assert_eq!(report.completed, 0);
    }

    #[test]
    fn scenario_stress_recovery_runs_on_failure() {
        let mut cfg = scenario_cfg(2);
        cfg.failure_policy = "recover".into();
        cfg.recovery_commands = Some(vec!["AT+MIPCLOSE=0".into()]);
        let flow = one_step_flow();
        let mut exec = MockExec {
            outcomes: VecDeque::from(vec![
                AtOutcome {
                    lines: vec![],
                    final_code: "ERROR".into(),
                },
                AtOutcome {
                    lines: vec![],
                    final_code: "OK".into(),
                },
                AtOutcome {
                    lines: vec![],
                    final_code: "OK".into(),
                },
            ]),
        };
        let report =
            run_scenario_stress(&flow, &cfg, &mut exec, &mut |_, _, _| {}, &mut || false).unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(report.recovery_attempts, 1);
        assert_eq!(report.recovery_failures, 0);
    }

    #[test]
    fn scenario_stress_duration_mode_planned_equals_completed() {
        use std::cell::Cell;
        let flow = one_step_flow();
        let mut exec = MockExec {
            outcomes: VecDeque::new(),
        };
        // 每轮 should_cancel 被多处调用（循环顶 + 引擎步骤边界 + run_step），
        // 用调用计数触发取消，确保至少跑完若干轮后取消。
        let calls = Cell::new(0u32);
        let mut should_cancel = move || {
            let n = calls.get();
            calls.set(n + 1);
            n >= 12
        };
        let cfg = ScenarioStressConfig {
            iterations: 0,
            interval_ms: 0,
            failure_policy: "continue".into(),
            recovery_commands: Some(Vec::new()),
            recovery_timeout_ms: 1000,
            warn_as_pass: true,
            max_duration_sec: 3600,
        };
        let report = run_scenario_stress(
            &flow,
            &cfg,
            &mut exec,
            &mut |_, _, _| {},
            &mut should_cancel,
        )
        .unwrap();
        assert!(report.cancelled);
        assert_eq!(report.planned, report.completed);
        assert!(report.completed >= 1);
    }
}
