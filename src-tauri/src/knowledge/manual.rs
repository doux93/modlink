use crate::app_paths::AppPaths;
use crate::config::load_settings;
use crate::knowledge::pack::{validate_pack_cached, validation_receipt_path};
use crate::types::{Knowledge, KnowledgePackManifest, KnowledgePackValidation, ManualContentLevel};
use modulink_core::knowledge::{document_applicability, DocumentApplicability};
use parking_lot::RwLock;
use std::sync::Arc;
use tauri::State;

// ---------------------------------------------------------------- 手册检索
#[derive(serde::Deserialize, serde::Serialize, Clone, Default)]
pub(crate) struct TopicMeta {
    pub(crate) topic_id: String,
    pub(crate) title: String,
    pub(crate) manual: String,
    pub(crate) kind: String,
    pub(crate) preview: String,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    /// 手册内部产品分支，例如 nb-iot；避免无型号主题被误当成全制式 generic。
    #[serde(default)]
    pub(crate) variants: Vec<String>,
    #[serde(default)]
    pub(crate) scope_state: String,
    #[serde(default)]
    pub(crate) language: String,
    #[serde(default)]
    pub(crate) content_hash: String,
    #[serde(default)]
    pub(crate) commands: Vec<String>,
    #[serde(default)]
    pub(crate) owners: Vec<TopicOwnerMeta>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Default)]
pub(crate) struct TopicOwnerMeta {
    pub(crate) map_id: String,
    #[serde(default)]
    pub(crate) manual: String,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) scope_source: String,
    #[serde(default)]
    pub(crate) language: String,
}

pub(crate) fn owner_applicability(
    owner: &TopicOwnerMeta,
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
) -> DocumentApplicability {
    if owner.scope_source.eq_ignore_ascii_case("unknown") || owner.models.is_empty() {
        DocumentApplicability::Unknown
    } else {
        document_applicability(profile, &owner.models)
    }
}

pub(crate) fn topic_applicability<'a>(
    topic: &'a TopicMeta,
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
) -> (DocumentApplicability, Option<&'a TopicOwnerMeta>) {
    if topic.owners.is_empty() {
        return (document_applicability(profile, &topic.models), None);
    }
    for wanted in [
        DocumentApplicability::Exact,
        DocumentApplicability::Series,
        DocumentApplicability::Generic,
        DocumentApplicability::Unknown,
    ] {
        if let Some(owner) = topic
            .owners
            .iter()
            .find(|owner| owner_applicability(owner, profile) == wanted)
        {
            return (wanted, Some(owner));
        }
    }
    (DocumentApplicability::Incompatible, None)
}

pub(crate) fn topic_variant_compatible(
    topic: &TopicMeta,
    relative_path: Option<&str>,
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
) -> bool {
    let path = relative_path.unwrap_or("").to_ascii_lowercase();
    let requires_nb_iot = topic.variants.iter().any(|variant| {
        variant.eq_ignore_ascii_case("nb-iot") || variant.eq_ignore_ascii_case("nbiot")
    }) || path.contains("nb_topics")
        || path.contains("common_basic_topics");
    if !requires_nb_iot {
        return true;
    }
    profile
        .map(|profile| {
            profile.family.eq_ignore_ascii_case("nb-iot")
                || profile
                    .capabilities
                    .iter()
                    .any(|capability| capability.eq_ignore_ascii_case("nb-iot"))
        })
        .unwrap_or(false)
}

#[derive(Clone)]
pub(crate) struct ManualIndex(pub std::sync::Arc<RwLock<Option<Arc<Vec<TopicMeta>>>>>);

