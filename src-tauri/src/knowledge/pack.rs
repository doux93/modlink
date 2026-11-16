use crate::app_paths::AppPaths;
use crate::config::load_settings;
use crate::knowledge::manual::{
    is_manual_path_syntax_safe, manual_index, normalize_model_code, topic_matches_pack_scope,
    topic_pack_scope, CachedPackValidation, ManualIndex, PackTopicScope, PackValidationCache,
    PackValidationReceipt, TopicMeta, KNOWLEDGE_PACK_V1, KNOWLEDGE_PACK_V2, KNOWLEDGE_PACK_V3,
};
use crate::types::{KnowledgePackManifest, KnowledgePackTopicCounts, KnowledgePackValidation};
use tauri::State;

/// 验证知识包内部逻辑（不写配置）：契约检查 manifest、路径映射、文件覆盖率和安全性。
pub(crate) fn validate_pack_internal(
    paths: &AppPaths,
    manual_state: &ManualIndex,
    pack_dir: &str,
) -> KnowledgePackValidation {
    let mut errors: Vec<String> = Vec::new();
    let dir = std::path::Path::new(pack_dir);
    if pack_dir.is_empty() || !dir.is_dir() {
        errors.push("目录不存在".into());
        return KnowledgePackValidation {
            valid: false,
            manifest: None,
            topic_paths_count: 0,
            sample_readable: 0,
            sample_total: 0,
            errors,
        };
    }

    // 1. manifest.json 契约检查
    let manifest_path = dir.join("manifest.json");
    let manifest = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|c| serde_json::from_str::<KnowledgePackManifest>(&c).ok());
    match &manifest {
        Some(m) => {
            if m.schema != KNOWLEDGE_PACK_V1
                && m.schema != KNOWLEDGE_PACK_V2
                && m.schema != KNOWLEDGE_PACK_V3
            {
                errors.push(format!(
                    "不支持的 schema: {}（支持 {}、{} 或 {}）",
                    m.schema, KNOWLEDGE_PACK_V1, KNOWLEDGE_PACK_V2, KNOWLEDGE_PACK_V3
                ));
            }
            if m.id.trim().is_empty() {
                errors.push("manifest.id 不能为空".into());
            }
            if m.version.trim().is_empty() {
                errors.push("manifest.version 不能为空".into());
            }
            if m.schema == KNOWLEDGE_PACK_V2 || m.schema == KNOWLEDGE_PACK_V3 {
                let normalized: Vec<String> = m
                    .models
                    .iter()
                    .map(|model| normalize_model_code(model))
                    .filter(|model| !model.is_empty())
                    .collect();
                let unique: std::collections::HashSet<&String> = normalized.iter().collect();
                if normalized.is_empty() {
                    errors.push("型号知识包的 manifest.models 不能为空".into());
                }
                if normalized.len() != m.models.len() || unique.len() != normalized.len() {
                    errors.push("manifest.models 含空值或重复型号".into());
                }
            }
        }
        None => errors.push("manifest.json 缺失或格式错误".into()),
    }

    // 2. topic_paths.jsonl：检查格式、重复 ID、路径安全性
    let tp_path = dir.join("topic_paths.jsonl");
    let raw_tp = match std::fs::read_to_string(&tp_path) {
        Ok(content) => content,
        Err(_) => {
            errors.push("topic_paths.jsonl 缺失或不可读".into());
            String::new()
        }
    };
    let mut topic_paths: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut malformed_lines = 0;
    let mut duplicate_ids = 0;
    let mut unsafe_paths = 0;
    let mut missing_files = 0;
    let canonical_root = dir.canonicalize().ok();
    for line in raw_tp.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => {
                let id = v["topic_id"].as_str().unwrap_or("").to_string();
                let path = v["path"].as_str().unwrap_or("").to_string();
                if id.is_empty() || path.is_empty() {
                    malformed_lines += 1;
                    continue;
                }
                if !seen_ids.insert(id.clone()) {
                    duplicate_ids += 1;
                    continue;
                }
                if !is_manual_path_syntax_safe(&path) {
                    unsafe_paths += 1;
                    continue;
                }
                match (dir.join(&path).canonicalize(), canonical_root.as_ref()) {
                    (Ok(absolute), Some(root))
                        if absolute.starts_with(root) && absolute.is_file() =>
                    {
                        topic_paths.insert(id, path);
                    }
                    (Ok(_), _) => unsafe_paths += 1,
                    (Err(_), _) => missing_files += 1,
                }
            }
            Err(_) => malformed_lines += 1,
        }
    }
    if topic_paths.is_empty() {
        errors.push("topic_paths.jsonl 缺失或无有效记录".into());
    }
    if malformed_lines > 0 {
        errors.push(format!("路径映射中有 {} 行格式错误", malformed_lines));
    }
    if duplicate_ids > 0 {
        errors.push(format!("路径映射中有 {} 个重复 topic_id", duplicate_ids));
    }
    if unsafe_paths > 0 {
        errors.push(format!("路径映射中有 {} 条不安全路径", unsafe_paths));
    }
    if missing_files > 0 {
        errors.push(format!("路径映射中有 {} 个文件不存在", missing_files));
    }

    // 3. 覆盖率：@1 必须覆盖全量索引；@2 必须精确覆盖 manifest 声明的型号作用域。
    let index = manual_index(paths, manual_state);
    if let Some(m) = &manifest {
        if m.schema == KNOWLEDGE_PACK_V2 || m.schema == KNOWLEDGE_PACK_V3 {
            let indexed_models: std::collections::HashSet<String> = index
                .iter()
                .flat_map(|topic| topic.models.iter())
                .map(|model| normalize_model_code(model))
                .collect();
            let unknown_models: Vec<String> = m
                .models
                .iter()
                .map(|model| normalize_model_code(model))
                .filter(|model| !model.is_empty() && !indexed_models.contains(model))
                .collect();
            if !unknown_models.is_empty() {
                errors.push(format!(
                    "manifest.models 含索引未收录型号: {}",
                    unknown_models.join(", ")
                ));
            }
        }
    }
    let expected_topics: Vec<&TopicMeta> = match &manifest {
        Some(m) if m.schema == KNOWLEDGE_PACK_V2 || m.schema == KNOWLEDGE_PACK_V3 => index
            .iter()
            .filter(|topic| topic_matches_pack_scope(topic, m))
            .collect(),
        _ => index.iter().collect(),
    };
    if let Some(m) = manifest
        .as_ref()
        .filter(|m| m.schema == KNOWLEDGE_PACK_V2 || m.schema == KNOWLEDGE_PACK_V3)
    {
        let mut actual_counts = KnowledgePackTopicCounts::default();
        for topic in &expected_topics {
            match topic_pack_scope(topic, m) {
                Some(PackTopicScope::Exact) => actual_counts.exact += 1,
                Some(PackTopicScope::Series) => actual_counts.series += 1,
                Some(PackTopicScope::Generic) => actual_counts.generic += 1,
                Some(PackTopicScope::Unknown) => actual_counts.unknown += 1,
                None => {}
            }
        }
        match &m.topic_counts {
            Some(declared) if declared != &actual_counts => errors.push(format!(
                "manifest.topicCounts 与实际作用域不一致（实际 exact={}, series={}, generic={}, unknown={}）",
                actual_counts.exact, actual_counts.series, actual_counts.generic, actual_counts.unknown
            )),
            None => errors.push("型号知识包缺少 manifest.topicCounts".into()),
            _ => {}
        }
    }
    let sample_total = expected_topics.len().min(5);
    let mut missing_mappings = 0;
    let sample_readable = expected_topics
        .iter()
        .take(5)
        .filter(|t| {
            topic_paths
                .get(&t.topic_id)
                .map(|rel| dir.join(rel).is_file())
                .unwrap_or(false)
        })
        .count();
    // 映射不能超出公开索引，也不能超出 @2 声明的型号作用域。
    let index_ids: std::collections::HashSet<String> =
        index.iter().map(|t| t.topic_id.clone()).collect();
    let expected_ids: std::collections::HashSet<String> =
        expected_topics.iter().map(|t| t.topic_id.clone()).collect();
    let mapping_ids: std::collections::HashSet<String> = topic_paths.keys().cloned().collect();
    let unknown_mappings = mapping_ids.difference(&index_ids).count();
    let out_of_scope_mappings = if manifest
        .as_ref()
        .map(|m| m.schema == KNOWLEDGE_PACK_V2 || m.schema == KNOWLEDGE_PACK_V3)
        .unwrap_or(false)
    {
        mapping_ids
            .iter()
            .filter(|id| index_ids.contains(*id) && !expected_ids.contains(*id))
            .count()
    } else {
        0
    };
    for t in &expected_topics {
        if !topic_paths.contains_key(&t.topic_id) {
            missing_mappings += 1;
        }
    }
    // topic_id 的算法随索引编译器演进过（路径哈希 → 规范化正文哈希）。整包对不上时，
    // 逐条报“未知 topic_id / 缺少映射”只会淹没真正的原因，用户看不出该重新导出。
    if !mapping_ids.is_empty() && mapping_ids.is_disjoint(&index_ids) {
        errors.push(format!(
            "知识包的 {} 条 topic_id 与当前内置索引完全不匹配，通常表示该包由旧版索引编译器生成；请用当前版本重新导出知识包",
            mapping_ids.len()
        ));
    } else {
        if unknown_mappings > 0 {
            errors.push(format!(
                "路径映射中有 {} 个未知 topic_id，不属于公开索引",
                unknown_mappings
            ));
        }
        if out_of_scope_mappings > 0 {
            errors.push(format!(
                "路径映射中有 {} 个 topic_id 超出声明的型号范围",
                out_of_scope_mappings
            ));
        }
        if missing_mappings > 0 {
            errors.push(format!(
                "知识包作用域有 {} 个 topic_id 缺少路径映射，未完整覆盖",
                missing_mappings
            ));
        }
    }
    let required_sample = sample_total.min(3);
    if sample_readable < required_sample && missing_mappings == 0 && missing_files == 0 {
        errors.push(format!(
            "采样可读文件 {}/{} 不足，无法确认全文可用",
            sample_readable, sample_total
        ));
    }

    // 4. manifest topicCount 必须 > 0 且与路径映射严格相等
    if let Some(m) = &manifest {
        if m.topic_count == 0 {
            errors.push("manifest.topicCount 必须大于 0".into());
        } else if m.topic_count != topic_paths.len() {
            errors.push(format!(
                "manifest.topicCount ({}) 与路径映射条数 ({}) 不一致",
                m.topic_count,
                topic_paths.len()
            ));
        }
    }

    let valid = errors.is_empty();
    KnowledgePackValidation {
        valid,
        manifest,
        topic_paths_count: topic_paths.len(),
        sample_readable,
        sample_total,
        errors,
    }
}

