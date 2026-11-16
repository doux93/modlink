//! 日志分析引擎：AT 口交互配对 + log 口模式匹配 + 错误码关联。
//!
//! 两种分析模式：
//! - AT 口分析：把 Frame 序列按"请求-响应"配对，提取 ERROR/TIMEOUT/CME/CMS，
//!   查 error_codes 知识库关联原因和建议。
//! - log 口分析：按 cmiot_log_patterns 知识库做正则匹配，提取模块/级别/消息。
//!
//! 纯逻辑，不接触串口——通过入参 Frame 序列和 LogPatternStore 驱动，可单测。

use crate::at_protocol::classify_final;
use crate::knowledge::ErrorCodeEntry;
use regex::Regex;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- 日志模式知识库

/// 日志模式知识库（从 data/log_patterns/*.yaml 加载）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPatternStore {
    pub modules: Vec<LogPatternModule>,
}

impl Default for LogPatternStore {
    fn default() -> Self {
        LogPatternStore { modules: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPatternModule {
    pub id: String,
    pub name: String,
    pub patterns: Vec<LogPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPattern {
    pub message: String,
    pub pattern: String,
    #[serde(default = "default_level")]
    pub level: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default = "default_severity")]
    pub severity: String,
}

fn default_level() -> String {
    "error".into()
}
fn default_severity() -> String {
    "error".into()
}

impl LogPatternStore {
    pub fn load_from_dir(dir: &std::path::Path) -> Self {
        let mut modules = Vec::new();
        if dir.is_dir() {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if entry.path().extension().is_none_or(|e| e != "yaml") {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    if let Ok(store) = serde_yaml::from_str::<LogPatternStore>(&text) {
                        modules.extend(store.modules);
                    }
                }
            }
        }
        LogPatternStore { modules }
    }

    /// 编译所有 pattern 为正则，返回 (module_id, module_name, compiled_pattern, level) 列表
    fn compiled_patterns(&self) -> Vec<CompiledPattern> {
        let mut out = Vec::new();
        for module in &self.modules {
            for pat in &module.patterns {
                if let Ok(re) = Regex::new(&expand_format_specifiers(&pat.pattern)) {
                    out.push(CompiledPattern {
                        module_id: module.id.clone(),
                        module_name: module.name.clone(),
                        message: pat.message.clone(),
                        level: pat.level.clone(),
                        severity: pat.severity.clone(),
                        regex: re,
                    });
                }
            }
        }
        out
    }
}

/// 把 SDK 源码里的 printf 格式符还原成正则占位。
///
/// 模式库由外部脚本从 cmiot SDK 的日志调用点扫描生成，保留的是**格式串**
/// （如 `%s:cid %d do not active !`），而真实日志里格式符已被实参替换。
/// 不展开的话这类模式只能匹配字面量 `%s`/`%d`——即永远不命中（当前库中
/// 209 条有 96 条属于此类）。
///
/// 展开策略刻意保守：
/// - `%d`（及 `%i/%u/%x/%X/%o`）→ `-?[0-9a-fA-F]+`，只吃单个数字令牌；
/// - `%s` → `\S+`，不跨空白，避免把整行吞掉造成误命中；
/// - 其余格式符 → `\S+`，同样受限。
///
/// 转换在编译期做而不是改 YAML：模式库是生成物，改数据下次重新生成即丢失。
fn expand_format_specifiers(pattern: &str) -> String {
    let bytes: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut i = 0;
    while i < bytes.len() {
        // 反斜杠转义原样保留（生成器对字面量做过 regex 转义，如 `\ ` `\!`）
        if bytes[i] == '\\' && i + 1 < bytes.len() {
            out.push(bytes[i]);
            out.push(bytes[i + 1]);
            i += 2;
            continue;
        }
        if bytes[i] != '%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        // 尝试按 printf 语法吃掉一个完整格式符：%[flags][width][.prec][length]conv
        let start = i;
        let mut j = i + 1;
        while j < bytes.len() && "-+ #0".contains(bytes[j]) {
            j += 1;
        }
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == '.' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
        }
        for len_mod in ["hh", "ll", "h", "l", "z", "j", "t", "L"] {
            if bytes[j..].starts_with(&len_mod.chars().collect::<Vec<_>>()[..]) {
                j += len_mod.len();
                break;
            }
        }
        match bytes.get(j) {
            // `%%` 是字面百分号
            Some('%') => {
                out.push('%');
                i = j + 1;
            }
            Some(&conv) if "diouxX".contains(conv) => {
                out.push_str("-?[0-9a-fA-F]+");
                i = j + 1;
            }
            Some(&conv) if "eEfgGaAcspn".contains(conv) => {
                out.push_str(r"\S+");
                i = j + 1;
            }
            // 不是合法格式符（如末尾孤立的 `%`）——按字面量处理
            _ => {
                out.push_str("%");
                i = start + 1;
            }
        }
    }
    out
}

