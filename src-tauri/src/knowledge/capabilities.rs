use crate::app_paths::AppPaths;
use crate::config::load_settings;
use crate::knowledge::manual::{
    determine_effective_manual_mode, manual_index, ManualIndex, PackValidationCache,
    KNOWLEDGE_PACK_V2, KNOWLEDGE_PACK_V3,
};
use crate::knowledge::pack::validate_pack_cached;
use crate::types::{Knowledge, KnowledgeCapabilities, KnowledgePackInfo, ManualContentLevel};
use tauri::State;

#[allow(dead_code)]
/// 根据索引是否为空、手册目录与采样可读性判定手册内容级别。
/// 抽取为纯函数以便单元测试，调用方负责计算 sample_readable。
pub(crate) fn determine_manual_mode(
    index_empty: bool,
    manuals_dir: &str,
    sample_readable: bool,
) -> ManualContentLevel {
    if index_empty {
        ManualContentLevel::None
    } else if manuals_dir.is_empty() || !std::path::Path::new(manuals_dir).is_dir() {
        ManualContentLevel::Preview
    } else if sample_readable {
        ManualContentLevel::FullText
    } else {
        ManualContentLevel::Preview
    }
}

#[tauri::command]
pub async fn get_knowledge_capabilities(
    paths: State<'_, AppPaths>,
    manual_state: State<'_, ManualIndex>,
    pack_cache: State<'_, PackValidationCache>,
    knowledge: State<'_, Knowledge>,
    profile_id: Option<String>,
) -> Result<KnowledgeCapabilities, String> {
    let requested_profile_id = profile_id.clone();
    let (core_available, mut model_coverage, profile) = {
        let k = knowledge.0.read();
        let core_available =
            !k.profiles.is_empty() && !k.flows.is_empty() && !k.error_codes.is_empty();
        let model_coverage: Vec<String> = k
            .profiles
            .iter()
            .flat_map(|p| p.model_codes.iter().cloned())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let profile = profile_id
            .as_deref()
            .and_then(|id| k.profiles.iter().find(|profile| profile.id == id))
            .cloned();
        (core_available, model_coverage, profile)
    };
    model_coverage.sort();

    let paths = paths.inner().clone();
    let manual_state = manual_state.inner().clone();
    let pack_cache = pack_cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        build_knowledge_capabilities(
            &paths,
            &manual_state,
            &pack_cache,
            core_available,
            model_coverage,
            requested_profile_id,
            profile,
        )
    })
    .await
    .map_err(|e| format!("知识能力查询失败: {e}"))?
}

pub(crate) fn build_knowledge_capabilities(
    paths: &AppPaths,
    manual_state: &ManualIndex,
    pack_cache: &PackValidationCache,
    core_available: bool,
    model_coverage: Vec<String>,
    effective_profile_id: Option<String>,
    profile: Option<modulink_core::knowledge::ModuleProfile>,
) -> Result<KnowledgeCapabilities, String> {
    // 判断手册级别：有索引 + 配了 manuals_dir = fulltext；有索引无 dir = preview；无索引 = none
    let index = manual_index(paths, manual_state);
    let settings = load_settings(&paths.settings_file())?;
    let manuals_dir = settings["kb"]["manuals_dir"]
        .as_str()
        .unwrap_or("")
        .to_string();
    // 由完整验证结果决定 manualMode，避免采样通过但契约失败的不一致
    let pack_validation = if !manuals_dir.is_empty() {
        validate_pack_cached(paths, manual_state, pack_cache, &manuals_dir)
    } else {
        None
    };
    let pack_valid = pack_validation.as_ref().map(|v| v.valid).unwrap_or(false);
    let manual_mode = if index.is_empty() {
        ManualContentLevel::None
    } else if !manuals_dir.is_empty() && std::path::Path::new(&manuals_dir).is_dir() && pack_valid {
        ManualContentLevel::FullText
    } else {
        ManualContentLevel::Preview
    };

    // 核心包始终描述内置能力（摘要或全文）
    let mut installed_packs = vec![KnowledgePackInfo {
        id: "cmiot.core.builtin".into(),
        version: "2026.07".into(),
        content_level: if index.is_empty() {
            ManualContentLevel::None
        } else {
            ManualContentLevel::Preview
        },
        models: None,
    }];
    let mut fulltext_model_coverage = None;
    // 仅当外部包完整验证通过时才加入 installedPacks
    if pack_valid {
        if let Some(val) = &pack_validation {
            if let Some(m) = &val.manifest {
                installed_packs.push(KnowledgePackInfo {
                    id: m.id.clone(),
                    version: m.version.clone(),
                    content_level: manual_mode.clone(),
                    models: if m.schema == KNOWLEDGE_PACK_V2 || m.schema == KNOWLEDGE_PACK_V3 {
                        Some(m.models.clone())
                    } else {
                        None
                    },
                });
                if m.schema == KNOWLEDGE_PACK_V2 || m.schema == KNOWLEDGE_PACK_V3 {
                    fulltext_model_coverage = Some(m.models.clone());
                }
            }
        }
    }

    let degradation_reason = match manual_mode {
        ManualContentLevel::FullText => None,
        ManualContentLevel::Preview => {
            if manuals_dir.is_empty() {
                Some("未配置手册目录，仅可用内置摘要".into())
            } else if !std::path::Path::new(&manuals_dir).is_dir() {
                Some("配置的手册目录不存在，降级为摘要模式".into())
            } else if !pack_valid {
                Some(
                    "知识包验证未通过（manifest/路径映射/文件覆盖率不合格），降级为摘要模式".into(),
                )
            } else {
                Some("未知原因，降级为摘要模式".into())
            }
        }
        ManualContentLevel::None => Some("手册索引未生成".into()),
    };

    let (effective_manual_mode, effective_degradation_reason) = determine_effective_manual_mode(
        &manual_mode,
        degradation_reason.as_deref(),
        pack_validation
            .as_ref()
            .and_then(|validation| validation.manifest.as_ref()),
        effective_profile_id.as_deref(),
        profile.as_ref(),
    );

    Ok(KnowledgeCapabilities {
        core_available,
        manual_mode,
        effective_manual_mode,
        effective_profile_id,
        effective_degradation_reason,
        installed_packs,
        model_coverage,
        fulltext_model_coverage,
        topic_count: index.len(),
        degradation_reason,
    })
}
