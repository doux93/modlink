use serde_json::Value;
use std::path::Path;

/// 默认设置：未配置时回退到这里。
/// AI 配置（base_url/model/recent_log_lines）和 KB 配置（manuals_dir）。
pub(crate) fn default_settings() -> Value {
    serde_json::json!({
        "ai": {
            "base_url": "https://onerouter.cmaiot.cn/v1",
            "model": "Auto",
            "recent_log_lines": 80
        },
        "kb": {
            // 手册语料根目录（内部机器指向 anythingllm_kb/manuals；客户版留空即禁用）
            "manuals_dir": ""
        }
    })
}

/// 深度合并：patch 覆盖 base 的同名字段，未冲突字段保留。
pub(crate) fn merge_json(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                match base_map.get_mut(&k) {
                    Some(existing) => merge_json(existing, v),
                    None => {
                        base_map.insert(k, v);
                    }
                }
            }
        }
        (base_slot, patch_value) => *base_slot = patch_value,
    }
}

/// Only a missing file means first use. Invalid or unreadable files must not be overwritten.
pub(crate) fn load_settings(path: &Path) -> Result<Value, String> {
    let mut settings = default_settings();
    match std::fs::read(path) {
        Ok(content) => {
            let local = serde_json::from_slice(&content)
                .map_err(|e| format!("配置文件损坏，原文件已保留: {e}"))?;
            validate_settings(&local)?;
            merge_json(&mut settings, local);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("配置读取失败，未使用默认值覆盖: {error}")),
    }
    Ok(settings)
}

pub(crate) fn validate_settings(settings: &Value) -> Result<(), String> {
    if !settings.is_object() {
        return Err("配置必须是 JSON 对象，原文件已保留".into());
    }
    for section in ["ai", "kb"] {
        if let Some(value) = settings.get(section) {
            if !value.is_object() {
                return Err(format!("配置 {section} 必须是对象"));
            }
        }
    }
    for (section, field) in [("ai", "base_url"), ("ai", "model"), ("kb", "manuals_dir")] {
        if let Some(value) = settings.get(section).and_then(|s| s.get(field)) {
            if !value.is_string() {
                return Err(format!("配置 {section}.{field} 必须是文本"));
            }
        }
    }
    if let Some(value) = settings.get("ai").and_then(|s| s.get("recent_log_lines")) {
        if !value.as_u64().is_some_and(|n| (20..=300).contains(&n)) {
            return Err("日志行数应为 20～300 的整数".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_shapes_before_indexing_or_writing() {
        for value in [
            serde_json::json!([]),
            serde_json::json!({"ai": false}),
            serde_json::json!({"ai":{"model":3}}),
            serde_json::json!({"ai":{"recent_log_lines":0}}),
        ] {
            assert!(validate_settings(&value).is_err());
        }
    }

    #[test]
    fn missing_defaults_but_corrupt_or_unreadable_files_fail() {
        let path = std::env::temp_dir().join(format!(
            "modulink-config-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert_eq!(load_settings(&path).unwrap(), default_settings());
        std::fs::write(&path, "{broken").unwrap();
        assert!(load_settings(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        std::fs::remove_file(&path).unwrap();
        assert!(load_settings(&std::env::temp_dir()).is_err());
    }

    #[test]
    fn merge_json_preserves_sibling_sections() {
        let mut base = serde_json::json!({
            "ai": { "model": "Auto", "recent_log_lines": 80 },
            "kb": { "manuals_dir": "E:/manuals" }
        });
        merge_json(
            &mut base,
            serde_json::json!({
                "ai": { "recent_log_lines": 120 }
            }),
        );
        assert_eq!(base["ai"]["model"], "Auto");
        assert_eq!(base["ai"]["recent_log_lines"], 120);
        assert_eq!(base["kb"]["manuals_dir"], "E:/manuals");
    }
}