pub(crate) fn file_stamp(path: &std::path::Path) -> (u64, u64) {
    std::fs::metadata(path)
        .map(|metadata| {
            let modified_ms = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0);
            (modified_ms, metadata.len())
        })
        .unwrap_or((0, 0))
}

pub(crate) fn validation_receipt_path(paths: &AppPaths) -> std::path::PathBuf {
    paths.config_dir.join("knowledge-pack-validation.json")
}

pub(crate) fn load_validation_receipt(
    paths: &AppPaths,
    pack_dir: &str,
    manifest_stamp: (u64, u64),
    topic_paths_stamp: (u64, u64),
) -> Option<KnowledgePackValidation> {
    let content = std::fs::read_to_string(validation_receipt_path(paths)).ok()?;
    let receipt: PackValidationReceipt = serde_json::from_str(&content).ok()?;
    (receipt.validation.valid
        && receipt.pack_dir == pack_dir
        && receipt.manifest_stamp == manifest_stamp
        && receipt.topic_paths_stamp == topic_paths_stamp)
        .then_some(receipt.validation)
}

pub(crate) fn save_validation_receipt(
    paths: &AppPaths,
    pack_dir: &str,
    manifest_stamp: (u64, u64),
    topic_paths_stamp: (u64, u64),
    validation: &KnowledgePackValidation,
) {
    if !validation.valid {
        return;
    }
    let receipt = PackValidationReceipt {
        pack_dir: pack_dir.to_string(),
        manifest_stamp,
        topic_paths_stamp,
        validation: validation.clone(),
    };
    if let Ok(json) = serde_json::to_string(&receipt) {
        let _ = std::fs::write(validation_receipt_path(paths), json);
    }
}

