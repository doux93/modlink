use crate::app_paths::AppPaths;
use crate::config::load_settings;
use crate::knowledge::catalog::{
    at_command_name, catalog_variant_compatible, command_catalog_restriction, command_pattern,
    load_command_catalog, CommandCatalogEntry,
};
use crate::knowledge::manual::{
    is_safe_manual_path, load_topic_path_map, manual_index, pack_supports_profile,
    topic_applicability, topic_pack_scope, topic_variant_compatible, ManualIndex,
    PackValidationCache,
};
use crate::knowledge::pack::validate_pack_cached;
use crate::types::{AtCommandAuthorization, Knowledge};
use modulink_core::diag::{DiagFlow, EvalRule};
use modulink_core::knowledge::{document_applicability, DocumentApplicability, KnowledgeStore};
use std::path::Path;
use tauri::State;

pub(crate) struct KnowledgeSnapshot {
    pub(crate) profile: Option<modulink_core::knowledge::ModuleProfile>,
    /// 当前画像已确认的内置 AT 指令名（画像自带 + at_docs 中 exact 适用的）
    pub(crate) builtin_exact: std::collections::HashSet<String>,
    /// 3GPP 标准指令名。它们由 27.007/27.005 规范定义，不是厂商私有扩展，
    /// 蜂窝模组默认实现；厂商手册通常只在“不支持”时才特别注明。
    /// 因此缺少型号专属手册页不代表该指令不可用——由 generic 画像统一提供基线。
    pub(crate) standard_3gpp: std::collections::HashSet<String>,
}

