//! Removes internal document metadata without consuming the following summary.

pub fn sanitize_preview(preview: &str) -> String {
    static PATTERNS: std::sync::LazyLock<Vec<regex::Regex>> = std::sync::LazyLock::new(|| {
        [
            r"(?:仓库|来源|类型|手册)[：:]\s*(?:`[^`\r\n]*`|[^\s`]+)",
            r"(?:developmentdomain|anythingllm_kb)/[^\s`]*",
            r"`[a-z0-9][a-z0-9_]{3,}`",
            r"`?(?:[\w.-]+[/\\])*[\w.-]+\.(?:ditamap|dita|md)\b`?",
        ]
        .into_iter()
        .map(|pattern| regex::Regex::new(pattern).expect("valid preview sanitizing regex"))
        .collect()
    });
    static WHITESPACE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\s{3,}").unwrap());

    let mut value = preview.to_owned();
    for pattern in PATTERNS.iter() {
        value = pattern.replace_all(&value, "").into_owned();
    }
    value = value
        .replace("anythingllm_kb", "")
        .replace("研发中心产品手册发布项目模板", "")
        .replace("仅供文档工程师使用", "")
        .replace('`', "");
    WHITESPACE.replace_all(&value, " ").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_sanitization_contract_preserves_commands_and_summary() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/preview_sanitization.json"
        ))
        .unwrap();
        for case in fixtures.as_array().unwrap() {
            let cleaned = sanitize_preview(case["input"].as_str().unwrap());
            for expected in case["contains"].as_array().unwrap() {
                assert!(
                    cleaned.contains(expected.as_str().unwrap()),
                    "{case}: {cleaned}"
                );
            }
            for forbidden in case["excludes"].as_array().unwrap() {
                assert!(
                    !cleaned.contains(forbidden.as_str().unwrap()),
                    "{case}: {cleaned}"
                );
            }
        }
    }
}