/// 带缓存的验证：进程内缓存命中最快；跨重启时复用已通过且契约文件未变的验证凭据。
pub(crate) fn validate_pack_cached(
    paths: &AppPaths,
    manual_state: &ManualIndex,
    cache: &PackValidationCache,
    pack_dir: &str,
) -> Option<KnowledgePackValidation> {
    if pack_dir.is_empty() {
        return None;
    }
    let dir = std::path::Path::new(pack_dir);
    let manifest_stamp = file_stamp(&dir.join("manifest.json"));
    let topic_paths_stamp = file_stamp(&dir.join("topic_paths.jsonl"));
    {
        let cached = cache.0.read();
        if let Some(cached) = cached.as_ref() {
            if cached.pack_dir == pack_dir
                && cached.manifest_stamp == manifest_stamp
                && cached.topic_paths_stamp == topic_paths_stamp
            {
                return Some(cached.validation.clone());
            }
        }
    }
    if let Some(validation) =
        load_validation_receipt(paths, pack_dir, manifest_stamp, topic_paths_stamp)
    {
        *cache.0.write() = Some(CachedPackValidation {
            pack_dir: pack_dir.to_string(),
            manifest_stamp,
            topic_paths_stamp,
            validation: validation.clone(),
        });
        return Some(validation);
    }
    let val = validate_pack_internal(paths, manual_state, pack_dir);
    if val.valid {
        save_validation_receipt(paths, pack_dir, manifest_stamp, topic_paths_stamp, &val);
        *cache.0.write() = Some(CachedPackValidation {
            pack_dir: pack_dir.to_string(),
            manifest_stamp,
            topic_paths_stamp,
            validation: val.clone(),
        });
    }
    Some(val)
}

