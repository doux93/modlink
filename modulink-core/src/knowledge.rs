//! 知识层：模组画像(profile) + 错误码表 的加载与检索。
//!
//! 这是 KnowledgeProvider 三级知识源中的 ②③ 级（本地快照 + 内置兜底）实现；
//! 后续内网知识库服务上线后，新增 HTTP Provider 即可，调用方不感知。
//! 所有文件为 YAML，FAE 直接编辑 data/ 目录即可扩展，不需要改代码。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------- 数据模型

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleProfile {
    pub schema: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub family: String,
    /// 可由 AT+CGMM 返回的精确型号编码，例如 ML307R。
    #[serde(default)]
    pub model_codes: Vec<String>,
    /// 经过验证可共享参考资料的产品系列，例如 ML307。
    #[serde(default)]
    pub series: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default = "default_support_level")]
    pub support_level: String,
    #[serde(default)]
    pub detect: Option<DetectRule>,
    #[serde(default)]
    pub ports: Vec<PortRole>,
    #[serde(default = "default_line_ending")]
    pub line_ending: String,
    #[serde(default)]
    pub urc_patterns: Vec<UrcPattern>,
    #[serde(default)]
    pub at_commands: Vec<AtCommandDoc>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub error_code_tables: Vec<String>,
    #[serde(default)]
    pub intents: Vec<Intent>,
}

#[derive(Debug, Deserialize)]
struct ModuleProfileCatalog {
    #[allow(dead_code)]
    schema: String,
    #[serde(default)]
    defaults: serde_yaml::Mapping,
    #[serde(default)]
    detect: Option<CatalogDetectRule>,
    profiles: Vec<serde_yaml::Mapping>,
}

#[derive(Debug, Deserialize)]
struct CatalogDetectRule {
    command: String,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    allow_hyphen_suffix: bool,
}

fn default_line_ending() -> String {
    "\r\n".into()
}