impl KnowledgeSnapshot {
    pub(crate) fn capture(knowledge: &Knowledge, profile_id: Option<&str>) -> Self {
        let store = knowledge.0.read();
        let profile = profile_id.and_then(|id| store.profile(id)).cloned();
        let builtin_exact = profile
            .as_ref()
            .map(|p| {
                p.at_commands
                    .iter()
                    .filter_map(|doc| at_command_name(&doc.cmd))
                    .chain(
                        applicable_at_doc_commands(&store, p)
                            .into_iter()
                            .map(|(name, _)| name),
                    )
                    .collect()
            })
            .unwrap_or_default();
        // 只有蜂窝画像继承 3GPP 基线；未选画像时不给，避免无型号上下文放行执行。
        let standard_3gpp = profile
            .as_ref()
            .filter(|p| {
                p.capabilities
                    .iter()
                    .any(|capability| capability.eq_ignore_ascii_case("cellular"))
            })
            .and_then(|_| store.profile("generic"))
            .map(|generic| {
                generic
                    .at_commands
                    .iter()
                    .filter_map(|doc| at_command_name(&doc.cmd))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            profile,
            builtin_exact,
            standard_3gpp,
        }
    }
}

/// 后端确定性查询：执行授权不依赖问答检索的 top-k 结果。
///
/// 目录查表、知识包验证和正文回退都要读盘，放在阻塞线程池里执行，
/// 避免占住 async runtime（串口收发共用）或 Tauri 主线程。
#[tauri::command]
pub async fn authorize_at_commands(
    paths: State<'_, AppPaths>,
    state: State<'_, ManualIndex>,
    pack_cache: State<'_, PackValidationCache>,
    knowledge: State<'_, Knowledge>,
    commands: Vec<String>,
    profile_id: Option<String>,
    port: Option<String>,
) -> Result<Vec<AtCommandAuthorization>, String> {
    let snapshot = KnowledgeSnapshot::capture(&knowledge, profile_id.as_deref());
    let (paths, state, pack_cache) = (
        paths.inner().clone(),
        state.inner().clone(),
        pack_cache.inner().clone(),
    );
    tauri::async_runtime::spawn_blocking(move || {
        authorize_at_commands_blocking(&paths, &state, &pack_cache, &snapshot, commands, port)
    })
    .await
    .map_err(|error| format!("指令授权失败: {error}"))
}

pub(crate) fn authorize_at_commands_blocking(
    paths: &AppPaths,
    state: &ManualIndex,
    pack_cache: &PackValidationCache,
    snapshot: &KnowledgeSnapshot,
    commands: Vec<String>,
    port: Option<String>,
) -> Vec<AtCommandAuthorization> {
    let settings = match load_settings(&paths.settings_file()) {
        Ok(settings) => settings,
        Err(error) => {
            return commands
                .into_iter()
                .map(|command| AtCommandAuthorization {
                    command,
                    authorized: false,
                    applicability: None,
                    manual: None,
                    topic_id: None,
                    pack_id: None,
                    pack_version: None,
                    reason_code: "settings_unavailable".into(),
                    reason: format!("配置读取失败，已阻止执行：{error}"),
                })
                .collect()
        }
    };
    let root = settings["kb"]["manuals_dir"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let index = manual_index(paths, state);
    let topic_paths = load_topic_path_map(paths, &root);
    let profile = snapshot.profile.as_ref();
    let validation = (!root.is_empty())
        .then(|| validate_pack_cached(paths, state, pack_cache, &root))
        .flatten();
    let pack_manifest = validation.as_ref().and_then(|v| v.manifest.as_ref());
    let fulltext_allowed = validation.as_ref().map(|v| v.valid).unwrap_or(false)
        && pack_manifest
            .map(|m| pack_supports_profile(m, profile))
            .unwrap_or(false);
    let command_catalog = load_command_catalog(paths);
    // 实测记录只读一次：它是按 (画像, 端口, 固件) 索引的小表，逐条命令重复读盘没有意义。
    let capability_store = profile
        .zip(port.as_deref())
        .map(|_| crate::capability_evidence::try_load(paths));

    commands
        .into_iter()
        .map(|raw| {
            if let Some(Err(error)) = capability_store.as_ref() {
                return AtCommandAuthorization {
                    command: raw,
                    authorized: false,
                    applicability: None,
                    manual: None,
                    topic_id: None,
                    pack_id: None,
                    pack_version: None,
                    reason_code: "runtime_evidence_unavailable".into(),
                    reason: format!("实测能力读取失败，已阻止执行：{error}"),
                };
            }
            let Some(name) = at_command_name(&raw) else {
                return AtCommandAuthorization {
                    command: raw,
                    authorized: false,
                    applicability: None,
                    manual: None,
                    topic_id: None,
                    pack_id: None,
                    pack_version: None,
                    reason_code: "invalid_command".into(),
                    reason: "不是合法的单条 AT 指令".into(),
                };
            };
            if let Some(reason) = capability_store
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .and_then(|store| {
                    crate::capability_evidence::restriction(
                        store,
                        profile.map(|p| p.id.as_str()),
                        port.as_deref(),
                        &raw,
                    )
                })
            {
                return AtCommandAuthorization {
                    command: name,
                    authorized: false,
                    applicability: Some("runtime".into()),
                    manual: Some("当前模组实测能力".into()),
                    topic_id: None,
                    pack_id: Some("modulink.capability-evidence".into()),
                    pack_version: Some("1".into()),
                    reason_code: "runtime_capability_restricted".into(),
                    reason,
                };
            }
            // The bundled structured catalog is also a capability deny-list.  Its
            // restrictions must remain active when no external full-text pack is open.
            let exact_catalog_entries: Vec<&CommandCatalogEntry> = command_catalog
                .iter()
                .filter(|entry| {
                    entry.command.eq_ignore_ascii_case(&name)
                        && !entry.scope_source.eq_ignore_ascii_case("unknown")
                        && catalog_variant_compatible(entry, profile)
                        && document_applicability(profile, &entry.models)
                            == DocumentApplicability::Exact
                })
                .collect();
            if let Some(reason) = command_catalog_restriction(&exact_catalog_entries, &raw, profile)
            {
                return AtCommandAuthorization {
                    command: name,
                    authorized: false,
                    applicability: Some("exact".into()),
                    manual: exact_catalog_entries
                        .first()
                        .map(|entry| entry.manual.clone()),
                    topic_id: exact_catalog_entries
                        .first()
                        .map(|entry| entry.topic_id.clone()),
                    pack_id: pack_manifest.map(|manifest| manifest.id.clone()),
                    pack_version: pack_manifest.map(|manifest| manifest.version.clone()),
                    reason_code: "model_capability_restricted".into(),
                    reason,
                };
            }
            if snapshot.builtin_exact.contains(&name) {
                return AtCommandAuthorization {
                    command: name,
                    authorized: true,
                    applicability: Some("exact".into()),
                    manual: Some("内置型号知识".into()),
                    topic_id: None,
                    pack_id: Some("cmiot.core.builtin".into()),
                    pack_version: Some("2026.07".into()),
                    reason_code: "authorized".into(),
                    reason: "当前型号内置 AT 指令资料已确认".into(),
                };
            }
            // 3GPP 基线放在实测能力和手册限制之后：任何“该型号不支持”的明确证据都已先行否决，
            // 到这里说明手册对它没有异议，按规范默认可用。
            if snapshot.standard_3gpp.contains(&name) {
                return AtCommandAuthorization {
                    command: name,
                    authorized: true,
                    applicability: Some("standard".into()),
                    manual: Some("3GPP 标准指令集".into()),
                    topic_id: None,
                    pack_id: Some("cmiot.core.builtin".into()),
                    pack_version: Some("2026.07".into()),
                    reason_code: "authorized_standard".into(),
                    reason: "3GPP 标准指令，手册未注明当前型号不支持".into(),
                };
            }
            if fulltext_allowed {
                if let Some(evidence) = exact_catalog_entries
                    .iter()
                    .copied()
                    .find(|entry| topic_paths.contains_key(&entry.topic_id))
                {
                    let manifest = pack_manifest.expect("fulltext package has manifest");
                    return AtCommandAuthorization {
                        command: name,
                        authorized: true,
                        applicability: Some("exact".into()),
                        manual: Some(evidence.manual.clone()),
                        topic_id: Some(evidence.topic_id.clone()),
                        pack_id: Some(manifest.id.clone()),
                        pack_version: Some(manifest.version.clone()),
                        reason_code: "authorized".into(),
                        reason: "当前型号命令目录存在精确 DITAMAP 依据".into(),
                    };
                }
                // Legacy packs without a structured catalog retain the previous full-text fallback.
                if !command_catalog.is_empty() {
                    return AtCommandAuthorization {
                        command: name,
                        authorized: false,
                        applicability: None,
                        manual: None,
                        topic_id: None,
                        pack_id: pack_manifest.map(|m| m.id.clone()),
                        pack_version: pack_manifest.map(|m| m.version.clone()),
                        reason_code: "command_not_exact".into(),
                        reason: "命令目录中没有当前型号的精确依据（unknown/series 资料仅供参考）"
                            .into(),
                    };
                }
                let pattern = command_pattern(&name);
                for topic in index.iter() {
                    // 授权只检查明确索引到该命令的主题，避免逐条读取数千个正文文件。
                    if !pattern.is_match(&topic.title) && !pattern.is_match(&topic.preview) {
                        continue;
                    }
                    if !topic_variant_compatible(
                        topic,
                        topic_paths.get(&topic.topic_id).map(String::as_str),
                        profile,
                    ) {
                        continue;
                    }
                    let (applicability, _) = topic_applicability(topic, profile);
                    if applicability != DocumentApplicability::Exact {
                        continue;
                    }
                    let Some(manifest) = pack_manifest else {
                        continue;
                    };
                    if topic_pack_scope(topic, manifest).is_none() {
                        continue;
                    }
                    let Some(rel) = topic_paths.get(&topic.topic_id) else {
                        continue;
                    };
                    if !is_safe_manual_path(Path::new(&root), rel) {
                        continue;
                    }
                    let Ok(body) = std::fs::read_to_string(Path::new(&root).join(rel)) else {
                        continue;
                    };
                    if pattern.is_match(&body) {
                        let m = pack_manifest.unwrap();
                        return AtCommandAuthorization {
                            command: name,
                            authorized: true,
                            applicability: Some(applicability.as_str().into()),
                            manual: Some(topic.manual.clone()),
                            topic_id: Some(topic.topic_id.clone()),
                            pack_id: Some(m.id.clone()),
                            pack_version: Some(m.version.clone()),
                            reason_code: "authorized".into(),
                            reason: "当前型号知识包全文明确包含该指令".into(),
                        };
                    }
                }
            }
            AtCommandAuthorization {
                command: name,
                authorized: false,
                applicability: None,
                manual: None,
                topic_id: None,
                pack_id: None,
                pack_version: None,
                reason_code: if !fulltext_allowed {
                    "no_verified_fulltext".into()
                } else {
                    "command_not_found".into()
                },
                reason: if !fulltext_allowed {
                    "当前型号没有可用于执行授权的已验证全文依据".into()
                } else {
                    "当前型号知识包中未找到该指令".into()
                },
            }
        })
        .collect()
}

pub(crate) fn flow_at_commands(flow: &DiagFlow) -> Vec<String> {
    flow.steps
        .iter()
        .flat_map(|step| {
            step.send.iter().cloned().chain(
                step.evaluate
                    .iter()
                    .flat_map(|rule| rule.collect.iter().cloned()),
            )
        })
        .collect()
}

/// 用户确认未复核流程后，只能放宽“缺少精确手册证据”这一类拒绝。
/// 未知的新拒绝码默认仍视为硬拒绝，避免后续新增安全规则时被意外绕过。
pub(crate) fn flow_command_allowed(
    decision: &AtCommandAuthorization,
    allow_evidence_gap: bool,
) -> bool {
    decision.authorized
        || (allow_evidence_gap
            && matches!(
                decision.reason_code.as_str(),
                "no_verified_fulltext" | "command_not_exact"
            ))
}

pub(crate) fn prepare_authorized_flow(
    mut flow: DiagFlow,
    decisions: Vec<AtCommandAuthorization>,
    allow_unverified: bool,
) -> Result<DiagFlow, String> {
    if !flow.verified && !allow_unverified {
        return Err(format!(
            "流程「{}」尚未经过实机复核，需在界面明确确认后才能执行",
            flow.name
        ));
    }

    let expected = flow_at_commands(&flow).len();
    if decisions.len() != expected {
        return Err(format!(
            "流程执行授权结果不完整：预期 {expected} 条，实际 {} 条",
            decisions.len()
        ));
    }

    let allow_evidence_gap = !flow.verified && allow_unverified;
    let mut decisions = decisions.into_iter();
    let mut required_denied = Vec::new();

    for step in &mut flow.steps {
        let original_send = std::mem::take(&mut step.send);
        let had_send = !original_send.is_empty();
        for raw in original_send {
            let decision = decisions
                .next()
                .ok_or_else(|| "流程执行授权结果提前结束".to_string())?;
            if flow_command_allowed(&decision, allow_evidence_gap) {
                step.send.push(raw);
            } else if !step.optional {
                required_denied.push(format!(
                    "{}（{}：{}）",
                    raw, decision.reason_code, decision.reason
                ));
            }
        }

        for rule in &mut step.evaluate {
            let original_collect = std::mem::take(&mut rule.collect);
            for raw in original_collect {
                let decision = decisions
                    .next()
                    .ok_or_else(|| "流程执行授权结果提前结束".to_string())?;
                if flow_command_allowed(&decision, allow_evidence_gap) {
                    rule.collect.push(raw);
                }
            }
        }

        // 保留原步骤 id 和位置，使前端计划步骤、实时事件与最终报告始终一一对应。
        if step.optional && had_send && step.send.is_empty() {
            step.parse = None;
            step.retry = None;
            step.expect_urc = None;
            step.evaluate = vec![EvalRule {
                when: None,
                default: true,
                result: "skip".into(),
                detail: None,
                reason: Some("可选步骤的指令未获得当前型号执行授权，已跳过".into()),
                suggestion: None,
                collect: Vec::new(),
                abort: false,
            }];
        }
    }

    if !required_denied.is_empty() {
        return Err(format!(
            "流程「{}」包含未授权的必需指令：{}",
            flow.name,
            required_denied.join("；")
        ));
    }
    Ok(flow)
}

pub(crate) fn require_strict_command_authorization(
    label: &str,
    commands: &[String],
    decisions: &[AtCommandAuthorization],
) -> Result<(), String> {
    if commands.len() != decisions.len() {
        return Err(format!(
            "{label}授权结果不完整：预期 {} 条，实际 {} 条",
            commands.len(),
            decisions.len()
        ));
    }
    let denied = commands
        .iter()
        .zip(decisions)
        .filter(|(_, decision)| !decision.authorized)
        .map(|(raw, decision)| format!("{}（{}：{}）", raw, decision.reason_code, decision.reason))
        .collect::<Vec<_>>();
    if denied.is_empty() {
        Ok(())
    } else {
        Err(format!("{label}包含未授权指令：{}", denied.join("；")))
    }
}

pub(crate) async fn authorize_flow_execution(
    paths: AppPaths,
    state: ManualIndex,
    pack_cache: PackValidationCache,
    snapshot: KnowledgeSnapshot,
    flow: DiagFlow,
    port: String,
    allow_unverified: bool,
    strict_commands: Vec<String>,
) -> Result<DiagFlow, String> {
    if !flow.verified && !allow_unverified {
        return Err(format!(
            "流程「{}」尚未经过实机复核，需在界面明确确认后才能执行",
            flow.name
        ));
    }
    let mut commands = flow_at_commands(&flow);
    let flow_command_count = commands.len();
    commands.extend(strict_commands.iter().cloned());
    let mut decisions = tauri::async_runtime::spawn_blocking(move || {
        authorize_at_commands_blocking(&paths, &state, &pack_cache, &snapshot, commands, Some(port))
    })
    .await
    .map_err(|error| format!("流程执行授权失败：授权检查未能完成（{error}）"))?;
    if decisions.len() < flow_command_count {
        return Err(format!(
            "流程执行授权结果不完整：预期至少 {flow_command_count} 条，实际 {} 条",
            decisions.len()
        ));
    }
    let strict_decisions = decisions.split_off(flow_command_count);
    let flow = prepare_authorized_flow(flow, decisions, allow_unverified)?;
    require_strict_command_authorization("场景恢复指令", &strict_commands, &strict_decisions)?;
    Ok(flow)
}

pub(crate) fn applicable_at_doc_commands(
    store: &KnowledgeStore,
    profile: &modulink_core::knowledge::ModuleProfile,
) -> Vec<(String, String)> {
    store
        .at_docs
        .iter()
        .flat_map(|file| {
            file.commands.iter().filter_map(|doc| {
                let models = if doc.models.is_empty() {
                    match &file.applies_to {
                        serde_yaml::Value::Sequence(items) => items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect(),
                        _ => Vec::new(),
                    }
                } else {
                    doc.models.clone()
                };
                (document_applicability(Some(profile), &models) == DocumentApplicability::Exact)
                    .then(|| at_command_name(&doc.cmd).map(|name| (name, doc.source.clone())))
                    .flatten()
            })
        })
        .collect()
}