struct CompiledPattern {
    module_id: String,
    module_name: String,
    message: String,
    level: String,
    severity: String,
    regex: Regex,
}

// ---------------------------------------------------------------- 输入帧

/// 分析输入：统一的日志行结构（从 Frame 或导出文件解析）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFrame {
    /// Unix 毫秒（无法解析时为 0）
    pub ts: i64,
    /// "tx" | "rx"
    pub dir: String,
    /// 原始文本
    pub text: String,
}

// ---------------------------------------------------------------- AT 口分析

/// AT 口分析报告
#[derive(Debug, Clone, Serialize)]
pub struct AtLogReport {
    pub total_interactions: u32,
    pub error_interactions: u32,
    pub anomalies: Vec<AtAnomaly>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AtAnomaly {
    pub index: u32,
    pub timestamp: i64,
    pub request: String,
    pub kind: String,
    pub final_code: String,
    pub raw_lines: Vec<String>,
    pub error_code: Option<ErrorCodeEntry>,
}

/// 分析 AT 口 Frame 序列：按 tx→rx→final 配对，提取异常。
/// `error_lookup` 闭包用于查 error_codes 知识库（code_type, code）→ entry。
pub fn analyze_at_log(
    frames: &[LogFrame],
    error_lookup: &dyn Fn(&str, u32) -> Option<ErrorCodeEntry>,
) -> AtLogReport {
    let interactions = pair_interactions(frames);
    let mut anomalies = Vec::new();
    let mut error_count = 0u32;

    for (idx, inter) in interactions.iter().enumerate() {
        let is_error = inter.final_code == "ERROR"
            || inter.final_code == "TIMEOUT"
            || inter.final_code.starts_with("+CME ERROR")
            || inter.final_code.starts_with("+CMS ERROR")
            || inter.final_code == "NO CARRIER";

        if !is_error {
            continue;
        }
        error_count += 1;

        let (code_type, code_num) = extract_error_code(&inter.final_code);
        let error_code = code_type
            .and_then(|ct| code_num.and_then(|cn| error_lookup(ct, cn)));

        let kind = if inter.final_code == "TIMEOUT" {
            "timeout"
        } else if inter.final_code == "NO CARRIER" {
            "disconnect"
        } else if inter.final_code.starts_with("+CME ERROR") {
            "cme_error"
        } else if inter.final_code.starts_with("+CMS ERROR") {
            "cms_error"
        } else {
            "error"
        };

        anomalies.push(AtAnomaly {
            index: (idx + 1) as u32,
            timestamp: inter.ts,
            request: inter.request.clone(),
            kind: kind.into(),
            final_code: inter.final_code.clone(),
            raw_lines: inter.raw_lines.clone(),
            error_code,
        });
    }

    let total = interactions.len() as u32;
    let summary = if error_count == 0 {
        format!("{total} 次交互，无异常")
    } else {
        format!("{total} 次交互中 {error_count} 次失败")
    };

    AtLogReport {
        total_interactions: total,
        error_interactions: error_count,
        anomalies,
        summary,
    }
}

/// 一个 AT 交互单元：请求 + 响应行 + 终结码
struct Interaction {
    ts: i64,
    request: String,
    final_code: String,
    raw_lines: Vec<String>,
}

/// 把 Frame 序列配对为交互单元。
/// 逻辑：tx 行开启一个新交互，后续 rx 行累积，直到遇到 final 行关闭。
/// tx 后无 final（下一 tx 到来或序列结束）→ final_code = "TIMEOUT"。
fn pair_interactions(frames: &[LogFrame]) -> Vec<Interaction> {
    let mut result = Vec::new();
    let mut current: Option<Interaction> = None;

    for frame in frames {
        let text = frame.text.trim();
        if text.is_empty() {
            continue;
        }

        if frame.dir == "tx" {
            // 新请求：关闭上一个未完成的交互
            if let Some(mut inter) = current.take() {
                if inter.final_code.is_empty() {
                    inter.final_code = "TIMEOUT".into();
                    inter.raw_lines.push("(无终结码，推断超时)".into());
                }
                result.push(inter);
            }
            current = Some(Interaction {
                ts: frame.ts,
                request: text.to_string(),
                final_code: String::new(),
                raw_lines: Vec::new(),
            });
        } else {
            // rx 行
            let final_code = classify_final(text);
            if let Some(inter) = current.as_mut() {
                if let Some(fc) = &final_code {
                    inter.final_code = fc.clone();
                    inter.raw_lines.push(text.to_string());
                } else {
                    inter.raw_lines.push(text.to_string());
                }
            } else {
                // 无挂起请求的 rx 行：可能是 URC，跳过
            }
        }
    }

    // 关闭最后一个
    if let Some(mut inter) = current {
        if inter.final_code.is_empty() {
            inter.final_code = "TIMEOUT".into();
            inter.raw_lines.push("(无终结码，推断超时)".into());
        }
        result.push(inter);
    }

    result
}

/// 从终结码提取错误类型和码值："+CME ERROR: 148" → ("cme", 148)
fn extract_error_code(final_code: &str) -> (Option<&str>, Option<u32>) {
    let upper = final_code.to_uppercase();
    if let Some(rest) = upper.strip_prefix("+CME ERROR:") {
        let code: String = rest.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
        return (Some("cme"), code.parse().ok());
    }
    if let Some(rest) = upper.strip_prefix("+CMS ERROR:") {
        let code: String = rest.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
        return (Some("cms"), code.parse().ok());
    }
    (None, None)
}

// ---------------------------------------------------------------- log 口分析

/// log 口分析报告
#[derive(Debug, Clone, Serialize)]
pub struct RawLogReport {
    pub total_lines: u32,
    pub matched_events: u32,
    pub events: Vec<LogEvent>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    pub timestamp: i64,
    pub module_id: String,
    pub module_name: String,
    pub level: String,
    pub severity: String,
    pub message: String,
    pub raw_line: String,
}

/// 分析 log 口文本行：用 LogPatternStore 做正则匹配。
pub fn analyze_raw_log(lines: &[LogFrame], patterns: &LogPatternStore) -> RawLogReport {
    let compiled = patterns.compiled_patterns();
    let mut events = Vec::new();
    let mut matched = 0u32;

    for frame in lines {
        let text = frame.text.trim();
        if text.is_empty() {
            continue;
        }
        for cp in &compiled {
            if cp.regex.is_match(text) {
                matched += 1;
                events.push(LogEvent {
                    timestamp: frame.ts,
                    module_id: cp.module_id.clone(),
                    module_name: cp.module_name.clone(),
                    level: cp.level.clone(),
                    severity: cp.severity.clone(),
                    message: cp.message.clone(),
                    raw_line: text.to_string(),
                });
                break; // 每行只取第一个命中
            }
        }
    }

    let total = lines.iter().filter(|f| !f.text.trim().is_empty()).count() as u32;
    let error_count = events.iter().filter(|e| e.severity == "error").count();
    let summary = if matched == 0 {
        format!("{total} 行日志，未匹配到已知模式")
    } else {
        format!("{total} 行日志，匹配 {matched} 条事件（{error_count} 条错误）")
    };

    RawLogReport {
        total_lines: total,
        matched_events: matched,
        events,
        summary,
    }
}

// ---------------------------------------------------------------- 统一报告

/// 日志分析统一报告（AT 口 + log 口）
#[derive(Debug, Clone, Serialize)]
pub struct LogAnalysisReport {
    pub at: Option<AtLogReport>,
    pub raw: Option<RawLogReport>,
}

// ---------------------------------------------------------------- 解析导出文件

/// 解析 export_session_log 导出的文本格式：[ts] >>/<< text
pub fn parse_exported_log(content: &str) -> Vec<LogFrame> {
    let mut frames = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // 格式: [1700000000000] >> AT+CSQ  或  [1700000000000] << +CSQ: 23,0
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                let ts_str = &rest[..end];
                let after = rest[end + 1..].trim();
                let ts = ts_str.parse::<i64>().unwrap_or(0);
                let (dir, text) = if after.starts_with(">>") {
                    ("tx", after[2..].trim())
                } else if after.starts_with("<<") {
                    ("rx", after[2..].trim())
                } else {
                    ("rx", after)
                };
                frames.push(LogFrame {
                    ts,
                    dir: dir.into(),
                    text: text.into(),
                });
            }
        }
    }
    frames
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    fn lf(ts: i64, dir: &str, text: &str) -> LogFrame {
        LogFrame {
            ts,
            dir: dir.into(),
            text: text.into(),
        }
    }

    fn no_lookup(_: &str, _: u32) -> Option<ErrorCodeEntry> {
        None
    }

    #[test]
    fn at_log_pairs_request_response() {
        let frames = vec![
            lf(100, "tx", "AT+CSQ"),
            lf(101, "rx", "+CSQ: 23,0"),
            lf(102, "rx", "OK"),
            lf(200, "tx", "AT+CGPADDR"),
            lf(201, "rx", "+CGPADDR: 1,\"10.1.2.3\""),
            lf(202, "rx", "OK"),
        ];
        let report = analyze_at_log(&frames, &no_lookup);
        assert_eq!(report.total_interactions, 2);
        assert_eq!(report.error_interactions, 0);
    }

    #[test]
    fn at_log_detects_cme_error() {
        let frames = vec![
            lf(100, "tx", "AT+CGACT=1,1"),
            lf(101, "rx", "+CME ERROR: 148"),
        ];
        let report = analyze_at_log(&frames, &no_lookup);
        assert_eq!(report.error_interactions, 1);
        assert_eq!(report.anomalies[0].kind, "cme_error");
        assert_eq!(report.anomalies[0].final_code, "+CME ERROR: 148");
    }

    #[test]
    fn at_log_detects_timeout() {
        let frames = vec![
            lf(100, "tx", "AT+MIPOPEN=0,\"TCP\",\"1.2.3.4\",80"),
            lf(200, "tx", "AT"), // 下一个请求，上一个无终结码
            lf(201, "rx", "OK"),
        ];
        let report = analyze_at_log(&frames, &no_lookup);
        assert_eq!(report.total_interactions, 2);
        assert_eq!(report.error_interactions, 1);
        assert_eq!(report.anomalies[0].kind, "timeout");
    }

    #[test]
    fn at_log_error_code_lookup() {
        let lookup = |ct: &str, code: u32| -> Option<ErrorCodeEntry> {
            if ct == "cme" && code == 148 {
                Some(ErrorCodeEntry {
                    code: 148,
                    meaning: "APN 错误".into(),
                    causes: vec!["APN 未配置".into()],
                    suggestions: vec!["检查 APN 设置".into()],
                    severity: "error".into(),
                    related_at: vec![],
                })
            } else {
                None
            }
        };
        let frames = vec![
            lf(100, "tx", "AT+CGACT=1,1"),
            lf(101, "rx", "+CME ERROR: 148"),
        ];
        let report = analyze_at_log(&frames, &lookup);
        assert_eq!(report.anomalies[0].error_code.as_ref().unwrap().meaning, "APN 错误");
    }

    #[test]
    fn raw_log_matches_pattern() {
        let patterns = LogPatternStore {
            modules: vec![LogPatternModule {
                id: "FTP".into(),
                name: "FTP".into(),
                patterns: vec![LogPattern {
                    message: "send error!".into(),
                    pattern: r"send\ error!".into(),
                    level: "error".into(),
                    count: 1,
                    severity: "error".into(),
                }],
            }],
        };
        let lines = vec![
            lf(100, "rx", "[FTP_LOGE] ftp_send(765) send error!"),
            lf(200, "rx", "[FTP_LOGI] ftp_send(700) ok"),
        ];
        let report = analyze_raw_log(&lines, &patterns);
        assert_eq!(report.matched_events, 1);
        assert_eq!(report.events[0].module_id, "FTP");
        assert_eq!(report.events[0].level, "error");
    }

    fn store_with(pattern: &str) -> LogPatternStore {
        LogPatternStore {
            modules: vec![LogPatternModule {
                id: "HTTP".into(),
                name: "HTTP".into(),
                patterns: vec![LogPattern {
                    message: "test".into(),
                    pattern: pattern.into(),
                    level: "error".into(),
                    count: 1,
                    severity: "error".into(),
                }],
            }],
        }
    }

    fn matches(pattern: &str, line: &str) -> bool {
        analyze_raw_log(&[lf(100, "rx", line)], &store_with(pattern)).matched_events == 1
    }

    /// 把模式库里的格式串还原成一条"模组真的会打出来"的日志行：
    /// 剥掉生成器加的 regex 转义，再把 `%s`/`%d` 换成代表性实参。
    fn synth_log_line(pattern: &str) -> String {
        let mut out = String::new();
        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]); // 去转义，还原字面量
                i += 2;
            } else if chars[i] == '%' && i + 1 < chars.len() && chars[i + 1] == 's' {
                out.push_str("cm_http_open");
                i += 2;
            } else if chars[i] == '%' && i + 1 < chars.len() && chars[i + 1] == 'd' {
                out.push_str("42");
                i += 2;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }

    /// 回归：模式库存的是 SDK 格式串，`%s`/`%d` 必须展开才能匹配真实日志。
    /// 修复前这些断言全部失败（模式只能匹配字面量 `%s`）。
    #[test]
    fn format_specifiers_match_substituted_log_lines() {
        assert!(matches(
            r"%s:client\ is\ not\ create",
            "[HTTP_LOGE] cm_http_send(233) cm_http_send:client is not create"
        ));
        assert!(matches(
            r"%s:cid\ %d\ do\ not\ active\ !",
            "[HTTP_LOGE] foo(1) cm_http_open:cid 3 do not active !"
        ));
        assert!(matches(r"conn\ err,%d", "[FTP_LOGE] ftp(9) conn err,-110"));
        assert!(matches(
            r"%s\(\)\ line:%d\ cm_malloc\ failed!",
            "cm_asocket_bind() line:412 cm_malloc failed!"
        ));
    }

    /// `%s` 展开为 `\S+` 而非 `.+`：不得跨空白吞掉整行造成误命中。
    #[test]
    fn format_specifiers_do_not_overmatch() {
        // %s 只吃一个非空白令牌，中间隔了空格的无关行不应命中
        assert!(!matches(
            r"%s:client\ is\ not\ create",
            "[HTTP_LOGI] send ok, client is created"
        ));
        // %d 只吃数字令牌
        assert!(!matches(r"conn\ err,%d", "[FTP_LOGI] conn err,timeout"));
        // 良性日志行不应被任何展开后的模式命中
        assert!(!matches(
            r"%s:param\ in\ error",
            "[MODEM_LOGI] cm_modem_init(120) modem init done"
        ));
    }

    /// `%%` 是字面百分号，不能当格式符展开。
    #[test]
    fn double_percent_is_literal() {
        assert!(matches(r"battery\ %d%%", "battery 87%"));
        assert!(!matches(r"battery\ %d%%", "battery 87"));
    }

    /// 末尾孤立的 `%` 不是格式符，按字面量处理且不得让正则编译失败。
    #[test]
    fn trailing_percent_is_literal() {
        assert!(matches(r"progress\ 100%", "progress 100%"));
    }

    /// 真实模式库回归：加载 data/log_patterns，确认展开后
    /// (1) 全部可编译；(2) 含格式符的模式确实存在且被展开；
    /// (3) 不会命中良性日志行。
    #[test]
    fn real_pattern_library_expands_and_stays_clean() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../data")
            .join("log_patterns");
        let store = LogPatternStore::load_from_dir(&dir);
        assert!(!store.modules.is_empty(), "模式库未加载");

        let total: usize = store.modules.iter().map(|m| m.patterns.len()).sum();
        let with_fmt = store
            .modules
            .iter()
            .flat_map(|m| &m.patterns)
            .filter(|p| p.pattern.contains("%s") || p.pattern.contains("%d"))
            .count();
        assert!(with_fmt > 0, "模式库应含格式符模式，否则本回归失去意义");

        // 展开后全部可编译：compiled_patterns 会静默丢弃编译失败的模式，
        // 所以用数量相等来断言"没有一条因展开而损坏"。
        assert_eq!(
            store.compiled_patterns().len(),
            total,
            "展开后有模式无法编译"
        );

        // 端到端：把每条格式串按实参替换合成"真实日志行"，展开后必须命中。
        // 这是本修复的核心保证——修复前这里会有 96 条失配。
        let mut unmatched = Vec::new();
        for module in &store.modules {
            for pat in &module.patterns {
                if !(pat.pattern.contains("%s") || pat.pattern.contains("%d")) {
                    continue;
                }
                let line = synth_log_line(&pat.pattern);
                let hit = analyze_raw_log(&[lf(1, "rx", &line)], &store).matched_events == 1;
                if !hit {
                    unmatched.push((pat.pattern.clone(), line));
                }
            }
        }
        assert!(
            unmatched.is_empty(),
            "{} 条格式串模式无法匹配替换后的日志行，例如: {:?}",
            unmatched.len(),
            &unmatched[..unmatched.len().min(3)]
        );

        // 良性日志行不应命中任何模式
        for benign in [
            "[MODEM_LOGI] cm_modem_init(120) modem init done",
            "[HTTP_LOGI] cm_http_send(233) send 1024 bytes ok",
            "AT+CSQ",
            "+CSQ: 23,0",
            "OK",
        ] {
            let report = analyze_raw_log(&[lf(1, "rx", benign)], &store);
            assert_eq!(
                report.matched_events, 0,
                "良性日志行被误命中: {benign} -> {:?}",
                report.events.first().map(|e| &e.message)
            );
        }
    }

    #[test]
    fn parse_exported_log_roundtrip() {
        let content = "[1700000000000] >> AT+CSQ\n[1700000000001] << +CSQ: 23,0\n[1700000000002] << OK\n";
        let frames = parse_exported_log(content);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].dir, "tx");
        assert_eq!(frames[0].text, "AT+CSQ");
        assert_eq!(frames[1].dir, "rx");
        assert_eq!(frames[2].dir, "rx");
    }
}