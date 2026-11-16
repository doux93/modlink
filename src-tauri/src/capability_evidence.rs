use crate::app_paths::AppPaths;
use crate::core::execution::{ExecutionReport, ExecutionRequest};
use modulink_core::at_protocol::split_at_arguments as split_arguments;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityEvidenceStore {
    #[serde(default = "schema")]
    pub schema: String,
    #[serde(default)]
    pub port_firmware: HashMap<String, PortFirmware>,
    #[serde(default)]
    pub observations: Vec<CapabilityObservation>,
}

fn schema() -> String {
    "modulink/capability-evidence@1".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortFirmware {
    pub profile_id: String,
    pub firmware: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityObservation {
    pub profile_id: String,
    pub port: String,
    pub firmware: Option<String>,
    pub command: String,
    pub query: String,
    pub response_lines: Vec<String>,
    pub updated_at: i64,
}

fn command_name(raw: &str) -> Option<String> {
    let upper = raw.trim().to_ascii_uppercase();
    let end = upper.find(['=', '?', ' ', ',']).unwrap_or(upper.len());
    let name = &upper[..end];
    (name == "AT"
        || name.starts_with("AT+")
        || name
            .chars()
            .skip(2)
            .all(|character| character.is_ascii_alphanumeric()))
    .then(|| name.to_string())
}

pub fn try_load(paths: &AppPaths) -> Result<CapabilityEvidenceStore, String> {
    crate::storage::JsonDocument {
        database: &paths.db_file(),
        key: "capability-evidence@1",
        legacy_file: &paths.capability_evidence_file(),
    }
    .load(&CapabilityEvidenceStore {
        schema: schema(),
        ..Default::default()
    })
}

pub fn load(paths: &AppPaths) -> CapabilityEvidenceStore {
    try_load(paths).unwrap_or_else(|error| {
        eprintln!("实测能力读取失败: {error}");
        CapabilityEvidenceStore {
            schema: schema(),
            ..Default::default()
        }
    })
}

#[cfg(test)]
fn save(paths: &AppPaths, store: &CapabilityEvidenceStore) -> Result<(), String> {
    update(paths, |current| {
        *current = store.clone();
        Ok(true)
    })
    .map(|_| ())
}

fn update(
    paths: &AppPaths,
    apply: impl FnOnce(&mut CapabilityEvidenceStore) -> Result<bool, String>,
) -> Result<bool, String> {
    crate::storage::JsonDocument {
        database: &paths.db_file(),
        key: "capability-evidence@1",
        legacy_file: &paths.capability_evidence_file(),
    }
    .update(
        &CapabilityEvidenceStore {
            schema: schema(),
            ..Default::default()
        },
        apply,
    )
}

fn meaningful_lines(request: &str, lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| {
            !line.is_empty()
                && !line.eq_ignore_ascii_case(request.trim())
                && !line.eq_ignore_ascii_case("OK")
        })
        .map(str::to_string)
        .collect()
}

pub fn record_execution(
    paths: &AppPaths,
    request: &ExecutionRequest,
    report: &ExecutionReport,
) -> Result<bool, String> {
    let Some(profile_id) = request
        .profile_id
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Ok(false);
    };
    update(paths, |store| {
        let now = chrono::Utc::now().timestamp_millis();
        let mut changed = false;
        for result in &report.results {
            if result.final_code != "OK" {
                continue;
            }
            let Some(name) = command_name(&result.request) else {
                continue;
            };
            let lines = meaningful_lines(&result.request, &result.lines);
            if name == "AT+CGMR" && !lines.is_empty() {
                store.port_firmware.insert(
                    request.target.port.clone(),
                    PortFirmware {
                        profile_id: profile_id.into(),
                        firmware: lines.join(" | "),
                        updated_at: now,
                    },
                );
                changed = true;
                continue;
            }
            if !result.request.trim().ends_with("=?") || lines.is_empty() {
                continue;
            }
            let firmware = store
                .port_firmware
                .get(&request.target.port)
                .filter(|identity| identity.profile_id == profile_id)
                .map(|identity| identity.firmware.clone());
            store.observations.retain(|observation| {
                !(observation.profile_id == profile_id
                    && observation.port.eq_ignore_ascii_case(&request.target.port)
                    && observation.firmware == firmware
                    && observation.command == name)
            });
            store.observations.push(CapabilityObservation {
                profile_id: profile_id.into(),
                port: request.target.port.clone(),
                firmware,
                command: name,
                query: result.request.clone(),
                response_lines: lines,
                updated_at: now,
            });
            changed = true;
        }
        if !changed {
            return Ok(false);
        }
        store
            .observations
            .sort_by_key(|observation| observation.updated_at);
        if store.observations.len() > 500 {
            store.observations.drain(..store.observations.len() - 500);
        }
        Ok(true)
    })
}