/// 知识包验证结果缓存，避免每次查询能力都全量遍历万条映射。
pub(crate) struct CachedPackValidation {
    pub(crate) pack_dir: String,
    pub(crate) manifest_stamp: (u64, u64),
    pub(crate) topic_paths_stamp: (u64, u64),
    pub(crate) validation: KnowledgePackValidation,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackValidationReceipt {
    pub(crate) pack_dir: String,
    pub(crate) manifest_stamp: (u64, u64),
    pub(crate) topic_paths_stamp: (u64, u64),
    pub(crate) validation: KnowledgePackValidation,
}

#[derive(Clone)]
pub(crate) struct PackValidationCache(pub std::sync::Arc<RwLock<Option<CachedPackValidation>>>);

pub(crate) const KNOWLEDGE_PACK_V1: &str = "modulink/knowledge-pack@1";
pub(crate) const KNOWLEDGE_PACK_V2: &str = "modulink/knowledge-pack@2";
pub(crate) const KNOWLEDGE_PACK_V3: &str = "modulink/knowledge-pack@3";

pub(crate) fn normalize_model_code(model: &str) -> String {
    model.trim().to_ascii_uppercase()
}

/// 当前型号目录均使用“两位字母 + 数字系列 + 型号后缀”，如 ML307C -> ML307。
pub(crate) fn model_series(model: &str) -> Option<String> {
    let model = normalize_model_code(model);
    let bytes = model.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || !bytes[1].is_ascii_alphabetic() {
        return None;
    }
    let digit_end = bytes[2..]
        .iter()
        .position(|b| !b.is_ascii_digit())
        .map(|offset| offset + 2)
        .unwrap_or(bytes.len());
    if digit_end == 2 {
        None
    } else {
        Some(model[..digit_end].to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackTopicScope {
    Exact,
    Series,
    Generic,
    Unknown,
}

pub(crate) fn topic_pack_scope(
    topic: &TopicMeta,
    manifest: &KnowledgePackManifest,
) -> Option<PackTopicScope> {
    if manifest.schema == KNOWLEDGE_PACK_V1 {
        return Some(PackTopicScope::Exact);
    }
    let selected: std::collections::HashSet<String> = manifest
        .models
        .iter()
        .map(|model| normalize_model_code(model))
        .collect();
    let model_sets: Vec<&[String]> = if topic.owners.is_empty() {
        vec![topic.models.as_slice()]
    } else {
        topic
            .owners
            .iter()
            .filter(|owner| {
                !owner.scope_source.eq_ignore_ascii_case("unknown") && !owner.models.is_empty()
            })
            .map(|owner| owner.models.as_slice())
            .collect()
    };
    if model_sets.iter().any(|models| {
        models
            .iter()
            .any(|model| selected.contains(&normalize_model_code(model)))
    }) {
        return Some(PackTopicScope::Exact);
    }
    if manifest.include_series {
        let selected_series: std::collections::HashSet<String> = selected
            .iter()
            .filter_map(|model| model_series(model))
            .collect();
        if model_sets.iter().any(|models| {
            models.iter().any(|model| {
                model_series(model)
                    .map(|series| selected_series.contains(&series))
                    .unwrap_or(false)
            })
        }) {
            return Some(PackTopicScope::Series);
        }
    }
    let has_unknown_owner = if topic.owners.is_empty() {
        topic.models.is_empty()
    } else {
        topic.owners.iter().any(|owner| {
            owner.scope_source.eq_ignore_ascii_case("unknown") || owner.models.is_empty()
        })
    };
    if manifest.schema == KNOWLEDGE_PACK_V3
        && manifest.include_unknown_reference
        && has_unknown_owner
    {
        return Some(PackTopicScope::Unknown);
    }
    if manifest.schema == KNOWLEDGE_PACK_V2 && manifest.include_generic && topic.models.is_empty() {
        return Some(PackTopicScope::Generic);
    }
    None
}

pub(crate) fn topic_matches_pack_scope(
    topic: &TopicMeta,
    manifest: &KnowledgePackManifest,
) -> bool {
    topic_pack_scope(topic, manifest).is_some()
}

/// @2 型号包只有在当前画像精确命中 manifest.models 时才允许读取全文。
/// 同系列内容是“为已声明型号提供参考”，不能反向授权未声明的兄弟型号。
pub(crate) fn pack_supports_profile(
    manifest: &KnowledgePackManifest,
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
) -> bool {
    if manifest.schema == KNOWLEDGE_PACK_V1 {
        return true;
    }
    let Some(profile) = profile else {
        return false;
    };
    manifest.models.iter().any(|model| {
        profile
            .model_codes
            .iter()
            .any(|code| code.eq_ignore_ascii_case(model.trim()))
    })
}

pub(crate) fn determine_effective_manual_mode(
    manual_mode: &ManualContentLevel,
    degradation_reason: Option<&str>,
    manifest: Option<&KnowledgePackManifest>,
    effective_profile_id: Option<&str>,
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
) -> (ManualContentLevel, Option<String>) {
    match manual_mode {
        ManualContentLevel::None => (
            ManualContentLevel::None,
            Some(degradation_reason.unwrap_or("手册索引未生成").to_string()),
        ),
        ManualContentLevel::Preview => (
            ManualContentLevel::Preview,
            degradation_reason.map(str::to_string),
        ),
        ManualContentLevel::FullText
            if manifest.is_some_and(|m| pack_supports_profile(m, profile)) =>
        {
            (ManualContentLevel::FullText, None)
        }
        ManualContentLevel::FullText if effective_profile_id.is_none() => (
            ManualContentLevel::Preview,
            Some("尚未选择设备画像，型号手册全文暂不启用".into()),
        ),
        ManualContentLevel::FullText if profile.is_none() => (
            ManualContentLevel::Preview,
            Some("当前设备画像不存在，已降级为内置摘要".into()),
        ),
        ManualContentLevel::FullText
            if profile.is_some_and(|profile| profile.id.eq_ignore_ascii_case("generic")) =>
        {
            (
                ManualContentLevel::Preview,
                Some("尚未识别具体型号，型号手册全文暂不启用".into()),
            )
        }
        ManualContentLevel::FullText => (
            ManualContentLevel::Preview,
            Some("当前型号不在已激活全文知识包的覆盖范围内，已降级为内置摘要".into()),
        ),
    }
}

pub(crate) use modulink_core::preview::sanitize_preview;

pub(crate) fn manual_index(paths: &AppPaths, state: &ManualIndex) -> Arc<Vec<TopicMeta>> {
    if let Some(index) = state.0.read().as_ref() {
        return index.clone();
    }
    let path = paths
        .resource_data_dir
        .join("kb_index")
        .join("manual_topics_public.jsonl");
    let topics: Vec<TopicMeta> = std::fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .filter_map(|line| {
                    let mut t: TopicMeta = serde_json::from_str(line).ok()?;
                    t.preview = sanitize_preview(&t.preview);
                    Some(t)
                })
                .collect()
        })
        .unwrap_or_default();
    let index: Arc<Vec<TopicMeta>> = Arc::new(topics);
    *state.0.write() = Some(index.clone());
    index
}

/// 验证知识包路径的静态格式：
/// - 必须是相对路径（含 .. 或绝对路径前缀则拒绝）
/// - 只允许 .md 扩展名
pub(crate) fn is_manual_path_syntax_safe(rel: &str) -> bool {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.contains("..")
        || rel.len() > 2 && rel.as_bytes()[1] == b':'
    {
        return false;
    }
    if !rel.to_lowercase().ends_with(".md") {
        return false;
    }
    true
}

pub(crate) fn is_safe_manual_path(root: &std::path::Path, rel: &str) -> bool {
    if !is_manual_path_syntax_safe(rel) {
        return false;
    }
    let full = root.join(rel);
    match (full.canonicalize(), root.canonicalize()) {
        (Ok(abs_file), Ok(abs_root)) => abs_file.starts_with(&abs_root),
        _ => false,
    }
}

/// topic_id -> file path 映射（仅 fulltext 模式使用）。
/// 优先从用户选定的手册目录（知识包）加载 topic_paths.jsonl，
/// 这样发布版用户选择完整 Markdown 目录后即可启用全文。
/// 开发环境回退到 tools/kb_index/topic_paths.jsonl。
pub(crate) fn load_topic_path_map(
    paths: &AppPaths,
    manuals_dir: &str,
) -> std::collections::HashMap<String, String> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if !manuals_dir.is_empty() {
        candidates.push(std::path::Path::new(manuals_dir).join("topic_paths.jsonl"));
    }
    candidates.push(
        paths
            .resource_data_dir
            .join("kb_index")
            .join("topic_paths.jsonl"),
    );
    candidates.push(std::path::PathBuf::from("tools/kb_index/topic_paths.jsonl"));
    for p in &candidates {
        if let Ok(content) = std::fs::read_to_string(p) {
            let map: std::collections::HashMap<String, String> = content
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter_map(|v| {
                    let id = v["topic_id"].as_str()?.to_string();
                    let path = v["path"].as_str()?.to_string();
                    Some((id, path))
                })
                .collect();
            if !map.is_empty() {
                return map;
            }
        }
    }
    std::collections::HashMap::new()
}

/// 检索词分两档：用户真正输入的词，和长中文串切出的字符 bigram。
///
/// bigram 是补召回用的弱信号——"配置""设置""连接"这类高频组合会命中大量无关主题，
/// 若与 ASCII 整词同权，一个长中文问句就能靠几个 bigram 压过精确命令名命中。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct SearchTerms {
    pub(crate) words: Vec<String>,
    pub(crate) bigrams: Vec<String>,
}

