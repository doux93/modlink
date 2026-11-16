use crate::app_paths::AppPaths;
use serde_json::Value;

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

/// 读取设置文件并与默认值合并。读取失败时回退到默认值。
pub(crate) fn load_settings(paths: &AppPaths) -> Value {
    let mut settings = default_settings();
    if let Ok(content) = std::fs::read_to_string(paths.settings_file()) {
        if let Ok(local) = serde_json::from_str(&content) {
            merge_json(&mut settings, local);
        }
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

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