fn supported_numeric_range(line: &str, selector: &str) -> Option<(i64, i64)> {
    let quoted = regex::Regex::new(&format!(r#"(?i)["']{}["']"#, regex::escape(selector))).ok()?;
    let selector_match = quoted.find(line)?;
    let range = regex::Regex::new(r"\(?\s*(-?\d+)\s*(?:[-~]\s*(-?\d+))?\s*\)?").ok()?;
    let captures = range.captures(&line[selector_match.end()..])?;
    let minimum = captures.get(1)?.as_str().parse().ok()?;
    let maximum = captures
        .get(2)
        .and_then(|value| value.as_str().parse().ok())
        .unwrap_or(minimum);
    Some((minimum, maximum))
}

fn numeric_ranges(line: &str) -> Vec<(i64, i64)> {
    let Ok(range) = regex::Regex::new(r"\(?\s*(-?\d+)\s*(?:[-~]\s*(-?\d+))?\s*\)?") else {
        return Vec::new();
    };
    range
        .captures_iter(line)
        .filter_map(|captures| {
            let minimum = captures.get(1)?.as_str().parse().ok()?;
            let maximum = captures
                .get(2)
                .and_then(|value| value.as_str().parse().ok())
                .unwrap_or(minimum);
            Some((minimum, maximum))
        })
        .collect()
}

fn positional_parameter(command: &str, index: usize) -> Option<&'static str> {
    match command {
        "AT+MGPIO" => ["pin", "control", "pull"].get(index).copied(),
        "AT+MADC" => ["channel"].get(index).copied(),
        _ => None,
    }
}

/// 用已加载的观测表判断命令是否被实测能力否决。
///
/// 调用方负责 `load`——授权一次要判定多条命令，逐条重新读盘没有意义。
pub fn restriction(
    store: &CapabilityEvidenceStore,
    profile_id: Option<&str>,
    port: Option<&str>,
    raw: &str,
) -> Option<String> {
    let (profile_id, port, name) = (profile_id?, port?, command_name(raw)?);
    if raw.trim().ends_with("=?") {
        return None;
    }
    let current_firmware = store
        .port_firmware
        .get(port)
        .filter(|identity| identity.profile_id == profile_id)
        .map(|identity| identity.firmware.as_str());
    let observation = store.observations.iter().rev().find(|observation| {
        observation.profile_id == profile_id
            && observation.port.eq_ignore_ascii_case(port)
            && observation.command == name
            && observation.firmware.as_deref() == current_firmware
    })?;
    let arguments = split_arguments(raw);
    for (index, selector) in arguments.iter().enumerate() {
        let Some(value) = arguments
            .get(index + 1)
            .and_then(|value| value.parse::<i64>().ok())
        else {
            continue;
        };
        let Some((minimum, maximum)) = observation
            .response_lines
            .iter()
            .find_map(|line| supported_numeric_range(line, selector))
        else {
            continue;
        };
        if value < minimum || value > maximum {
            return Some(format!(
                "当前模组实测 {} 参数 {} 仅支持 {}，拒绝值 {}（固件：{}）",
                observation.query,
                selector,
                if minimum == maximum {
                    minimum.to_string()
                } else {
                    format!("{minimum}-{maximum}")
                },
                value,
                observation.firmware.as_deref().unwrap_or("未记录")
            ));
        }
    }
    // Some AT test responses expose positional ranges without parameter names,
    // e.g. `+MGPIO: (0-49),(0-3),(0-2)` and `+MADC: (0-1)`.
    if let Some(command) = positional_parameter(&name, 0).map(|_| name.as_str()) {
        for (index, value) in arguments.iter().enumerate() {
            let Ok(value) = value.parse::<i64>() else {
                continue;
            };
            let Some(selector) = positional_parameter(command, index) else {
                continue;
            };
            let Some((minimum, maximum)) = observation
                .response_lines
                .iter()
                .flat_map(|line| numeric_ranges(line).into_iter())
                .nth(index)
            else {
                continue;
            };
            if value < minimum || value > maximum {
                return Some(format!(
                    "当前模组实测 {} 参数 {} 仅支持 {}，拒绝值 {}（固件：{}）",
                    observation.query,
                    selector,
                    if minimum == maximum {
                        minimum.to_string()
                    } else {
                        format!("{minimum}-{maximum}")
                    },
                    value,
                    observation.firmware.as_deref().unwrap_or("未记录")
                ));
            }
        }
    }
    None
}

pub fn context(paths: &AppPaths, profile_id: Option<&str>, port: Option<&str>) -> String {
    let (Some(profile_id), Some(port)) = (profile_id, port) else {
        return "（当前没有绑定型号与串口的持久化实测记录。）".into();
    };
    let store = load(paths);
    let firmware = store
        .port_firmware
        .get(port)
        .filter(|item| item.profile_id == profile_id);
    let mut output = Vec::new();
    if let Some(firmware) = firmware {
        output.push(format!("固件: {}", firmware.firmware));
    }
    let observations: Vec<_> = store
        .observations
        .iter()
        .filter(|item| item.profile_id == profile_id && item.port.eq_ignore_ascii_case(port))
        .rev()
        .take(10)
        .collect();
    for observation in observations.into_iter().rev() {
        output.push(format!(
            ">> {}\n{}",
            observation.query,
            observation
                .response_lines
                .iter()
                .map(|line| format!("<< {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if output.is_empty() {
        "（当前型号/串口尚无持久化实测记录。）".into()
    } else {
        output.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_value_and_ranges() {
        assert_eq!(
            supported_numeric_range(r#"+MFTPCFG: (0-5),"ssl",(0,0)"#, "ssl"),
            Some((0, 0))
        );
        assert_eq!(
            supported_numeric_range(r#"+CFG: "mode",(0-2)"#, "mode"),
            Some((0, 2))
        );
    }

    #[test]
    fn splits_quoted_arguments() {
        assert_eq!(
            split_arguments(r#"AT+MFTPCFG=0,"ssl",1,0"#),
            vec!["0", "ssl", "1", "0"]
        );
    }

    #[test]
    fn parses_positional_gpio_and_adc_ranges() {
        assert_eq!(
            numeric_ranges("+MGPIO: (0-49),(0-3),(0-2)"),
            vec![(0, 49), (0, 3), (0, 2)]
        );
        assert_eq!(numeric_ranges("+MADC: (0-1)"), vec![(0, 1)]);
        assert_eq!(positional_parameter("AT+MGPIO", 1), Some("control"));
        assert_eq!(positional_parameter("AT+MADC", 0), Some("channel"));
    }

    #[test]
    fn persisted_positional_probe_blocks_out_of_range_value() {
        let unique = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let root = std::env::temp_dir().join(format!("modulink-positional-capability-{unique}"));
        std::fs::create_dir_all(&root).unwrap();
        let paths = AppPaths {
            resource_data_dir: root.clone(),
            config_dir: root.clone(),
            user_data_dir: root.clone(),
            legacy_data_dir: None,
        };
        let store = CapabilityEvidenceStore {
            schema: schema(),
            observations: vec![CapabilityObservation {
                profile_id: "ml307c".into(),
                port: "COM113".into(),
                firmware: None,
                command: "AT+MGPIO".into(),
                query: "AT+MGPIO=?".into(),
                response_lines: vec!["+MGPIO: (0-49),(0-3),(0-2)".into()],
                updated_at: 1,
            }],
            ..Default::default()
        };
        save(&paths, &store).unwrap();
        let reloaded = load(&paths);
        assert!(restriction(&reloaded, Some("ml307c"), Some("COM113"), "AT+MGPIO=0,4,0").is_some());
        assert!(restriction(&reloaded, Some("ml307c"), Some("COM113"), "AT+MGPIO=0,2,0").is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn persisted_probe_blocks_out_of_range_value_after_reload() {
        let unique = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let root = std::env::temp_dir().join(format!("modulink-capability-{unique}"));
        std::fs::create_dir_all(&root).unwrap();
        let paths = AppPaths {
            resource_data_dir: root.clone(),
            config_dir: root.clone(),
            user_data_dir: root.clone(),
            legacy_data_dir: None,
        };
        let store = CapabilityEvidenceStore {
            schema: schema(),
            port_firmware: HashMap::from([(
                "COM113".into(),
                PortFirmware {
                    profile_id: "ml307c".into(),
                    firmware: "ML307C-V1".into(),
                    updated_at: 1,
                },
            )]),
            observations: vec![CapabilityObservation {
                profile_id: "ml307c".into(),
                port: "COM113".into(),
                firmware: Some("ML307C-V1".into()),
                command: "AT+MFTPCFG".into(),
                query: "AT+MFTPCFG=?".into(),
                response_lines: vec![r#"+MFTPCFG: (0-5),"ssl",(0,0)"#.into()],
                updated_at: 2,
            }],
        };
        save(&paths, &store).unwrap();
        let reloaded = load(&paths);
        assert!(restriction(
            &reloaded,
            Some("ml307c"),
            Some("COM113"),
            r#"AT+MFTPCFG=0,"ssl",1,0"#
        )
        .is_some());
        assert!(restriction(
            &reloaded,
            Some("ml307c"),
            Some("COM113"),
            r#"AT+MFTPCFG=0,"ssl",0,0"#
        )
        .is_none());
        std::fs::remove_dir_all(root).ok();
    }
}
