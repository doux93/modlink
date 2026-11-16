use crate::app_paths::AppPaths;
use std::path::PathBuf;

pub(crate) fn at_command_name(command: &str) -> Option<String> {
    let trimmed = command.trim().to_ascii_uppercase();
    let bytes = trimmed.as_bytes();
    let end = bytes
        .iter()
        .position(|b| *b == b'=' || *b == b'?' || *b == b' ' || *b == b',')
        .unwrap_or(bytes.len());
    let name = trimmed[..end].to_string();
    static VALID_NAME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^AT(?:[+^&*][A-Z0-9]+|[A-Z][A-Z0-9]*)?$")
            .expect("valid AT name pattern")
    });
    VALID_NAME.is_match(&name).then_some(name)
}

pub(crate) fn command_pattern(name: &str) -> regex::Regex {
    regex::Regex::new(&format!(
        r"(?i)(?:^|[^A-Z0-9+^]){}(?:[=?\s,]|$)",
        regex::escape(name)
    ))
    .expect("valid AT command pattern")
}

#[derive(serde::Deserialize)]
pub(crate) struct CommandCatalogEntry {
    pub(crate) command: String,
    pub(crate) topic_id: String,
    pub(crate) manual: String,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) scope_source: String,
    #[serde(default)]
    pub(crate) variants: Vec<String>,
    #[serde(default)]
    pub(crate) unsupported_models: Vec<String>,
    #[serde(default)]
    pub(crate) parameter_constraints: Vec<CommandParameterConstraint>,
}

#[derive(serde::Deserialize)]
pub(crate) struct CommandParameterConstraint {
    pub(crate) parameter: String,
    pub(crate) selector: String,
    #[serde(default)]
    pub(crate) position: Option<usize>,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) allowed_values: Vec<String>,
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) values: Vec<String>,
    #[serde(default)]
    pub(crate) confidence: String,
    #[serde(default)]
    pub(crate) enforce: Option<bool>,
    pub(crate) note: String,
}

pub(crate) fn load_command_catalog(paths: &AppPaths) -> Vec<CommandCatalogEntry> {
    [
        paths
            .resource_data_dir
            .join("kb_index")
            .join("command_catalog.jsonl"),
        PathBuf::from("data/kb_index/command_catalog.jsonl"),
    ]
    .into_iter()
    .find_map(|path| {
        std::fs::read_to_string(path).ok().map(|content| {
            content
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect::<Vec<_>>()
        })
    })
    .unwrap_or_default()
}

pub(crate) fn catalog_variant_compatible(
    entry: &CommandCatalogEntry,
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
) -> bool {
    let requires_nb = entry.variants.iter().any(|variant| {
        variant.eq_ignore_ascii_case("nb-iot") || variant.eq_ignore_ascii_case("nbiot")
    });
    !requires_nb
        || profile.is_some_and(|profile| {
            profile.family.eq_ignore_ascii_case("nb-iot")
                || profile
                    .capabilities
                    .iter()
                    .any(|capability| capability.eq_ignore_ascii_case("nb-iot"))
        })
}

pub(crate) fn profile_matches_any_model(
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
    models: &[String],
) -> bool {
    profile.is_some_and(|profile| {
        profile.model_codes.iter().any(|profile_model| {
            let profile_model = profile_model.to_ascii_uppercase();
            models.iter().any(|model| {
                let model = model.to_ascii_uppercase();
                profile_model == model || profile_model.starts_with(&(model + "-"))
            })
        })
    })
}

pub(crate) use modulink_core::at_protocol::split_at_arguments;

pub(crate) fn command_catalog_restriction(
    entries: &[&CommandCatalogEntry],
    raw: &str,
    profile: Option<&modulink_core::knowledge::ModuleProfile>,
) -> Option<String> {
    if entries
        .iter()
        .any(|entry| profile_matches_any_model(profile, &entry.unsupported_models))
    {
        return Some("手册明确注明当前型号不支持该命令".into());
    }
    let arguments = split_at_arguments(raw);
    for entry in entries {
        for constraint in &entry.parameter_constraints {
            if !profile_matches_any_model(profile, &constraint.models) {
                continue;
            }
            if constraint.enforce == Some(false)
                || constraint
                    .confidence
                    .eq_ignore_ascii_case("review_required")
            {
                continue;
            }
            let kind = if constraint.kind.is_empty() && !constraint.allowed_values.is_empty() {
                "allowed"
            } else {
                constraint.kind.as_str()
            };
            let values = if constraint.values.is_empty() {
                &constraint.allowed_values
            } else {
                &constraint.values
            };
            if kind == "required" {
                if arguments.is_empty() {
                    return Some(format!(
                        "当前型号必须显式配置参数 {}；{}",
                        constraint.parameter, constraint.note
                    ));
                }
                continue;
            }
            let value = constraint
                .position
                .and_then(|position| arguments.get(position))
                .or_else(|| {
                    arguments
                        .iter()
                        .position(|argument| argument.eq_ignore_ascii_case(&constraint.selector))
                        .and_then(|selector_index| arguments.get(selector_index + 1))
                });
            let Some(value) = value else { continue };
            if kind == "allowed"
                && !values
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(value))
            {
                return Some(format!(
                    "当前型号参数 {} 仅支持 {}；{}",
                    constraint.parameter,
                    values.join("/"),
                    constraint.note
                ));
            }
            if kind == "denied"
                && values
                    .iter()
                    .any(|denied| denied.eq_ignore_ascii_case(value))
            {
                return Some(format!(
                    "当前型号参数 {} 不支持值 {}；{}",
                    constraint.parameter, value, constraint.note
                ));
            }
        }
    }
    None
}
