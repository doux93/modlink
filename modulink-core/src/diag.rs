//! 诊断引擎：数据驱动的流程解释器。
//!
//! 流程定义在 data/diagnostics/*.yaml（DiagFlow），每步 = 发送指令 → 正则提取
//! 变量 → 按规则求值（pass/warn/fail + 原因 + 建议）→ 可选重试/终止/补采信息。
//! 引擎不接触串口：通过 AtExecutor trait 抽象执行，UI 层与单测各自实现。
//!
//! 表达式语法（刻意极小，FAE 能读懂、AI 能生成、无注入风险）：
//!   atom  := ident op literal        op ∈ { == != < > <= >= }
//!   expr  := atom (&& atom)* (|| atom)*   （|| 优先级低于 &&）
//!   字面量: 'str' 或 数字 或裸词；两侧均为数字时按数值比较，否则按字符串。
//!
//! 模板插值: "{var}"；引擎提供派生变量 dbm（rssi*2-113）与 roam（stat==5 → " (漫游)"）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------- 流程定义

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagFlow {
    pub schema: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub applies_to: Vec<String>,
    /// "checkup"(体检流程,默认) | "usecase"(典型用例,来自手册示例)
    #[serde(default = "default_flow_kind")]
    pub kind: String,
    /// 用例分类（socket/power/sms/network/...），UI 分组用
    #[serde(default)]
    pub category: String,
    /// 来源说明（如手册名），报告与 UI 展示
    #[serde(default)]
    pub source: String,
    /// 自动转换生成的用例先标 false，FAE 复核后置 true
    #[serde(default = "default_verified")]
    pub verified: bool,
    /// "readonly" | "mutating" | "unknown"，用于执行前展示流程影响范围。
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    pub steps: Vec<DiagStep>,
}

fn default_flow_kind() -> String {
    "checkup".into()
}

fn default_verified() -> bool {
    true
}