fn default_support_level() -> String {
    "unspecified".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectRule {
    pub command: String,
    pub r#match: String,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortRole {
    pub role: String,
    #[serde(default)]
    pub default_baud: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrcPattern {
    pub id: String,
    pub pattern: String,
    pub label: String,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub severity_field: Option<String>,
    #[serde(default)]
    pub severity_map: HashMap<String, String>,
    #[serde(default)]
    pub severity_default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtCommandDoc {
    pub cmd: String,
    #[serde(default)]
    pub syntax: String,
    #[serde(default)]
    pub category: String,
    #[serde(default = "default_risk")]
    pub risk: String,
    #[serde(default)]
    pub doc: String,
    #[serde(default)]
    pub examples: Vec<String>,
}

fn default_risk() -> String {
    "none".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub phrases: Vec<String>,
    pub sequence: Vec<String>,
}

/// 批量导入的 AT 指令文档（来自知识库转换器 tools/kb_convert.py）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtDocsFile {
    pub schema: String,
    pub protocol: String,
    #[serde(default)]
    pub applies_to: serde_yaml::Value,
    #[serde(default)]
    pub source: String,
    pub commands: Vec<ExtendedAtDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedAtDoc {
    pub cmd: String,
    #[serde(default)]
    pub syntax: String,
    #[serde(default)]
    pub category: String,
    /// command | urc
    #[serde(default)]
    pub kind: String,
    #[serde(default = "default_risk")]
    pub risk: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub doc: String,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub source: String,
    /// 从原始手册标题推断出的适用型号；空数组表示通用资料。
    #[serde(default)]
    pub models: Vec<String>,
    /// 同一指令可能出现在多本手册中，保留全部来源用于追溯。
    #[serde(default)]
    pub sources: Vec<String>,
    /// 手册内部产品分支，例如 nb-iot。
    #[serde(default)]
    pub variants: Vec<String>,
    /// Canonical topic evidence and DITAMAP ownership (V3 compiler).
    #[serde(default)]
    pub topic_id: String,
    #[serde(default)]
    pub map_id: String,
    #[serde(default)]
    pub scope_source: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentApplicability {
    Exact,
    Series,
    Generic,
    Unknown,
    Incompatible,
}

impl DocumentApplicability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Series => "series",
            Self::Generic => "generic",
            Self::Unknown => "unknown",
            Self::Incompatible => "incompatible",
        }
    }

    pub fn score_bonus(self) -> i32 {
        match self {
            Self::Exact => 20,
            Self::Series => 10,
            Self::Generic | Self::Unknown | Self::Incompatible => 0,
        }
    }
}

/// 文档适用性按“精确型号 > 同系列 > 通用”分级。
/// 未选择画像、画像未知或选择通用画像时，只允许通用资料。
pub fn document_applicability(
    profile: Option<&ModuleProfile>,
    models: &[String],
) -> DocumentApplicability {
    if models.is_empty() {
        return DocumentApplicability::Generic;
    }
    let Some(profile) = profile else {
        return DocumentApplicability::Incompatible;
    };
    if profile.id.eq_ignore_ascii_case("generic") || profile.support_level == "generic" {
        return DocumentApplicability::Incompatible;
    }

    if models.iter().any(|model| {
        profile
            .model_codes
            .iter()
            .any(|code| code.eq_ignore_ascii_case(model))
    }) {
        return DocumentApplicability::Exact;
    }

    let series = profile.series.trim().to_ascii_uppercase();
    if !series.is_empty()
        && models.iter().any(|model| {
            let model = model.trim().to_ascii_uppercase();
            model == series || model.starts_with(&series)
        })
    {
        return DocumentApplicability::Series;
    }

    DocumentApplicability::Incompatible
}

/// 诊断流程/典型用例对画像的适用性判定——前后端唯一权威实现。
///
/// 体检流程（kind != "usecase"）：
/// - 画像 `diagnostics` 白名单非空时，白名单完全决定（显式引用即授权，
///   不再看 applies_to——体检流程的 applies_to 用的是 family 值如 "cat1"，
///   与型号键体系不同，显式引用优先避免两套语义打架）；
/// - 白名单为空时回退 applies_to：空 = 全适用；否则按 family 精确匹配。
///
/// 典型用例（kind == "usecase"）：
/// - `applies_to` 为空 → 通用参考（任何画像可见，UI 单独分组提示）；
/// - 匹配键：画像 id / model_codes / series（family 不是型号前缀，不参与）。
///   命中 = 用例型号与 key 相等，或以 key 为前缀（"ml307c-dl" 匹配 "ml307c"）。
///   不做反向前缀，避免画像 "ml307" 误吞其他型号。
pub fn flow_applies(flow: &crate::diag::DiagFlow, profile: Option<&ModuleProfile>) -> bool {
    let Some(profile) = profile else { return true };

    if flow.kind != "usecase" {
        if !profile.diagnostics.is_empty() {
            return profile.diagnostics.iter().any(|id| id == &flow.id);
        }
        if flow.applies_to.is_empty() {
            return true;
        }
        let family = profile.family.to_lowercase();
        let id = profile.id.to_lowercase();
        return flow
            .applies_to
            .iter()
            .any(|t| t.to_lowercase() == family || t.to_lowercase() == id);
    }

    if flow.applies_to.is_empty() {
        return true;
    }
    let keys: Vec<String> = std::iter::once(profile.id.clone())
        .chain(profile.model_codes.iter().cloned())
        .chain(std::iter::once(profile.series.clone()))
        .filter(|k| !k.trim().is_empty())
        .map(|k| k.to_lowercase())
        .collect();
    flow.applies_to.iter().any(|target| {
        let t = target.to_lowercase();
        keys.iter().any(|k| t == *k || t.starts_with(k.as_str()))
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorCodeTable {
    pub schema: String,
    pub code_type: String,
    #[serde(default)]
    pub applies_to: serde_yaml::Value,
    pub entries: Vec<ErrorCodeEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorCodeEntry {
    pub code: u32,
    pub meaning: String,
    #[serde(default)]
    pub causes: Vec<String>,
    #[serde(default)]
    pub suggestions: Vec<String>,
    #[serde(default = "default_severity")]
    pub severity: String,
    #[serde(default)]
    pub related_at: Vec<String>,
}

fn default_severity() -> String {
    "error".into()
}

// ---------------------------------------------------------------- 知识库

#[derive(Debug, Default, Clone, Serialize)]
pub struct KnowledgeStore {
    pub profiles: Vec<ModuleProfile>,
    /// (code_type, code) -> entry；表加载顺序决定同码覆盖
    pub error_codes: HashMap<String, ErrorCodeEntry>,
    /// 按画像引用的错误码表，key = profile id。
    pub profile_error_codes: HashMap<String, HashMap<String, ErrorCodeEntry>>,
    /// 诊断流程（data/diagnostics/*.yaml）
    pub flows: Vec<crate::diag::DiagFlow>,
    /// 批量指令文档（data/at_docs/*.yaml，知识库转换产物）
    pub at_docs: Vec<AtDocsFile>,
    /// log 口日志模式知识库（data/log_patterns/*.yaml）
    pub log_patterns: crate::log_analysis::LogPatternStore,
    /// 加载过程中的告警（文件损坏等，不中断）
    pub warnings: Vec<String>,
}

impl KnowledgeStore {
    /// 从数据目录加载：profiles/*.yaml + error_codes/*.yaml。
    /// 单个文件解析失败只记 warning，不影响其余文件。
    /// 加载资源目录 + 用户数据目录。用户目录当前只存放手工创建的用例
    /// （usecases/my_*.yaml）——它必须与只读的安装资源分离，且不能被
    /// 构建时的资源副本清理（clean-stale-resources）波及。
    pub fn load_with_user_dir(data_dir: &Path, user_dir: &Path) -> Self {
        let mut store = Self::load_from_dir(data_dir);
        for entry in read_yaml_files(&user_dir.join("usecases")) {
            match serde_yaml::from_str::<crate::diag::DiagFlow>(&entry.1) {
                Ok(f) => {
                    let mut f = f;
                    if f.execution_mode == "unknown" {
                        f.execution_mode = crate::diag::infer_execution_mode(&f);
                    }
                    if store.flows.iter().any(|existing| existing.id == f.id) {
                        store
                            .warnings
                            .push(format!("用户用例与内置流程 id 冲突: {}", f.id));
                        continue;
                    }
                    let issues = crate::diag::validate_flow(&f);
                    if issues.is_empty() {
                        store.flows.push(f);
                    } else {
                        store
                            .warnings
                            .push(format!("{}: {}", entry.0, issues.join("; ")));
                    }
                }
                Err(err) => store.warnings.push(format!("{}: {}", entry.0, err)),
            }
        }
        store
    }

    pub fn load_from_dir(data_dir: &Path) -> Self {
        let mut store = KnowledgeStore::default();
        let mut error_tables: HashMap<String, HashMap<String, ErrorCodeEntry>> = HashMap::new();

        for entry in read_yaml_files(&data_dir.join("error_codes")) {
            match serde_yaml::from_str::<ErrorCodeTable>(&entry.1) {
                Ok(table) => {
                    let table_id = Path::new(&entry.0)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let mut table_entries = HashMap::new();
                    for e in table.entries {
                        let key = format!("{}:{}", table.code_type, e.code);
                        table_entries.insert(key.clone(), e.clone());
                        store.error_codes.insert(key, e);
                    }
                    error_tables.insert(table_id, table_entries);
                }
                Err(err) => store.warnings.push(format!("{}: {}", entry.0, err)),
            }
        }

        for entry in read_yaml_files(&data_dir.join("profiles")) {
            match parse_profile_file(&entry.1) {
                Ok(profiles) => {
                    for p in profiles {
                        if store.profiles.iter().any(|existing| existing.id == p.id) {
                            store.warnings.push(format!("重复画像 id: {}", p.id));
                        }
                        for model in &p.model_codes {
                            if store.profiles.iter().any(|existing| {
                                existing
                                    .model_codes
                                    .iter()
                                    .any(|code| code.eq_ignore_ascii_case(model))
                            }) {
                                store.warnings.push(format!("重复画像型号: {model}"));
                            }
                        }
                        if let Some(detect) = &p.detect {
                            if regex::Regex::new(&detect.r#match).is_err() {
                                store.warnings.push(format!("{}: 识别正则无效", p.id));
                            }
                        }
                        for urc in &p.urc_patterns {
                            if regex::Regex::new(&urc.pattern).is_err() {
                                store
                                    .warnings
                                    .push(format!("{}:{}: URC 正则无效", p.id, urc.id));
                            }
                        }
                        store.profiles.push(p)
                    }
                }
                Err(err) => store.warnings.push(format!("{}: {}", entry.0, err)),
            }
        }

        // 体检流程 + 典型用例（手册示例转换产物）共用 DiagFlow 结构与校验
        let flow_files = read_yaml_files(&data_dir.join("diagnostics"))
            .into_iter()
            .chain(read_yaml_files(&data_dir.join("usecases")));
        for entry in flow_files {
            match serde_yaml::from_str::<crate::diag::DiagFlow>(&entry.1) {
                Ok(f) => {
                    let mut f = f;
                    if f.execution_mode == "unknown" {
                        f.execution_mode = crate::diag::infer_execution_mode(&f);
                    }
                    if store.flows.iter().any(|existing| existing.id == f.id) {
                        store.warnings.push(format!("重复诊断流程 id: {}", f.id));
                    }
                    let mut step_ids = std::collections::HashSet::new();
                    for step in &f.steps {
                        if !step_ids.insert(&step.id) {
                            store
                                .warnings
                                .push(format!("{}: 重复步骤 id {}", f.id, step.id));
                        }
                        if let Some(parse) = &step.parse {
                            if regex::Regex::new(parse).is_err() {
                                store
                                    .warnings
                                    .push(format!("{}:{}: 解析正则无效", f.id, step.id));
                            }
                        }
                        if let Some(retry) = &step.retry {
                            if retry.times == 0 || !crate::diag::validate_expr(&retry.until) {
                                store
                                    .warnings
                                    .push(format!("{}:{}: 重试规则无效", f.id, step.id));
                            }
                        }
                        for rule in &step.evaluate {
                            if !matches!(rule.result.as_str(), "pass" | "warn" | "fail") {
                                store.warnings.push(format!(
                                    "{}:{}: 未知结果 {}",
                                    f.id, step.id, rule.result
                                ));
                            }
                            if let Some(condition) = &rule.when {
                                if !crate::diag::validate_expr(condition) {
                                    store
                                        .warnings
                                        .push(format!("{}:{}: 条件表达式无效", f.id, step.id));
                                }
                            }
                        }
                    }
                    store.flows.push(f)
                }
                Err(err) => store.warnings.push(format!("{}: {}", entry.0, err)),
            }
        }

        for entry in read_yaml_files(&data_dir.join("at_docs")) {
            match serde_yaml::from_str::<AtDocsFile>(&entry.1) {
                Ok(d) => store.at_docs.push(d),
                Err(err) => store.warnings.push(format!("{}: {}", entry.0, err)),
            }
        }
        for profile in &store.profiles {
            let mut codes = HashMap::new();
            for table_id in &profile.error_code_tables {
                match error_tables.get(table_id) {
                    Some(table) => codes.extend(table.clone()),
                    None => store
                        .warnings
                        .push(format!("{}: 引用不存在的错误码表 {}", profile.id, table_id)),
                }
            }
            if codes.is_empty() {
                codes = store.error_codes.clone();
            }
            store.profile_error_codes.insert(profile.id.clone(), codes);
        }
        store
            .profiles
            .sort_by_key(|p| -(p.detect.as_ref().map(|d| d.priority).unwrap_or(0)));
        store.log_patterns =
            crate::log_analysis::LogPatternStore::load_from_dir(&data_dir.join("log_patterns"));
        store
    }

    pub fn lookup_error_code(&self, code_type: &str, code: u32) -> Option<&ErrorCodeEntry> {
        self.error_codes.get(&format!("{code_type}:{code}"))
    }

    pub fn profile(&self, id: &str) -> Option<&ModuleProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }
}

fn parse_profile_file(content: &str) -> Result<Vec<ModuleProfile>, String> {
    let value =
        serde_yaml::from_str::<serde_yaml::Value>(content).map_err(|err| err.to_string())?;
    let schema = value
        .as_mapping()
        .and_then(|mapping| mapping.get(&serde_yaml::Value::String("schema".into())))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or_default();

    if schema != "modulink/profile-catalog@1" {
        return serde_yaml::from_value::<ModuleProfile>(value)
            .map(|profile| vec![profile])
            .map_err(|err| err.to_string());
    }

    let catalog =
        serde_yaml::from_value::<ModuleProfileCatalog>(value).map_err(|err| err.to_string())?;
    let model_codes: Vec<String> = catalog
        .profiles
        .iter()
        .filter_map(|profile| {
            profile
                .get(&serde_yaml::Value::String("model_codes".into()))?
                .as_sequence()?
                .first()?
                .as_str()
                .map(str::to_ascii_uppercase)
        })
        .collect();
    let defaults = catalog.defaults;
    let default_detect = catalog.detect;
    catalog
        .profiles
        .into_iter()
        .map(|profile| {
            let mut merged = defaults.clone();
            for (key, value) in profile {
                merged.insert(key, value);
            }
            let mut profile =
                serde_yaml::from_value::<ModuleProfile>(serde_yaml::Value::Mapping(merged))
                    .map_err(|err| err.to_string())?;
            if profile.detect.is_none() {
                if let Some(rule) = &default_detect {
                    let model = profile
                        .model_codes
                        .first()
                        .ok_or_else(|| format!("{}: 目录画像缺少 model_codes", profile.id))?;
                    if !model
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
                    {
                        return Err(format!("{}: 型号包含不支持的识别字符", profile.id));
                    }
                    let model = model.to_ascii_uppercase();
                    let has_hyphen_child = model_codes
                        .iter()
                        .any(|candidate| candidate.starts_with(&format!("{model}-")));
                    let suffix = if rule.allow_hyphen_suffix && !has_hyphen_child {
                        "(?:-[A-Z0-9]+)*"
                    } else {
                        ""
                    };
                    profile.detect = Some(DetectRule {
                        command: rule.command.clone(),
                        r#match: format!("(?:^|[^A-Z0-9-]){}{}(?:[^A-Z0-9-]|$)", model, suffix),
                        priority: rule.priority,
                    });
                }
            }
            Ok(profile)
        })
        .collect()
}

fn read_yaml_files(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        let is_yaml = p
            .extension()
            .map(|x| x == "yaml" || x == "yml")
            .unwrap_or(false);
        if is_yaml {
            if let Ok(content) = std::fs::read_to_string(&p) {
                out.push((p.display().to_string(), content));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 从 "+CME ERROR: 148" / "+CMS ERROR: 300" 提取 (code_type, code)。
pub fn parse_error_final(line: &str) -> Option<(String, u32)> {
    let t = line.trim();
    for (prefix, ty) in [("+CME ERROR:", "cme"), ("+CMS ERROR:", "cms")] {
        if let Some(rest) = t.strip_prefix(prefix) {
            if let Ok(code) = rest.trim().parse::<u32>() {
                return Some((ty.to_string(), code));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../data")
    }

    #[test]
    fn load_real_data_dir() {
        let store = KnowledgeStore::load_from_dir(&data_dir());
        assert!(store.warnings.is_empty(), "warnings: {:?}", store.warnings);
        assert!(!store.profiles.is_empty(), "no profiles loaded");
        let p = store.profile("ml307r").expect("ml307r profile");
        assert_eq!(p.family, "cat1");
        assert_eq!(p.model_codes, ["ML307R"]);
        assert_eq!(p.series, "ML307");
        assert_eq!(p.support_level, "verified");
        assert!(!p.urc_patterns.is_empty());
        assert!(!p.at_commands.is_empty());
        // 高危指令标注存在
        assert!(p.at_commands.iter().any(|c| c.risk == "reboot"));

        assert_eq!(store.profiles.len(), 41);
        let reference = store.profile("mn328_brz15").expect("catalog profile");
        assert_eq!(reference.model_codes, ["MN328-BRZ15"]);
        assert_eq!(reference.support_level, "reference");
        assert!(!reference.urc_patterns.is_empty());
        assert!(!reference.at_commands.is_empty());
    }

    #[test]
    fn at_docs_loaded_from_kb_conversion() {
        let store = KnowledgeStore::load_from_dir(&data_dir());
        assert!(store.warnings.is_empty(), "warnings: {:?}", store.warnings);
        assert!(!store.at_docs.is_empty(), "at_docs 未加载");
        let all: Vec<&ExtendedAtDoc> = store.at_docs.iter().flat_map(|d| &d.commands).collect();
        let mqttcfg = all
            .iter()
            .find(|c| c.cmd == "AT+MQTTCFG" && !c.models.is_empty())
            .expect("model-scoped AT+MQTTCFG");
        assert!(mqttcfg.doc.contains("MQTT"), "doc 内容缺失");
        assert!(!mqttcfg.models.is_empty(), "型号适用范围缺失");
        assert!(!mqttcfg.sources.is_empty(), "来源列表缺失");
        // URC 条目也应转入
        assert!(all.iter().any(|c| c.kind == "urc"));
    }

    #[test]
    fn detect_rules_allow_product_suffixes_without_cross_model_matches() {
        let store = KnowledgeStore::load_from_dir(&data_dir());
        let cases = [
            ("ML302", "ml302"),
            ("+CGMM: ML302A-DL", "ml302a"),
            ("ML305U-GC", "ml305u"),
            ("ML307C-DL", "ml307c"),
            ("ML307R-DL", "ml307r"),
            ("MN316-S-DL", "mn316_s"),
            ("MN328-BRZ15-AZ", "mn328_brz15"),
        ];

        for (response, expected) in cases {
            let matches: Vec<&str> = store
                .profiles
                .iter()
                .filter(|profile| {
                    profile.detect.as_ref().is_some_and(|detect| {
                        regex::Regex::new(&detect.r#match)
                            .is_ok_and(|pattern| pattern.is_match(response))
                    })
                })
                .map(|profile| profile.id.as_str())
                .collect();
            assert_eq!(matches, [expected], "response: {response}");
        }

        assert_eq!(
            store
                .profile("ml302")
                .unwrap()
                .detect
                .as_ref()
                .unwrap()
                .priority,
            20
        );
        assert_eq!(
            store
                .profile("ml302a")
                .unwrap()
                .detect
                .as_ref()
                .unwrap()
                .priority,
            20
        );
    }

    #[test]
    fn error_code_lookup() {
        let store = KnowledgeStore::load_from_dir(&data_dir());
        let e = store.lookup_error_code("cme", 10).expect("cme 10");
        assert!(e.meaning.contains("SIM"));
        let e148 = store.lookup_error_code("cme", 148).expect("cme 148");
        assert!(!e148.suggestions.is_empty());
        assert!(store.lookup_error_code("cme", 99999).is_none());
    }

    #[test]
    fn parse_error_lines() {
        assert_eq!(
            parse_error_final("+CME ERROR: 148"),
            Some(("cme".into(), 148))
        );
        assert_eq!(
            parse_error_final("+CMS ERROR: 300"),
            Some(("cms".into(), 300))
        );
        assert_eq!(parse_error_final("ERROR"), None);
        assert_eq!(parse_error_final("+CSQ: 20,0"), None);
    }

    #[test]
    fn urc_pattern_regexes_are_valid() {
        // 画像里的每条正则都必须能被前端 JS RegExp 与 Rust regex 兼容编译；
        // 这里至少验证 Rust 侧（命名捕获语法 (?<name>...) 两边通用）
        let store = KnowledgeStore::load_from_dir(&data_dir());
        for p in &store.profiles {
            for u in &p.urc_patterns {
                // 简单结构校验（不引 regex crate，编译校验交给使用方）
                assert!(!u.pattern.is_empty(), "{}: empty pattern", u.id);
                assert!(
                    u.severity.is_some()
                        || u.severity_field.is_some()
                        || u.severity_default.is_some(),
                    "{}: no severity rule",
                    u.id
                );
            }
        }
    }

    #[test]
    fn flow_applies_unifies_checkup_and_usecase_rules() {
        let profile: ModuleProfile = serde_yaml::from_str(
            r#"
schema: modulink/profile@1
id: ml307r
name: ML307R
family: cat1
model_codes: [ML307R]
series: ML307
support_level: verified
diagnostics: [network_attach]
"#,
        )
        .expect("profile");

        let mk = |kind: &str, id: &str, applies: &[&str]| crate::diag::DiagFlow {
            schema: "modulink/diagflow@1".into(),
            id: id.into(),
            name: id.into(),
            applies_to: applies.iter().map(|s| s.to_string()).collect(),
            kind: kind.into(),
            category: String::new(),
            source: String::new(),
            verified: true,
            execution_mode: "unknown".into(),
            steps: Vec::new(),
        };

        // 体检流程：白名单非空时完全由白名单决定
        assert!(flow_applies(
            &mk("checkup", "network_attach", &["cat9"]),
            Some(&profile)
        ));
        assert!(!flow_applies(
            &mk("checkup", "other_flow", &["cat1"]),
            Some(&profile)
        ));
        // 用例：applies_to 空 = 通用放行
        assert!(flow_applies(&mk("usecase", "uc1", &[]), Some(&profile)));
        // 用例：前缀单向匹配（型号带后缀命中画像 key；反向不吞）
        assert!(flow_applies(
            &mk("usecase", "uc2", &["ml307r-dl"]),
            Some(&profile)
        ));
        assert!(flow_applies(
            &mk("usecase", "uc3", &["ml307c"]),
            Some(&profile)
        )); // series 前缀
        assert!(!flow_applies(
            &mk("usecase", "uc4", &["mr880a"]),
            Some(&profile)
        ));
        // family 不参与用例匹配
        assert!(!flow_applies(
            &mk("usecase", "uc5", &["cat1"]),
            Some(&profile)
        ));
        // 无画像全放行
        assert!(flow_applies(&mk("usecase", "uc6", &["mr880a"]), None));
    }

    #[test]
    fn document_scope_prefers_exact_then_series_and_rejects_other_models() {
        let profile: ModuleProfile = serde_yaml::from_str(
            r#"
schema: modulink/profile@1
id: ml307r
name: ML307R
model_codes: [ML307R]
series: ML307
support_level: verified
"#,
        )
        .expect("profile");

        assert_eq!(
            document_applicability(Some(&profile), &["ML307R".into()]),
            DocumentApplicability::Exact
        );
        assert_eq!(
            document_applicability(Some(&profile), &["ML307C".into()]),
            DocumentApplicability::Series
        );
        assert_eq!(
            document_applicability(Some(&profile), &[]),
            DocumentApplicability::Generic
        );
        assert_eq!(
            document_applicability(Some(&profile), &["MR880A".into()]),
            DocumentApplicability::Incompatible
        );
        assert_eq!(
            document_applicability(None, &["ML307R".into()]),
            DocumentApplicability::Incompatible
        );
    }
}