pub(crate) fn manual_search_terms(query: &str) -> SearchTerms {
    let query = query.to_lowercase();
    let mut terms = SearchTerms::default();
    let mut seen = std::collections::HashSet::new();
    let ascii = regex::Regex::new(r"[a-z0-9+]{2,}").unwrap();
    let han = regex::Regex::new(r"[\p{Han}]{2,}").unwrap();
    for found in ascii.find_iter(&query) {
        let term = found.as_str().to_string();
        if seen.insert(term.clone()) {
            terms.words.push(term);
        }
    }
    for found in han.find_iter(&query) {
        let segment: Vec<char> = found.as_str().chars().collect();
        if segment.len() <= 2 {
            let term: String = segment.iter().collect();
            if seen.insert(term.clone()) {
                terms.words.push(term);
            }
            continue;
        }
        for pair in segment.windows(2) {
            let term: String = pair.iter().collect();
            if seen.insert(term.clone()) {
                terms.bigrams.push(term);
            }
        }
    }
    terms
}

pub(crate) fn clean_manual_body(content: &str) -> String {
    let body = content.splitn(3, "---").nth(2).unwrap_or(content);
    body.lines()
        .filter(|line| {
            !["- 手册：", "- 仓库：", "- 类型：", "- 来源：", "- 主命令："]
                .iter()
                .any(|prefix| line.trim_start().starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// 宽泛的协议使用/流程问题应优先召回能组成闭环的主题，而不是由索引顺序
/// 决定若干同分的单指令卡片。具体 AT 指令查询不应用这组扩展权重。
pub(crate) fn manual_workflow_bonus(query: &str, title: &str) -> i32 {
    let query = query.to_lowercase();
    let title = title.to_lowercase();
    let workflow_intent = [
        "完整", "流程", "使用", "用法", "怎么", "如何", "指令", "命令", "汇总", "总览", "哪些",
    ]
    .iter()
    .any(|term| query.contains(term));
    if !workflow_intent {
        return 0;
    }
    let protocol = [
        "https", "http", "mqtt", "ftp", "fota", "tcp", "udp", "ssl", "gnss", "ntp",
    ]
    .into_iter()
    .find(|protocol| query.contains(protocol));
    let Some(protocol) = protocol else {
        return 0;
    };
    let title_protocol = if protocol == "https" {
        "http"
    } else {
        protocol
    };
    if !title.contains(title_protocol) {
        return 0;
    }

    let lifecycle = if ["示例", "example"].iter().any(|term| title.contains(term)) {
        30
    } else if ["连接", "建立", "创建", "conn", "connect", "create", "open"]
        .iter()
        .any(|term| title.contains(term))
    {
        24
    } else if ["配置", "cfg", "config"]
        .iter()
        .any(|term| title.contains(term))
    {
        18
    } else if ["at命令", "at指令", "at command"]
        .iter()
        .any(|term| title.contains(term))
    {
        14
    } else if ["断开", "销毁", "disc", "close", "destroy"]
        .iter()
        .any(|term| title.contains(term))
    {
        10
    } else {
        0
    };
    let language_match = if query
        .chars()
        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
        && title
            .chars()
            .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
    {
        20
    } else {
        0
    };
    lifecycle + language_match
}

/// 协议问题优先进入对应的专用手册，避免被其他手册中顺带出现的协议词
/// （例如固件升级里的 HTTP 下载）凭型号适用性加分挤到前面。
pub(crate) fn manual_domain_bonus(query: &str, title: &str, manual: &str) -> i32 {
    let query = query.to_lowercase();
    let title = title.to_lowercase();
    let manual = manual.to_lowercase();
    let protocol = [
        "https", "http", "mqtt", "ftp", "fota", "tcp", "udp", "ssl", "gnss", "ntp",
    ]
    .into_iter()
    .find(|protocol| query.contains(protocol));
    let Some(protocol) = protocol else {
        return 0;
    };
    let title_protocol = if protocol == "https" {
        "http"
    } else {
        protocol
    };
    if manual.contains(protocol) && title.contains(title_protocol) {
        40
    } else {
        0
    }
}

pub(crate) fn model_restriction_notes(
    content: &str,
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
) -> Vec<String> {
    let Some(profile) = profile else {
        return Vec::new();
    };
    let mut notes = Vec::new();
    for line in content.lines() {
        let normalized = line.trim().trim_start_matches('>').trim();
        let lower = normalized.to_lowercase();
        let is_restriction = [
            "不支持",
            "只支持",
            "仅支持",
            "not supported",
            "does not support",
            "only support",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
        if !is_restriction {
            continue;
        }
        let upper = normalized.to_ascii_uppercase();
        let applies = profile.model_codes.iter().any(|model| {
            let model = model.to_ascii_uppercase();
            upper.contains(&model)
                || model
                    .split_once('-')
                    .is_some_and(|(base, _)| upper.contains(base))
        });
        if applies && !notes.iter().any(|note| note == normalized) {
            notes.push(normalized.to_string());
        }
    }
    notes
}

/// 本地手册全文检索（AI 助手 search_manuals 工具的实现）。
/// 索引只存标题/预览，命中后按需读盘取正文——1.2 万主题内存占用可控。
#[tauri::command]
pub fn search_manuals(
    paths: State<'_, AppPaths>,
    state: State<'_, ManualIndex>,
    pack_cache: State<'_, PackValidationCache>,
    knowledge: State<'_, Knowledge>,
    query: String,
    top_k: Option<usize>,
    profile_id: Option<String>,
) -> Vec<serde_json::Value> {
    if query.trim().is_empty() {
        return vec![serde_json::json!({"error": "检索关键词不能为空"})];
    }
    let settings = load_settings(&paths);
    let root = settings["kb"]["manuals_dir"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let index = manual_index(&paths, &state);
    let topic_paths = load_topic_path_map(&paths, &root);
    if index.is_empty() {
        return vec![
            serde_json::json!({"error": "手册索引未生成（运行 tools/kb_convert.py manuals）"}),
        ];
    }
    let q = query.to_lowercase();
    let command_inventory_intent = ["指令", "命令", "清单", "一览", "汇总", "哪些", "所有"]
        .iter()
        .filter(|term| q.contains(*term))
        .count()
        >= 2;
    let terms = manual_search_terms(&query);
    let required_ascii_terms: Vec<&String> = terms
        .words
        .iter()
        .filter(|term| {
            term.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '+')
        })
        .collect();
    let knowledge = knowledge.0.read();
    let profile = profile_id.as_deref().and_then(|id| knowledge.profile(id));

    let mut scored: Vec<(
        i32,
        &TopicMeta,
        DocumentApplicability,
        Option<&TopicOwnerMeta>,
    )> = index
        .iter()
        .filter_map(|t| {
            if !topic_variant_compatible(
                t,
                topic_paths.get(&t.topic_id).map(String::as_str),
                profile,
            ) {
                return None;
            }
            if command_inventory_intent
                && !t.kind.eq_ignore_ascii_case("at command")
                && !t.title.to_lowercase().contains("at命令")
                && !t.title.to_lowercase().contains("at command")
            {
                return None;
            }
            let (applicability, owner) = topic_applicability(t, profile);
            if applicability == DocumentApplicability::Incompatible {
                return None;
            }
            let title = t.title.to_lowercase();
            let manual = owner
                .map(|owner| owner.manual.as_str())
                .unwrap_or(&t.manual)
                .to_lowercase();
            let preview = t.preview.to_lowercase();
            if required_ascii_terms.len() >= 2
                && !required_ascii_terms.iter().all(|term| {
                    title.contains(term.as_str())
                        || manual.contains(term.as_str())
                        || preview.contains(term.as_str())
                })
            {
                return None;
            }
            let mut s = 0;
            // 整句命中（中文查询通常无空格）
            if title.contains(&q) {
                s += 12;
            }
            if manual.contains(&q) {
                s += 5;
            }
            if preview.contains(&q) {
                s += 3;
            }
            for term in &terms.words {
                if title.contains(term) {
                    s += 8;
                }
                if manual.contains(term) {
                    s += 3;
                }
                if preview.contains(term) {
                    s += 2;
                }
            }
            for bigram in &terms.bigrams {
                if title.contains(bigram) {
                    s += 2;
                }
                if manual.contains(bigram) || preview.contains(bigram) {
                    s += 1;
                }
            }
            s += manual_workflow_bonus(&q, &title);
            s += manual_domain_bonus(&q, &title, &manual);
            if s == 0 {
                None
            } else {
                Some((s + applicability.score_bonus(), t, applicability, owner))
            }
        })
        .collect();
    scored.sort_by_key(|(s, _, _, _)| -*s);

    // 预先获取验证结果，仅当完整验证通过时才读取手册正文
    let pack_validation = if root.is_empty() {
        None
    } else {
        validate_pack_cached(&paths, &state, &pack_cache, &root)
    };
    let pack_valid = pack_validation.as_ref().map(|v| v.valid).unwrap_or(false);
    let pack_allows_fulltext = pack_valid
        && pack_validation
            .as_ref()
            .and_then(|validation| validation.manifest.as_ref())
            .map(|manifest| pack_supports_profile(manifest, profile))
            .unwrap_or(false);
    let mut read_failures = 0u32;
    let mut seen_titles = std::collections::HashSet::new();
    let results: Vec<serde_json::Value> = scored
        .into_iter()
        .filter(|(_, topic, _, _)| seen_titles.insert(topic.title.to_lowercase()))
        .take(top_k.unwrap_or(3).min(6))
        .map(|(score, t, applicability, owner)| {
            let content = if root.is_empty() || !pack_allows_fulltext {
                String::new()
            } else {
                let file_path = topic_paths.get(&t.topic_id);
                match file_path {
                    Some(rel) => {
                        if !is_safe_manual_path(std::path::Path::new(&root), rel) {
                            read_failures += 1;
                            String::new()
                        } else {
                            match std::fs::read_to_string(std::path::Path::new(&root).join(rel)) {
                                Ok(c) => {
                                    let body = clean_manual_body(&c);
                                    body.chars().take(4000).collect::<String>()
                                }
                                Err(_) => {
                                    read_failures += 1;
                                    String::new()
                                }
                            }
                        }
                    }
                    None => {
                        read_failures += 1;
                        String::new()
                    }
                }
            };
            let (pack_id, pack_ver) = if content.is_empty() {
                ("cmiot.core.builtin".to_string(), "2026.07".to_string())
            } else {
                let m = pack_validation.as_ref().and_then(|v| v.manifest.as_ref());
                (
                    m.map(|m| m.id.clone())
                        .unwrap_or_else(|| "cmiot.core.builtin".into()),
                    m.map(|m| m.version.clone())
                        .unwrap_or_else(|| "2026.07".into()),
                )
            };
            let restrictions = model_restriction_notes(&content, profile);
            serde_json::json!({
                "score": score,
                "title": t.title,
                "manual": owner.map(|owner| owner.manual.as_str()).unwrap_or(&t.manual),
                "kind": t.kind,
                "models": owner.map(|owner| owner.models.as_slice()).unwrap_or(&t.models),
                "applicability": applicability.as_str(),
                "content": if content.is_empty() { t.preview.clone() } else { content.clone() },
                "modelRestrictions": restrictions,
                "contentLevel": if content.is_empty() { "preview" } else { "fulltext" },
                "packId": pack_id,
                "packVersion": pack_ver,
            })
        })
        .collect();
    // 正文读失败时清除缓存，下次重新验证并降级
    if read_failures > 0 {
        *pack_cache.0.write() = None;
        let _ = std::fs::remove_file(validation_receipt_path(&paths));
    }
    results
}