fn default_execution_mode() -> String {
    "unknown".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagStep {
    pub id: String,
    pub title: String,
    pub send: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetrySpec>,
    /// optional 步骤失败不计入整体结论
    #[serde(default)]
    pub optional: bool,
    /// 终结码之后还需等待的异步 URC（手册用例常见：OK 后 +MIPOPEN: 0,0 才算成功）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_urc: Option<UrcExpect>,
    pub evaluate: Vec<EvalRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrcExpect {
    /// 正则（支持命名捕获，捕获组并入变量表）
    pub pattern: String,
    #[serde(default = "default_urc_timeout")]
    pub timeout_ms: u64,
}

fn default_urc_timeout() -> u64 {
    10_000
}

fn default_timeout() -> u64 {
    5000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySpec {
    pub times: u32,
    pub interval_ms: u64,
    pub until: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default)]
    pub default: bool,
    /// pass | warn | fail
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// 命中该规则后额外执行的信息采集指令（结果进报告）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collect: Vec<String>,
    #[serde(default)]
    pub abort: bool,
}

// ---------------------------------------------------------------- 执行抽象

#[derive(Debug, Clone)]
pub struct AtOutcome {
    pub lines: Vec<String>,
    pub final_code: String,
}

/// 串口执行抽象：UI 层用真实 Session 实现，单测用剧本 mock 实现。
pub trait AtExecutor {
    fn execute(&mut self, cmd: &str, timeout_ms: u64) -> AtOutcome;
    /// 重试间隔等待（mock 可空实现加速测试）
    fn sleep_ms(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    /// 在发送指令前预注册 URC 等待器，避免 OK 与注册之间的竞态丢事件。
    /// 默认实现不支持（poll_urc 永远返回 Gone）。
    fn arm_urc(&mut self, _pattern: &str) {}
    /// 轮询已注册的 URC，最多阻塞 slice_ms。命中返回 Got；本切片超时返回
    /// Pending（waiter 仍有效，可继续轮询）；无 waiter 或通道断开返回 Gone。
    /// 命中后 waiter 自动清理。
    fn poll_urc(&mut self, _slice_ms: u64) -> UrcPoll {
        UrcPoll::Gone
    }
    /// 放弃并清理已注册的 URC 等待器（取消或同步命中后调用）。
    fn disarm_urc(&mut self) {}
}

/// 异步 URC 等待的轮询结果。
#[derive(Debug, Clone)]
pub enum UrcPoll {
    /// 命中并取到 URC 行。
    Got(String),
    /// 仍在等待（本切片内未到达，waiter 仍有效）。
    Pending,
    /// 无已注册 waiter 或通道已断开，应停止轮询。
    Gone,
}

impl UrcPoll {
    pub fn line(self) -> Option<String> {
        match self {
            UrcPoll::Got(s) => Some(s),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- 报告

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub id: String,
    pub title: String,
    /// pass | warn | fail | skip
    pub result: String,
    pub detail: String,
    pub reason: String,
    pub suggestion: String,
    pub raw: Vec<String>,
    pub collected: Vec<(String, Vec<String>)>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagReport {
    pub flow_id: String,
    pub flow_name: String,
    /// pass | warn | fail
    pub overall: String,
    pub aborted: bool,
    pub cancelled: bool,
    pub steps: Vec<StepReport>,
}

// ---------------------------------------------------------------- 表达式

fn is_num(s: &str) -> bool {
    !s.is_empty() && s.parse::<f64>().is_ok()
}

fn eval_atom(atom: &str, vars: &HashMap<String, String>) -> bool {
    let atom = atom.trim();
    for op in ["==", "!=", "<=", ">=", "<", ">"] {
        if let Some(pos) = atom.find(op) {
            let lhs = atom[..pos].trim();
            let rhs = atom[pos + op.len()..].trim().trim_matches('\'');
            let lval = vars.get(lhs).cloned().unwrap_or_default();
            let cmp: i32 = if is_num(&lval) && is_num(rhs) {
                let a: f64 = lval.parse().unwrap();
                let b: f64 = rhs.parse().unwrap();
                if a < b {
                    -1
                } else if a > b {
                    1
                } else {
                    0
                }
            } else if lval < rhs.to_string() {
                -1
            } else if lval > rhs.to_string() {
                1
            } else {
                0
            };
            return match op {
                "==" => cmp == 0,
                "!=" => cmp != 0,
                "<" => cmp < 0,
                ">" => cmp > 0,
                "<=" => cmp <= 0,
                ">=" => cmp >= 0,
                _ => unreachable!(),
            };
        }
    }
    false
}

/// || 优先级低于 &&
pub fn eval_expr(expr: &str, vars: &HashMap<String, String>) -> bool {
    expr.split("||")
        .any(|clause| clause.split("&&").all(|atom| eval_atom(atom, vars)))
}

/// Structural validation for the deliberately small expression language.
pub fn validate_expr(expr: &str) -> bool {
    !expr.trim().is_empty()
        && expr.split("||").all(|clause| {
            clause.split("&&").all(|atom| {
                let atom = atom.trim();
                ["==", "!=", "<=", ">=", "<", ">"].iter().any(|op| {
                    atom.find(op).is_some_and(|pos| {
                        !atom[..pos].trim().is_empty() && !atom[pos + op.len()..].trim().is_empty()
                    })
                })
            })
        })
}

/// Validate a flow definition; returns a list of warnings (empty = OK).
/// Used by the usecase editor before saving, mirroring load-time checks.
pub fn validate_flow(flow: &DiagFlow) -> Vec<String> {
    let mut warnings = Vec::new();
    if flow.id.is_empty() {
        warnings.push("flow id is empty".into());
    }
    if flow.name.is_empty() {
        warnings.push("flow name is empty".into());
    }
    if flow.steps.is_empty() {
        warnings.push("flow has no steps".into());
    }
    if !matches!(
        flow.execution_mode.as_str(),
        "readonly" | "mutating" | "unknown"
    ) {
        warnings.push(format!("invalid execution_mode {}", flow.execution_mode));
    }
    let mut step_ids = std::collections::HashSet::new();
    for step in &flow.steps {
        if step.id.is_empty() {
            warnings.push("step id is empty".into());
        } else if !step_ids.insert(step.id.clone()) {
            warnings.push(format!("duplicate step id {}", step.id));
        }
        if step.send.is_empty() {
            warnings.push(format!("{}: step has no send command", step.id));
        }
        if let Some(parse) = &step.parse {
            if regex::Regex::new(parse).is_err() {
                warnings.push(format!("{}: invalid parse regex", step.id));
            }
        }
        if let Some(retry) = &step.retry {
            if retry.times == 0 || !validate_expr(&retry.until) {
                warnings.push(format!("{}: invalid retry rule", step.id));
            }
        }
        if let Some(urc) = &step.expect_urc {
            if regex::Regex::new(&urc.pattern).is_err() {
                warnings.push(format!("{}: invalid URC regex", step.id));
            }
        }
        for rule in &step.evaluate {
            if !matches!(rule.result.as_str(), "pass" | "warn" | "fail") {
                warnings.push(format!("{}: unknown result {}", step.id, rule.result));
            }
            if let Some(condition) = &rule.when {
                if !validate_expr(condition) {
                    warnings.push(format!("{}: invalid when expression", step.id));
                }
            }
        }
    }
    warnings
}

/// 根据流程步骤里的指令推断执行影响范围。
/// - 含写 Flash / 重启 / 关机 / 写入网络参数 / 建立连接等 → "mutating"
/// - 全部为纯查询指令 → "readonly"
/// - 无指令或无法判断 → "unknown"
///
/// FAE 可在 YAML 中显式标注 execution_mode 覆盖推断结果；本函数用于自动转换
/// 生成、加载时补全未标注的流程。
pub fn infer_execution_mode(flow: &DiagFlow) -> String {
    // 若已显式标注非 unknown 值，保留人工标注
    if flow.execution_mode != "unknown" {
        return flow.execution_mode.clone();
    }
    let mut has_readonly = false;
    for step in &flow.steps {
        for cmd in &step.send {
            let upper = cmd.trim().to_uppercase();
            if upper.is_empty() {
                continue;
            }
            if is_mutating_command(&upper) {
                return "mutating".into();
            }
            if is_readonly_command(&upper) {
                has_readonly = true;
            }
        }
    }
    if has_readonly {
        "readonly".into()
    } else {
        "unknown".into()
    }
}

/// 判定指令是否会修改设备状态（写入/重启/关机/建立连接/激活 PDP 等）。
fn is_mutating_command(upper: &str) -> bool {
    // 写 Flash / 重启 / 关机 / 恢复出厂
    const MUTATING_PREFIXES: &[&str] = &[
        "AT+MREBOOT",
        "AT+CFUN=0",
        "AT+CFUN=1,1",
        "AT+MFLASH",
        "AT+WFLASH",
        "AT+MRESTORE",
        "AT+MRESET",
        "AT+RESET",
    ];
    if MUTATING_PREFIXES.iter().any(|p| upper.starts_with(p)) {
        return true;
    }
    // 写入网络参数：CGDCONT/CGACT 激活/CGCLASS 等带 "=" 赋值（非 "?"/"=?") 查询
    if (upper.starts_with("AT+CGDCONT=") || upper.starts_with("AT+CGACT="))
        && !upper.ends_with("?")
        && !upper.ends_with("=?")
    {
        return true;
    }
    // 建立连接类指令
    const CONNECT_PREFIXES: &[&str] = &[
        "AT+MIPOPEN",
        "AT+MIPLISTEN",
        "AT+MIPSEND",
        "AT+MQTTCONNECT",
        "AT+MQTTSUB",
        "AT+MQTTPUB",
        "AT+MFTPCONN",
        "AT+MFTPSTOR",
        "AT+MFTPRETR",
        "AT+MFTPDEL",
        "AT+MHTTPURL",
        "AT+MHTTPSEND",
        "AT+HTTPURL",
        "AT+HTTPACTION",
        "AT+NSOCR",
        "AT+NSOST",
        "AT+CIPSTART",
        "AT+CIPSEND",
    ];
    if CONNECT_PREFIXES.iter().any(|p| upper.starts_with(p)) {
        return true;
    }
    // 通用写指令启发式：AT+XXX=<value> 形式（非查询 ?/=?"），排除已知只读族
    if upper.contains('=') && !upper.ends_with('?') && !upper.ends_with("=?") {
        // 排除 CSQ/CEREG/CGPADDR 等查询族（它们不会用 = 赋值，此条件一般不误命中）
        // 但保留对设置类指令的判定（如 AT+COPS=, AT+CMGF= 等）
        return true;
    }
    false
}

/// 判定指令是否为纯查询（不修改设备状态）。
fn is_readonly_command(upper: &str) -> bool {
    // 以 ? 结尾的查询指令
    if upper.ends_with('?') {
        return true;
    }
    // 已知只读族前缀
    const READONLY_PREFIXES: &[&str] = &[
        "AT+CSQ",
        "AT+CEREG",
        "AT+CREG",
        "AT+CGREG",
        "AT+COPS?",
        "AT+CPIN",
        "AT+CGPADDR",
        "AT+CGDCONT?",
        "AT+CGACT?",
        "AT+CGATT?",
        "AT+CIMI",
        "AT+ICCID",
        "AT+QCCID",
        "AT+MIPSTAT",
        "AT+MQTTSTAT",
        "AT+MVERSION",
        "AT+CGMM",
        "AT+CGMR",
        "AT+CGSN",
        "ATI",
        "AT+MID",
        "AT+MSWVER",
        "AT+CEER",
    ];
    READONLY_PREFIXES.iter().any(|p| upper.starts_with(p))
}

fn interpolate(template: &str, vars: &HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    // A parse expression may not match an error response. Never leak the
    // flow's template placeholder into a user-facing diagnostic report.
    regex::Regex::new(r"\{[A-Za-z_][A-Za-z0-9_]*\}")
        .expect("valid diagnostic placeholder regex")
        .replace_all(&out, "未解析")
        .into_owned()
}

/// 派生便捷变量
fn add_derived(vars: &mut HashMap<String, String>) {
    if let Some(rssi) = vars.get("rssi").and_then(|v| v.parse::<i64>().ok()) {
        if rssi != 99 {
            vars.insert("dbm".into(), (rssi * 2 - 113).to_string());
        } else {
            vars.insert("dbm".into(), "-".into());
        }
    }
    if let Some(stat) = vars.get("stat") {
        vars.insert(
            "roam".into(),
            if stat == "5" {
                " (漫游)".into()
            } else {
                String::new()
            },
        );
    }
}

// ---------------------------------------------------------------- 引擎

/// 取消时返回的步骤报告（result=skip，不计入整体结论）。
fn cancelled_step_report(step: &DiagStep, started: std::time::Instant) -> StepReport {
    StepReport {
        id: step.id.clone(),
        title: step.title.clone(),
        result: "skip".into(),
        detail: String::new(),
        reason: "用户取消".into(),
        suggestion: String::new(),
        raw: Vec::new(),
        collected: Vec::new(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// 可被取消信号中断的 sleep：以 100ms 切片轮询 should_cancel。
fn cancellable_sleep(exec: &mut dyn AtExecutor, ms: u64, should_cancel: &mut dyn FnMut() -> bool) {
    let mut remaining = ms;
    while remaining > 0 {
        if should_cancel() {
            return;
        }
        let slice = remaining.min(100);
        exec.sleep_ms(slice);
        remaining -= slice;
    }
}

/// 执行单个步骤（含重试），返回步骤报告。
fn run_step(
    step: &DiagStep,
    exec: &mut dyn AtExecutor,
    should_cancel: &mut dyn FnMut() -> bool,
) -> StepReport {
    let started = std::time::Instant::now();
    let parse_re = step.parse.as_ref().and_then(|p| regex::Regex::new(p).ok());

    let attempts = step.retry.as_ref().map(|r| r.times.max(1)).unwrap_or(1);
    let mut vars: HashMap<String, String> = HashMap::new();
    let mut raw: Vec<String> = Vec::new();

    for attempt in 0..attempts {
        if should_cancel() {
            return cancelled_step_report(step, started);
        }
        vars.clear();
        raw.clear();
        // 0. 在发送前预注册 URC 等待器，避免 OK 与注册之间的竞态丢事件
        if step.expect_urc.is_some() {
            let pattern = step.expect_urc.as_ref().unwrap().pattern.clone();
            exec.arm_urc(&pattern);
        }
        // 1. 发送本步骤的指令序列
        for cmd in &step.send {
            if should_cancel() {
                exec.disarm_urc();
                return cancelled_step_report(step, started);
            }
            let outcome = exec.execute(cmd, step.timeout_ms);
            raw.push(format!("> {cmd}"));
            raw.extend(outcome.lines.iter().cloned());
            raw.push(outcome.final_code.clone());
            vars.insert("final".into(), outcome.final_code.clone());
            // 2. 正则提取命名捕获（对所有响应行尝试，首个命中生效）
            if let Some(re) = &parse_re {
                for line in &outcome.lines {
                    if let Some(caps) = re.captures(line) {
                        for name in re.capture_names().flatten() {
                            if let Some(m) = caps.name(name) {
                                vars.insert(name.to_string(), m.as_str().to_string());
                            }
                        }
                        break;
                    }
                }
            }
        }
        // 2.5 异步 URC 等待（终结码 OK 之后模组再上报结果的场景）。
        // 手册示例中的 +XXX 行有时是同步响应（OK 前已返回）——先在已收到的
        // 行里找，命中则无需等待，避免把同步响应误判成"URC 超时"。
        // URC 等待已在发送前 arm，故 OK 与 arm 之间到达的 URC 不会丢失；
        // 此处以短切片轮询，使取消信号能在等待中被及时响应。
        if let Some(urc) = &step.expect_urc {
            let re = regex::Regex::new(&urc.pattern).ok();
            let already = re
                .as_ref()
                .and_then(|re| raw.iter().find(|line| re.is_match(line)).cloned());
            let hit = if let Some(line) = already {
                // 同步命中：放弃预注册的 waiter（其可能也收到同一行）
                exec.disarm_urc();
                Some(line)
            } else {
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_millis(urc.timeout_ms);
                let mut got: Option<String> = None;
                while std::time::Instant::now() < deadline {
                    if should_cancel() {
                        exec.disarm_urc();
                        return cancelled_step_report(step, started);
                    }
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    let slice = remaining.as_millis().min(200).max(1) as u64;
                    match exec.poll_urc(slice) {
                        UrcPoll::Got(line) => {
                            got = Some(line);
                            break;
                        }
                        UrcPoll::Pending => continue,
                        UrcPoll::Gone => break,
                    }
                }
                // 超时仍未命中：清理可能残留的 waiter
                exec.disarm_urc();
                if let Some(line) = &got {
                    raw.push(line.clone());
                }
                got
            };
            match hit {
                Some(line) => {
                    vars.insert("urc".into(), "1".into());
                    if let Some(re) = &re {
                        if let Some(caps) = re.captures(&line) {
                            for name in re.capture_names().flatten() {
                                if let Some(m) = caps.name(name) {
                                    vars.insert(name.to_string(), m.as_str().to_string());
                                }
                            }
                        }
                    }
                }
                None => {
                    raw.push(format!("(等待 URC 超时: {})", urc.pattern));
                    vars.insert("urc".into(), "0".into());
                }
            }
        }
        add_derived(&mut vars);
        // 3. 重试判定
        if let Some(retry) = &step.retry {
            if eval_expr(&retry.until, &vars) || attempt + 1 == attempts {
                break;
            }
            cancellable_sleep(exec, retry.interval_ms, should_cancel);
            if should_cancel() {
                return cancelled_step_report(step, started);
            }
        } else {
            break;
        }
    }

    // 4. 规则求值
    for rule in &step.evaluate {
        let hit = rule.default
            || rule
                .when
                .as_ref()
                .map(|w| eval_expr(w, &vars))
                .unwrap_or(false);
        if !hit {
            continue;
        }
        // 5. 补采信息
        let mut collected = Vec::new();
        for cmd in &rule.collect {
            let o = exec.execute(cmd, step.timeout_ms);
            let mut lines = o.lines.clone();
            lines.push(o.final_code.clone());
            collected.push((cmd.clone(), lines));
        }
        return StepReport {
            id: step.id.clone(),
            title: step.title.clone(),
            result: rule.result.clone(),
            detail: interpolate(rule.detail.as_deref().unwrap_or(""), &vars),
            reason: interpolate(rule.reason.as_deref().unwrap_or(""), &vars),
            suggestion: interpolate(rule.suggestion.as_deref().unwrap_or(""), &vars),
            raw: raw.clone(),
            collected,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    // 没有规则命中：视为失败（流程定义缺陷，报告出来便于修 YAML）
    StepReport {
        id: step.id.clone(),
        title: step.title.clone(),
        result: "fail".into(),
        detail: String::new(),
        reason: "诊断流程未覆盖此响应（规则缺失）".into(),
        suggestion: format!("请完善诊断流程定义 {}", step.id),
        raw,
        collected: Vec::new(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// 执行整个流程。`on_step` 每步完成即回调（UI 实时刷新）。
pub fn run_flow(
    flow: &DiagFlow,
    exec: &mut dyn AtExecutor,
    on_step: &mut dyn FnMut(&StepReport),
) -> DiagReport {
    run_flow_controlled(flow, exec, on_step, &mut || false)
}

/// Execute a flow with cooperative cancellation. Cancellation is observed at
/// step boundaries; an in-flight serial command is allowed to finish or time out.
pub fn run_flow_controlled(
    flow: &DiagFlow,
    exec: &mut dyn AtExecutor,
    on_step: &mut dyn FnMut(&StepReport),
    should_cancel: &mut dyn FnMut() -> bool,
) -> DiagReport {
    let mut steps = Vec::new();
    let mut aborted = false;
    let mut cancelled = false;

    for step in &flow.steps {
        if !cancelled && should_cancel() {
            cancelled = true;
        }
        if aborted || cancelled {
            let skipped = StepReport {
                id: step.id.clone(),
                title: step.title.clone(),
                result: "skip".into(),
                detail: String::new(),
                reason: if cancelled {
                    "用户取消，已跳过".into()
                } else {
                    "前序步骤失败，已跳过".into()
                },
                suggestion: String::new(),
                raw: Vec::new(),
                collected: Vec::new(),
                elapsed_ms: 0,
            };
            on_step(&skipped);
            steps.push(skipped);
            continue;
        }
        let report = run_step(step, exec, should_cancel);
        if should_cancel() {
            cancelled = true;
        }
        // 该步骤结果若存在同结果的 abort 规则 → 终止后续步骤
        // （取消导致的 skip 不触发 abort）
        let should_abort = !cancelled
            && !step.optional
            && report.result != "pass"
            && report.result != "skip"
            && step
                .evaluate
                .iter()
                .any(|r| r.abort && r.result == report.result);
        on_step(&report);
        steps.push(report);
        if should_abort {
            aborted = true;
        }
    }

    let required_steps = flow
        .steps
        .iter()
        .zip(&steps)
        .filter(|(step, _)| !step.optional)
        .map(|(_, report)| report);
    let required_results: Vec<&StepReport> = required_steps.collect();

    let overall = if required_results
        .iter()
        .any(|s| s.result == "fail" || s.result == "skip")
    {
        "fail"
    } else if required_results.iter().any(|s| s.result == "warn") {
        "warn"
    } else {
        "pass"
    };

    DiagReport {
        flow_id: flow.id.clone(),
        flow_name: flow.name.clone(),
        overall: overall.into(),
        aborted,
        cancelled,
        steps,
    }
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn load_flow() -> DiagFlow {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../data/diagnostics/network_attach.yaml");
        serde_yaml::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn interpolation_does_not_expose_unresolved_placeholders() {
        let vars = HashMap::from([("final".to_string(), "ERROR".to_string())]);
        assert_eq!(
            interpolate("SIM 未就绪：{state}（{final}）", &vars),
            "SIM 未就绪：未解析（ERROR）"
        );
    }

    /// 剧本 mock：按指令前缀返回预设响应
    struct MockExec {
        scenario: &'static str,
    }

    impl AtExecutor for MockExec {
        fn execute(&mut self, cmd: &str, _timeout: u64) -> AtOutcome {
            let ok = |lines: Vec<&str>| AtOutcome {
                lines: lines.into_iter().map(String::from).collect(),
                final_code: "OK".into(),
            };
            let cme = |code: u32| AtOutcome {
                lines: vec![],
                final_code: format!("+CME ERROR: {code}"),
            };
            let s = self.scenario;
            if cmd == "AT" {
                return ok(vec![]);
            }
            if cmd.starts_with("AT+CPIN?") {
                return match s {
                    "no_sim" => cme(10),
                    "pin_locked" => ok(vec!["+CPIN: SIM PIN"]),
                    _ => ok(vec!["+CPIN: READY"]),
                };
            }
            if cmd.starts_with("AT+CSQ") {
                return match s {
                    "weak_signal" => ok(vec!["+CSQ: 4,0"]),
                    _ => ok(vec!["+CSQ: 23,0"]),
                };
            }
            if cmd.starts_with("AT+CEREG?") {
                return match s {
                    "reg_denied" => ok(vec!["+CEREG: 0,3"]),
                    "weak_signal" => ok(vec!["+CEREG: 0,1"]),
                    _ => ok(vec!["+CEREG: 0,1"]),
                };
            }
            if cmd.starts_with("AT+CEER") {
                return ok(vec!["+CEER: EMM_CAUSE_15"]);
            }
            if cmd.starts_with("AT+COPS") {
                return ok(vec!["+COPS: 0"]);
            }
            if cmd.starts_with("AT+CGDCONT?") {
                return ok(vec!["+CGDCONT: 1,\"IP\",\"\",\"0.0.0.0\""]);
            }
            if cmd.starts_with("AT+CGACT") {
                return match s {
                    "apn_wrong" => cme(148),
                    _ => ok(vec![]),
                };
            }
            if cmd.starts_with("AT+CGPADDR") {
                return match s {
                    "apn_wrong" => ok(vec!["+CGPADDR: 1,\"0.0.0.0\""]),
                    _ => ok(vec!["+CGPADDR: 1,\"10.1.2.3\""]),
                };
            }
            if cmd.starts_with("AT+MDNSGIP") {
                return match s {
                    "normal" => ok(vec!["+MDNSGIP: \"www.baidu.com\",\"39.156.66.14\""]),
                    _ => cme(100),
                };
            }
            AtOutcome {
                lines: vec![],
                final_code: "ERROR".into(),
            }
        }
        fn sleep_ms(&mut self, _ms: u64) {} // 测试不真等
    }

    fn run(scenario: &'static str) -> DiagReport {
        let flow = load_flow();
        let mut exec = MockExec { scenario };
        run_flow(&flow, &mut exec, &mut |_| {})
    }

    #[test]
    fn scenario_normal_all_pass() {
        let r = run("normal");
        assert_eq!(r.overall, "pass", "{:#?}", r.steps);
        assert!(!r.aborted);
    }

    #[test]
    fn scenario_no_sim_aborts_at_sim_step() {
        let r = run("no_sim");
        assert_eq!(r.overall, "fail");
        assert!(r.aborted);
        let sim = r.steps.iter().find(|s| s.id == "sim_check").unwrap();
        assert_eq!(sim.result, "fail");
        // SIM 之后的步骤全部跳过
        assert!(r.steps.iter().filter(|s| s.result == "skip").count() >= 3);
    }

    #[test]
    fn scenario_pin_locked_warns_and_aborts() {
        let r = run("pin_locked");
        let sim = r.steps.iter().find(|s| s.id == "sim_check").unwrap();
        assert_eq!(sim.result, "warn");
        assert!(sim.suggestion.contains("CPIN"));
        assert!(r.aborted);
    }

    #[test]
    fn scenario_weak_signal_warns_but_continues() {
        let r = run("weak_signal");
        let sig = r.steps.iter().find(|s| s.id == "signal").unwrap();
        assert_eq!(sig.result, "warn");
        assert!(sig.reason.contains("-105"), "dbm 派生变量: {}", sig.reason);
        // 弱信号只警告，不终止
        let reg = r.steps.iter().find(|s| s.id == "reg").unwrap();
        assert_eq!(reg.result, "pass");
    }

    #[test]
    fn scenario_reg_denied_collects_ceer() {
        let r = run("reg_denied");
        let reg = r.steps.iter().find(|s| s.id == "reg").unwrap();
        assert_eq!(reg.result, "fail");
        assert!(reg.reason.contains("拒绝"));
        // 自动补采了 AT+CEER
        assert!(reg.collected.iter().any(|(c, _)| c == "AT+CEER"));
    }

    #[test]
    fn scenario_apn_wrong_fails_at_pdp() {
        let r = run("apn_wrong");
        let pdp = r.steps.iter().find(|s| s.id == "pdp").unwrap();
        assert_eq!(pdp.result, "fail");
        assert!(pdp.suggestion.contains("APN"));
    }

    #[test]
    fn optional_failure_does_not_affect_overall_or_abort() {
        let pass_rule = EvalRule {
            when: Some("final == 'OK'".into()),
            default: false,
            result: "pass".into(),
            detail: None,
            reason: None,
            suggestion: None,
            collect: Vec::new(),
            abort: false,
        };
        let flow = DiagFlow {
            schema: "modulink/diagflow@1".into(),
            id: "optional_test".into(),
            name: "optional test".into(),
            applies_to: Vec::new(),
            kind: "checkup".into(),
            category: String::new(),
            source: String::new(),
            verified: true,
            execution_mode: "readonly".into(),
            steps: vec![
                DiagStep {
                    id: "required_before".into(),
                    title: "required before".into(),
                    send: vec!["AT".into()],
                    timeout_ms: 100,
                    parse: None,
                    retry: None,
                    optional: false,
                    expect_urc: None,
                    evaluate: vec![pass_rule.clone()],
                },
                DiagStep {
                    id: "optional_fail".into(),
                    title: "optional fail".into(),
                    send: vec!["AT+OPTIONAL_FAIL".into()],
                    timeout_ms: 100,
                    parse: None,
                    retry: None,
                    optional: true,
                    expect_urc: None,
                    evaluate: vec![EvalRule {
                        when: None,
                        default: true,
                        result: "fail".into(),
                        detail: None,
                        reason: Some("optional failure".into()),
                        suggestion: None,
                        collect: Vec::new(),
                        abort: true,
                    }],
                },
                DiagStep {
                    id: "required_after".into(),
                    title: "required after".into(),
                    send: vec!["AT".into()],
                    timeout_ms: 100,
                    parse: None,
                    retry: None,
                    optional: false,
                    expect_urc: None,
                    evaluate: vec![pass_rule],
                },
            ],
        };
        let mut exec = MockExec { scenario: "normal" };
        let r = run_flow(&flow, &mut exec, &mut |_| {});
        assert_eq!(r.overall, "pass", "{:#?}", r.steps);
        assert!(!r.aborted);
        assert_eq!(r.steps[1].result, "fail");
        assert_eq!(r.steps[2].result, "pass");
    }

    #[test]
    fn cancellation_skips_remaining_steps() {
        let flow = load_flow();
        let mut exec = MockExec { scenario: "normal" };
        let mut checks = 0;
        let r = run_flow_controlled(&flow, &mut exec, &mut |_| {}, &mut || {
            checks += 1;
            checks > 1
        });
        assert!(r.cancelled);
        assert!(r.steps.iter().any(|step| step.result == "skip"));
    }

    #[test]
    fn expr_eval_basics() {
        let mut v = HashMap::new();
        v.insert("stat".to_string(), "5".to_string());
        v.insert("rssi".to_string(), "8".to_string());
        assert!(eval_expr("stat == 1 || stat == 5", &v));
        assert!(eval_expr("rssi < 10", &v));
        assert!(!eval_expr("rssi < 10 && stat == 1", &v));
        v.insert("ip".to_string(), "10.0.0.1".to_string());
        assert!(eval_expr("ip != '' && ip != '0.0.0.0'", &v));
        assert!(validate_expr("ip != '' && stat >= 1"));
        assert!(!validate_expr("stat"));
    }

    fn mk_flow(send: Vec<&str>, execution_mode: &str) -> DiagFlow {
        DiagFlow {
            schema: "modulink/diagflow@1".into(),
            id: "infer_t".into(),
            name: "infer test".into(),
            applies_to: Vec::new(),
            kind: "usecase".into(),
            category: String::new(),
            source: String::new(),
            verified: true,
            execution_mode: execution_mode.into(),
            steps: vec![DiagStep {
                id: "s1".into(),
                title: "s1".into(),
                send: send.into_iter().map(String::from).collect(),
                timeout_ms: 1000,
                parse: None,
                retry: None,
                optional: false,
                expect_urc: None,
                evaluate: Vec::new(),
            }],
        }
    }

    #[test]
    fn infer_readonly_for_query_commands() {
        let flow = mk_flow(vec!["AT+CSQ", "AT+CEREG?"], "unknown");
        assert_eq!(infer_execution_mode(&flow), "readonly");
    }

    #[test]
    fn infer_mutating_for_write_commands() {
        let flow = mk_flow(vec!["AT+CSQ", "AT+MREBOOT"], "unknown");
        assert_eq!(infer_execution_mode(&flow), "mutating");
    }

    #[test]
    fn infer_mutating_for_connection_commands() {
        let flow = mk_flow(vec!["AT+MIPOPEN=0,\"TCP\",\"1.2.3.4\",80"], "unknown");
        assert_eq!(infer_execution_mode(&flow), "mutating");
    }

    #[test]
    fn infer_preserves_explicit_annotation() {
        let flow = mk_flow(vec!["AT+MREBOOT"], "readonly");
        assert_eq!(infer_execution_mode(&flow), "readonly");
    }

    /// expect_urc：等到 URC → 捕获组入变量表；等不到 → urc=0 走失败分支
    struct UrcMock {
        urc_line: Option<&'static str>,
        armed: Option<String>,
    }

    impl AtExecutor for UrcMock {
        fn execute(&mut self, _cmd: &str, _timeout: u64) -> AtOutcome {
            AtOutcome {
                lines: vec![],
                final_code: "OK".into(),
            }
        }
        fn sleep_ms(&mut self, _ms: u64) {}
        fn arm_urc(&mut self, pattern: &str) {
            self.armed = Some(pattern.to_string());
        }
        fn poll_urc(&mut self, _slice_ms: u64) -> UrcPoll {
            match self.armed.take() {
                Some(pattern) => {
                    let re = regex::Regex::new(&pattern).ok();
                    match re
                        .and_then(|re| self.urc_line.filter(|l| re.is_match(l)).map(String::from))
                    {
                        Some(line) => UrcPoll::Got(line),
                        None => {
                            // 已 arm 但 mock 无匹配行：标记为仍在等待，使引擎
                            // 轮询到超时；同时清空 armed 避免无限忙等。
                            UrcPoll::Gone
                        }
                    }
                }
                None => UrcPoll::Gone,
            }
        }
        fn disarm_urc(&mut self) {
            self.armed = None;
        }
    }

    fn urc_step() -> DiagStep {
        DiagStep {
            id: "open".into(),
            title: "建立 TCP 连接".into(),
            send: vec!["AT+MIPOPEN=0,\"TCP\",\"1.2.3.4\",9000".into()],
            timeout_ms: 1000,
            parse: None,
            retry: None,
            optional: false,
            expect_urc: Some(UrcExpect {
                pattern: r"\+MIPOPEN: 0,(?<result>\d+)".into(),
                timeout_ms: 5000,
            }),
            evaluate: vec![
                EvalRule {
                    when: Some("urc == 1 && result == 0".into()),
                    default: false,
                    result: "pass".into(),
                    detail: Some("连接成功".into()),
                    reason: None,
                    suggestion: None,
                    collect: Vec::new(),
                    abort: false,
                },
                EvalRule {
                    when: None,
                    default: true,
                    result: "fail".into(),
                    detail: None,
                    reason: Some("连接失败或超时 (urc={urc})".into()),
                    suggestion: None,
                    collect: Vec::new(),
                    abort: true,
                },
            ],
        }
    }

    #[test]
    fn expect_urc_success_extracts_captures() {
        let mut exec = UrcMock {
            urc_line: Some("+MIPOPEN: 0,0"),
            armed: None,
        };
        let report = run_step(&urc_step(), &mut exec, &mut || false);
        assert_eq!(report.result, "pass", "{report:?}");
        assert!(report.raw.iter().any(|l| l.contains("+MIPOPEN: 0,0")));
    }

    #[test]
    fn expect_urc_timeout_fails() {
        let mut exec = UrcMock {
            urc_line: None,
            armed: None,
        };
        let report = run_step(&urc_step(), &mut exec, &mut || false);
        assert_eq!(report.result, "fail");
        assert!(report.reason.contains("urc=0"));
    }
}