#[tauri::command]
pub async fn validate_knowledge_pack(
    paths: State<'_, AppPaths>,
    manual_state: State<'_, ManualIndex>,
    cache: State<'_, PackValidationCache>,
    pack_dir: String,
) -> Result<KnowledgePackValidation, String> {
    let paths = paths.inner().clone();
    let manual_state = manual_state.inner().clone();
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_pack_cached(&paths, &manual_state, &cache, &pack_dir)
            .unwrap_or_else(|| validate_pack_internal(&paths, &manual_state, &pack_dir))
    })
    .await
    .map_err(|e| format!("知识包验证失败: {e}"))
}

/// 原子操作：检查（可复用已通过凭据）并保存配置，避免验证目录 A 后激活目录 B。
#[tauri::command]
pub async fn validate_and_activate_knowledge_pack(
    paths: State<'_, AppPaths>,
    manual_state: State<'_, ManualIndex>,
    cache: State<'_, PackValidationCache>,
    pack_dir: String,
) -> Result<KnowledgePackValidation, String> {
    let paths = paths.inner().clone();
    let manual_state = manual_state.inner().clone();
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = validate_pack_cached(&paths, &manual_state, &cache, &pack_dir)
            .unwrap_or_else(|| validate_pack_internal(&paths, &manual_state, &pack_dir));
        if !result.valid {
            return Ok(result);
        }
        let mut settings = load_settings(&paths);
        settings["kb"]["manuals_dir"] = serde_json::Value::String(pack_dir.clone());
        let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        std::fs::write(paths.settings_file(), json).map_err(|e| format!("写入失败: {e}"))?;
        // 复用本次原子验证结果，避免激活后的能力刷新再次全量扫描。
        let dir = std::path::Path::new(&pack_dir);
        let manifest_stamp = file_stamp(&dir.join("manifest.json"));
        let topic_paths_stamp = file_stamp(&dir.join("topic_paths.jsonl"));
        *cache.0.write() = Some(CachedPackValidation {
            pack_dir,
            manifest_stamp,
            topic_paths_stamp,
            validation: result.clone(),
        });
        Ok(result)
    })
    .await
    .map_err(|e| format!("知识包激活失败: {e}"))?
}
