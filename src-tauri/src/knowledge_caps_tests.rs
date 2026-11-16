//! 知识中心能力分级回归测试。
#![cfg(test)]
use crate::ipc::capabilities::default_probe_commands;
use crate::knowledge::sanitize_preview;
use crate::knowledge::{
    at_command_name, command_catalog_restriction, command_pattern, determine_effective_manual_mode,
    determine_manual_mode, file_stamp, load_validation_receipt, manual_domain_bonus,
    manual_search_terms, manual_workflow_bonus, prepare_authorized_flow,
    require_strict_command_authorization, save_validation_receipt, split_at_arguments,
    topic_applicability, CommandCatalogEntry, CommandParameterConstraint, KnowledgeSnapshot,
    ManualIndex, TopicMeta, TopicOwnerMeta,
};
use crate::types::{
    AtCommandAuthorization, Knowledge, KnowledgePackManifest, KnowledgePackValidation,
    ManualContentLevel,
};
use modulink_core::diag::{DiagFlow, DiagStep, EvalRule};
use modulink_core::knowledge::{DocumentApplicability, ModuleProfile};

#[test]
fn authorization_normalizes_at_command_names() {
    assert_eq!(
        at_command_name("AT+MHTTPREQUEST=0,2,15,/post").as_deref(),
        Some("AT+MHTTPREQUEST")
    );
    assert_eq!(
        at_command_name("  at+mftpconn=? ").as_deref(),
        Some("AT+MFTPCONN")
    );
    assert_eq!(at_command_name("ATI").as_deref(), Some("ATI"));
    assert_eq!(at_command_name("AT").as_deref(), Some("AT"));
    assert_eq!(at_command_name("MHTTPREQUEST=0").as_deref(), None);
    assert_eq!(at_command_name("AT+MHTTP!REQUEST=0").as_deref(), None);
}

#[test]
fn default_probe_commands_use_catalog_command_names() {
    let commands = default_probe_commands();
    assert!(commands.contains(&"AT+MQTTCFG=?"));
    assert!(commands.contains(&"AT+MFTPCFG=?"));
    assert!(commands.contains(&"AT+MSSLCFG=?"));
    assert!(!commands.contains(&"AT+MQTTCONFIG=?"));
    assert!(!commands.contains(&"AT+MFTPSLOGIN=?"));
    assert!(!commands.contains(&"AT+MSSLCONFIG=?"));
}

#[test]
fn authorization_command_match_has_boundaries() {
    let pattern = command_pattern("AT+MHTTPREQUEST");
    assert!(pattern.is_match("# AT+MHTTPREQUEST 发送 HTTP 请求"));
    assert!(pattern.is_match("AT+MHTTPREQUEST=0,2,15,/post"));
    assert!(!pattern.is_match("AT+MHTTPREQUESTEX=0"));
}

fn ml307c_profile() -> ModuleProfile {
    serde_yaml::from_str(
        "schema: test\nid: ml307c\nname: ML307C\nmodel_codes: [ML307C, ML307C-DC-CN]\nseries: ML307\n",
    )
    .unwrap()
}

fn ml307c_manifest() -> KnowledgePackManifest {
    KnowledgePackManifest {
        schema: "modulink/knowledge-pack@3".into(),
        id: "cmiot.manuals.ml307c".into(),
        version: "2026.08".into(),
        models: vec!["ML307C".into()],
        generated_at: String::new(),
        topic_count: 1,
        include_series: true,
        include_generic: false,
        include_unknown_reference: true,
        topic_counts: None,
    }
}

#[test]
fn effective_manual_mode_requires_current_profile_coverage() {
    let manifest = ml307c_manifest();
    let profile = ml307c_profile();
    let (mode, reason) = determine_effective_manual_mode(
        &ManualContentLevel::FullText,
        None,
        Some(&manifest),
        Some("ml307c"),
        Some(&profile),
    );
    assert_eq!(mode, ManualContentLevel::FullText);
    assert!(reason.is_none());

    let mut other = profile;
    other.id = "ml302".into();
    other.model_codes = vec!["ML302".into()];
    let (mode, reason) = determine_effective_manual_mode(
        &ManualContentLevel::FullText,
        None,
        Some(&manifest),
        Some("ml302"),
        Some(&other),
    );
    assert_eq!(mode, ManualContentLevel::Preview);
    assert!(reason.unwrap().contains("不在"));
}

fn catalog_entry() -> CommandCatalogEntry {
    CommandCatalogEntry {
        command: "AT+MFTPCFG".into(),
        topic_id: "ftp".into(),
        manual: "FTP用户手册".into(),
        models: vec!["ML307C".into()],
        scope_source: "explicit_table".into(),
        variants: vec![],
        unsupported_models: vec![],
        parameter_constraints: vec![CommandParameterConstraint {
            parameter: "ssl_mode".into(),
            selector: "ssl".into(),
            position: Some(2),
            models: vec!["ML307C".into()],
            allowed_values: vec!["0".into()],
            kind: "allowed".into(),
            values: vec!["0".into()],
            confidence: "verified".into(),
            enforce: Some(true),
            note: "ML307C：只支持通用FTP".into(),
        }],
    }
}

#[test]
fn authorization_denies_only_the_explicit_enum_value() {
    let mut entry = catalog_entry();
    entry.command = "AT+MGPIO".into();
    entry.parameter_constraints = vec![CommandParameterConstraint {
        parameter: "control".into(),
        selector: "control".into(),
        position: Some(1),
        models: vec!["ML307C".into()],
        allowed_values: vec![],
        kind: "denied".into(),
        values: vec!["3".into()],
        confidence: "verified".into(),
        enforce: Some(true),
        note: "ML307C：不支持高阻态".into(),
    }];
    assert!(
        command_catalog_restriction(&[&entry], "AT+MGPIO=0,3,0", Some(&ml307c_profile())).is_some()
    );
    assert!(
        command_catalog_restriction(&[&entry], "AT+MGPIO=0,2,0", Some(&ml307c_profile())).is_none()
    );
}

#[test]
fn authorization_ignores_review_required_constraints() {
    let mut entry = catalog_entry();
    entry.parameter_constraints[0].confidence = "review_required".into();
    entry.parameter_constraints[0].enforce = Some(false);
    assert!(command_catalog_restriction(
        &[&entry],
        "AT+MFTPCFG=0,\"ssl\",1,0",
        Some(&ml307c_profile())
    )
    .is_none());
}

#[test]
fn authorization_uses_positional_parameter_for_platform_restriction() {
    let mut entry = catalog_entry();
    entry.command = "AT+MIPLCFG".into();
    entry.parameter_constraints[0].parameter = "platform".into();
    entry.parameter_constraints[0].selector = "platform".into();
    entry.parameter_constraints[0].position = Some(2);
    entry.parameter_constraints[0].kind = "denied".into();
    entry.parameter_constraints[0].values = vec!["2".into()];
    entry.parameter_constraints[0].allowed_values.clear();
    assert!(command_catalog_restriction(
        &[&entry],
        "AT+MIPLCFG=\"platform\",0,2",
        Some(&ml307c_profile())
    )
    .is_some());
    assert!(command_catalog_restriction(
        &[&entry],
        "AT+MIPLCFG=\"platform\",0,1",
        Some(&ml307c_profile())
    )
    .is_none());
}

#[test]
fn authorization_requires_explicit_non_default_argument() {
    let mut entry = catalog_entry();
    entry.command = "AT+MADC".into();
    entry.parameter_constraints[0].parameter = "channel".into();
    entry.parameter_constraints[0].selector = "channel".into();
    entry.parameter_constraints[0].position = None;
    entry.parameter_constraints[0].kind = "required".into();
    entry.parameter_constraints[0].values.clear();
    entry.parameter_constraints[0].allowed_values.clear();
    assert!(command_catalog_restriction(&[&entry], "AT+MADC", Some(&ml307c_profile())).is_some());
    assert!(command_catalog_restriction(&[&entry], "AT+MADC=0", Some(&ml307c_profile())).is_none());
}

#[test]
fn authorization_enforces_model_parameter_values() {
    let entry = catalog_entry();
    let profile = ml307c_profile();
    assert!(
        command_catalog_restriction(&[&entry], "AT+MFTPCFG=0,\"ssl\",1,0", Some(&profile))
            .is_some()
    );
    assert!(
        command_catalog_restriction(&[&entry], "AT+MFTPCFG=0,\"ssl\",0,0", Some(&profile))
            .is_none()
    );
    assert_eq!(
        split_at_arguments("AT+MFTPCFG=0,\"ssl\",1,0"),
        vec!["0", "ssl", "1", "0"]
    );
}

#[test]
fn authorization_enforces_whole_command_exclusions() {
    let mut entry = catalog_entry();
    entry.command = "AT+MSSLPSK".into();
    entry.unsupported_models = vec!["ML307C".into()];
    assert!(command_catalog_restriction(
        &[&entry],
        "AT+MSSLPSK=\"id\",\"secret\"",
        Some(&ml307c_profile())
    )
    .is_some());
}

fn flow_decision(command: &str, authorized: bool, reason_code: &str) -> AtCommandAuthorization {
    AtCommandAuthorization {
        command: command.into(),
        authorized,
        applicability: None,
        manual: None,
        topic_id: None,
        pack_id: None,
        pack_version: None,
        reason_code: reason_code.into(),
        reason: reason_code.into(),
    }
}

fn flow_rule(collect: Vec<&str>) -> EvalRule {
    EvalRule {
        when: None,
        default: true,
        result: "pass".into(),
        detail: None,
        reason: None,
        suggestion: None,
        collect: collect.into_iter().map(str::to_string).collect(),
        abort: false,
    }
}

fn authorization_flow(verified: bool) -> DiagFlow {
    DiagFlow {
        schema: "test".into(),
        id: "authorization".into(),
        name: "授权测试".into(),
        applies_to: Vec::new(),
        kind: "usecase".into(),
        category: "test".into(),
        source: "test".into(),
        verified,
        execution_mode: "readonly".into(),
        steps: vec![
            DiagStep {
                id: "optional".into(),
                title: "可选步骤".into(),
                send: vec!["AT+OPTIONAL".into()],
                timeout_ms: 1000,
                parse: Some("ignored".into()),
                retry: None,
                optional: true,
                expect_urc: None,
                evaluate: vec![flow_rule(vec!["AT+OPTIONALCOLLECT"])],
            },
            DiagStep {
                id: "required".into(),
                title: "必需步骤".into(),
                send: vec!["AT+REQUIRED".into()],
                timeout_ms: 1000,
                parse: None,
                retry: None,
                optional: false,
                expect_urc: None,
                evaluate: vec![flow_rule(vec!["AT+COLLECT"])],
            },
        ],
    }
}

#[test]
fn flow_authorization_filters_optional_and_collection_without_reordering_steps() {
    let prepared = prepare_authorized_flow(
        authorization_flow(true),
        vec![
            flow_decision("AT+OPTIONAL", false, "command_not_found"),
            flow_decision("AT+OPTIONALCOLLECT", false, "command_not_found"),
            flow_decision("AT+REQUIRED", true, "authorized"),
            flow_decision("AT+COLLECT", false, "command_not_found"),
        ],
        false,
    )
    .unwrap();

    assert_eq!(prepared.steps.len(), 2);
    assert_eq!(prepared.steps[0].id, "optional");
    assert!(prepared.steps[0].send.is_empty());
    assert_eq!(prepared.steps[0].evaluate[0].result, "skip");
    assert_eq!(prepared.steps[1].id, "required");
    assert_eq!(prepared.steps[1].send, vec!["AT+REQUIRED"]);
    assert!(prepared.steps[1].evaluate[0].collect.is_empty());
}

#[test]
fn unverified_confirmation_only_bypasses_missing_evidence() {
    let evidence_gap = vec![
        flow_decision("AT+OPTIONAL", false, "no_verified_fulltext"),
        flow_decision("AT+OPTIONALCOLLECT", false, "command_not_exact"),
        flow_decision("AT+REQUIRED", false, "command_not_exact"),
        flow_decision("AT+COLLECT", false, "no_verified_fulltext"),
    ];
    assert!(
        prepare_authorized_flow(authorization_flow(false), evidence_gap.clone(), false).is_err()
    );
    assert!(prepare_authorized_flow(authorization_flow(false), evidence_gap, true).is_ok());

    let verified_gap = vec![
        flow_decision("AT+OPTIONAL", false, "no_verified_fulltext"),
        flow_decision("AT+OPTIONALCOLLECT", false, "command_not_exact"),
        flow_decision("AT+REQUIRED", false, "command_not_exact"),
        flow_decision("AT+COLLECT", false, "no_verified_fulltext"),
    ];
    assert!(prepare_authorized_flow(authorization_flow(true), verified_gap, true).is_err());

    let hard_denial = vec![
        flow_decision("AT+OPTIONAL", true, "authorized"),
        flow_decision("AT+OPTIONALCOLLECT", true, "authorized"),
        flow_decision("AT+REQUIRED", false, "model_capability_restricted"),
        flow_decision("AT+COLLECT", true, "authorized"),
    ];
    let error = prepare_authorized_flow(authorization_flow(false), hard_denial, true).unwrap_err();
    assert!(error.contains("model_capability_restricted"));

    let not_found = vec![
        flow_decision("AT+OPTIONAL", true, "authorized"),
        flow_decision("AT+OPTIONALCOLLECT", true, "authorized"),
        flow_decision("AT+REQUIRED", false, "command_not_found"),
        flow_decision("AT+COLLECT", true, "authorized"),
    ];
    let error = prepare_authorized_flow(authorization_flow(false), not_found, true).unwrap_err();
    assert!(error.contains("command_not_found"));
}

#[test]
fn manual_recovery_commands_always_require_strict_authorization() {
    let commands = vec!["AT+MIPCLOSE=0".to_string()];
    let missing_evidence = vec![flow_decision("AT+MIPCLOSE", false, "no_verified_fulltext")];
    let error = require_strict_command_authorization("场景恢复指令", &commands, &missing_evidence)
        .unwrap_err();
    assert!(error.contains("no_verified_fulltext"));

    let authorized = vec![flow_decision("AT+MIPCLOSE", true, "authorized")];
    assert!(require_strict_command_authorization("场景恢复指令", &commands, &authorized).is_ok());
}

#[test]
fn sanitize_removes_internal_paths() {
    let input = "# AT+CSQ 手册：ML307C 仓库：ml307c 来源：developmentdomain/ml307c/reusable/at.dita 信号质量";
    let cleaned = sanitize_preview(input);
    assert!(
        !cleaned.contains("developmentdomain/"),
        "should not contain developmentdomain"
    );
    assert!(
        !cleaned.contains("仓库："),
        "should not contain warehouse marker"
    );
    assert!(
        !cleaned.contains("来源："),
        "should not contain source marker"
    );
    assert!(
        !cleaned.contains("anythingllm_kb"),
        "should not contain anythingllm_kb"
    );
}

#[test]
fn sanitize_preserves_content() {
    let input = "# AT+CSQ 手册：ML307C 仓库：ml307c 来源：developmentdomain/ml307c/reusable/at.dita 信号质量查询命令";
    let cleaned = sanitize_preview(input);
    assert!(cleaned.contains("AT+CSQ"), "should preserve command name");
    assert!(
        cleaned.contains("信号质量"),
        "should preserve summary content"
    );
    assert!(!cleaned.is_empty(), "should not be empty");
}

#[test]
fn sanitize_handles_pure_content() {
    let input = "# AT+CGDCONT 设置 PDP 上下文";
    let cleaned = sanitize_preview(input);
    assert_eq!(cleaned, input, "pure content should be preserved");
}

#[test]
fn sanitize_handles_empty() {
    let cleaned = sanitize_preview("");
    assert!(cleaned.is_empty());
}

#[test]
fn manual_search_terms_split_mixed_http_question() {
    let mixed = manual_search_terms("HTTP POST怎么完整使用");
    assert_eq!(mixed.words, vec!["http", "post"]);
    // 长中文串只提供 bigram 弱信号，不能冒充用户输入的整词。
    assert_eq!(mixed.bigrams, vec!["怎么", "么完", "完整", "整使", "使用"]);

    let command = manual_search_terms("AT+MHTTPREQUEST 参数");
    assert_eq!(command.words, vec!["at+mhttprequest", "参数"]);
    assert!(command.bigrams.is_empty());
}

#[test]
fn bigrams_cannot_outrank_an_exact_command_term() {
    // "MQTT怎么配置"：无关主题靠 3 个 bigram 得 6 分，命中 mqtt 的主题至少 8 分。
    let terms = manual_search_terms("MQTT怎么配置");
    let bigram_only: i32 = terms.bigrams.len() as i32 * 2;
    assert_eq!(terms.words, vec!["mqtt"]);
    assert!(
        bigram_only < 8,
        "bigram 合计 {bigram_only} 不应达到单个 ASCII 整词的标题权重"
    );
}

/// 构造只含 generic + 目标画像的知识库，用于验证 3GPP 基线的继承规则。
fn knowledge_with_generic_baseline(profile_yaml: &str) -> Knowledge {
    let generic: ModuleProfile = serde_yaml::from_str(
        r#"
schema: modulink/profile@1
id: generic
name: 通用蜂窝模组
family: generic
series: generic
capabilities: [cellular, 3gpp]
support_level: generic
at_commands:
  - cmd: "AT+CGMM"
    risk: none
  - cmd: "AT+CFUN"
    risk: reboot
"#,
    )
    .unwrap();
    let profile: ModuleProfile = serde_yaml::from_str(profile_yaml).unwrap();
    let mut store = modulink_core::knowledge::KnowledgeStore::default();
    store.profiles = vec![generic, profile];
    Knowledge(parking_lot::RwLock::new(store))
}

#[test]
fn cellular_profile_inherits_3gpp_baseline_without_a_model_specific_manual() {
    // ML307C 不在通用 AT 参考手册的“适用型号”表里，但 3GPP 标准指令仍应可用。
    let knowledge = knowledge_with_generic_baseline(
        "schema: modulink/profile@1\nid: ml307c\nname: ML307C\nmodel_codes: [ML307C]\nseries: ML307\ncapabilities: [cellular, lte-cat1]\n",
    );
    let snapshot = KnowledgeSnapshot::capture(&knowledge, Some("ml307c"));
    assert!(snapshot.standard_3gpp.contains("AT+CGMM"));
    assert!(snapshot.standard_3gpp.contains("AT+CFUN"));
    // 基线只补 3GPP，不能顺带放行厂商私有扩展。
    assert!(!snapshot.standard_3gpp.contains("AT+MHTTPCFG"));
}

#[test]
fn non_cellular_and_unselected_profiles_get_no_baseline() {
    let knowledge = knowledge_with_generic_baseline(
        "schema: modulink/profile@1\nid: sensor\nname: Sensor\nmodel_codes: [SEN1]\ncapabilities: [lora]\n",
    );
    assert!(KnowledgeSnapshot::capture(&knowledge, Some("sensor"))
        .standard_3gpp
        .is_empty());
    // 未选画像时没有型号上下文，不给任何执行授权基线。
    assert!(KnowledgeSnapshot::capture(&knowledge, None)
        .standard_3gpp
        .is_empty());
}

#[test]
fn unknown_owner_never_becomes_exact_evidence() {
    let profile: ModuleProfile = serde_yaml::from_str(
        "schema: test\nid: ml307c\nname: ML307C\nmodel_codes: [ML307C]\nseries: ML307\n",
    )
    .unwrap();
    let topic = TopicMeta {
        owners: vec![TopicOwnerMeta {
            map_id: "unknown/manual.ditamap".into(),
            manual: "Unknown".into(),
            models: vec![],
            scope_source: "unknown".into(),
            language: "zh".into(),
        }],
        ..Default::default()
    };
    assert_eq!(
        topic_applicability(&topic, Some(&profile)).0,
        DocumentApplicability::Unknown
    );
}

#[test]
fn workflow_search_prioritizes_examples_and_connection_steps() {
    let query = "FTP 指令有哪些";
    assert!(
        manual_workflow_bonus(query, "FTP示例")
            > manual_workflow_bonus(query, "AT+MFTPCONN 建立FTP连接")
    );
    assert!(
        manual_workflow_bonus(query, "AT+MFTPCONN 建立FTP连接")
            > manual_workflow_bonus(query, "AT+MFTPRETR 从服务器获取文件")
    );
    assert_eq!(manual_workflow_bonus("AT+MFTPRETR 参数", "FTP示例"), 0);
}

#[test]
fn protocol_search_prioritizes_its_canonical_manual() {
    assert_eq!(
        manual_domain_bonus("HTTP 有哪些指令", "HTTP示例", "HTTP/HTTPS用户手册"),
        40
    );
    assert_eq!(
        manual_domain_bonus("HTTP 有哪些指令", "编程设计注意", "ML307C 固件升级指导手册"),
        0
    );
    assert_eq!(
        manual_domain_bonus("HTTP/HTTPS 怎么使用", "HTTP示例", "HTTP/HTTPS用户手册"),
        40
    );
}

#[test]
fn validation_receipt_matches_contract_stamps_and_invalidates_on_change() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kb_receipt_test_{unique}"));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest_path = dir.join("manifest.json");
    let map_path = dir.join("topic_paths.jsonl");
    std::fs::write(&manifest_path, "{}").unwrap();
    std::fs::write(&map_path, "{}\n").unwrap();
    let paths = AppPaths {
        resource_data_dir: dir.clone(),
        config_dir: dir.clone(),
        user_data_dir: dir.clone(),
        legacy_data_dir: None,
    };
    let validation = KnowledgePackValidation {
        valid: true,
        manifest: None,
        topic_paths_count: 1,
        sample_readable: 1,
        sample_total: 1,
        errors: vec![],
    };
    let manifest_stamp = file_stamp(&manifest_path);
    let map_stamp = file_stamp(&map_path);
    save_validation_receipt(&paths, "E:/pack", manifest_stamp, map_stamp, &validation);
    assert!(load_validation_receipt(&paths, "E:/pack", manifest_stamp, map_stamp).is_some());

    std::fs::write(&manifest_path, "{\"changed\":true}").unwrap();
    assert!(
        load_validation_receipt(&paths, "E:/pack", file_stamp(&manifest_path), map_stamp).is_none()
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sanitize_removes_internal_template_phrases() {
    let input = "研发中心产品手册发布项目模板 仅供文档工程师使用 AT+CSQ 信号质量";
    let cleaned = sanitize_preview(input);
    assert!(
        !cleaned.contains("研发中心产品手册发布项目模板"),
        "should remove template phrase"
    );
    assert!(
        !cleaned.contains("仅供文档工程师使用"),
        "should remove internal-only phrase"
    );
    assert!(cleaned.contains("AT+CSQ"), "should preserve command");
}

#[test]
fn manual_content_level_serialization() {
    use crate::types::ManualContentLevel;
    let none = serde_json::to_string(&ManualContentLevel::None).unwrap();
    let preview = serde_json::to_string(&ManualContentLevel::Preview).unwrap();
    let fulltext = serde_json::to_string(&ManualContentLevel::FullText).unwrap();
    assert_eq!(none, r#""none""#);
    assert_eq!(preview, r#""preview""#);
    assert_eq!(fulltext, r#""fulltext""#);
}

#[test]
fn manual_mode_none_when_index_empty() {
    // 无索引时无论是否配了目录都应降级为 none
    assert_eq!(
        determine_manual_mode(true, "/nonexistent", true),
        ManualContentLevel::None
    );
    assert_eq!(
        determine_manual_mode(true, "", false),
        ManualContentLevel::None
    );
}

#[test]
fn manual_mode_preview_when_no_dir() {
    // 有索引但未配目录 → preview
    assert_eq!(
        determine_manual_mode(false, "", true),
        ManualContentLevel::Preview
    );
}

#[test]
fn manual_mode_preview_when_invalid_dir() {
    // 有索引、目录不存在 → preview（无效目录降级）
    assert_eq!(
        determine_manual_mode(false, "Z:/no/such/dir/xyz", true),
        ManualContentLevel::Preview
    );
    // 目录存在但采样文件不可读 → preview
    let temp = std::env::temp_dir();
    assert_eq!(
        determine_manual_mode(false, temp.to_str().unwrap(), false),
        ManualContentLevel::Preview
    );
}

#[test]
fn manual_mode_fulltext_when_dir_and_samples_readable() {
    // 有索引、目录存在、采样可读 → fulltext
    let temp = std::env::temp_dir();
    assert_eq!(
        determine_manual_mode(false, temp.to_str().unwrap(), true),
        ManualContentLevel::FullText
    );
}

#[test]
fn public_index_has_no_internal_paths() {
    // 公开索引不得包含内部仓库路径、来源标记或内部模板语句，且不得有 path 字段
    let manifest = env!("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(manifest)
        .join("..")
        .join("data")
        .join("kb_index")
        .join("manual_topics_public.jsonl");
    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("公开索引文件不存在: {:?}", path));
    let markers = [
        "developmentdomain/",
        "仓库：",
        "来源：",
        "anythingllm_kb",
        "研发中心产品手册发布项目模板",
        "仅供文档工程师使用",
    ];
    // 检查仓库标识和 .dita/.md 文件名残留
    let repo_re = regex::Regex::new(r"`[a-z0-9][a-z0-9_]{3,}`").unwrap();
    let file_re = regex::Regex::new(r"\w+\.(dita|md)").unwrap();
    let mut leaks = 0;
    let mut has_path_field = 0;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("path").is_some() {
                has_path_field += 1;
            }
        }
        for m in markers {
            if line.contains(m) {
                leaks += 1;
                break;
            }
        }
        if repo_re.is_match(line) || file_re.is_match(line) {
            leaks += 1;
        }
    }
    assert_eq!(
        has_path_field, 0,
        "公开索引中有 {} 条含 path 字段",
        has_path_field
    );
    assert_eq!(
        leaks, 0,
        "公开索引中发现 {} 条含内部标记/仓库标识/文件名的记录",
        leaks
    );
}

#[test]
fn sanitize_removes_repo_identifiers_and_filenames() {
    let input = "手册：ML307C `5glanfunctionusageguide` 类型：guide ` AT+CSQ 参考 `g_ml307c_examples.dita` 信号质量";
    let cleaned = sanitize_preview(input);
    assert!(
        !cleaned.contains("`5glanfunctionusageguide`"),
        "should remove backtick repo id"
    );
    assert!(!cleaned.contains(".dita"), "should remove .dita filename");
    assert!(
        !cleaned.contains("类型：guide"),
        "should remove type metadata"
    );
    assert!(cleaned.contains("AT+CSQ"), "should preserve command");
    assert!(cleaned.contains("信号质量"), "should preserve content");
}

#[test]
fn public_index_previews_non_empty() {
    // 脱敏后的 preview 不应被清空
    let manifest = env!("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(manifest)
        .join("..")
        .join("data")
        .join("kb_index")
        .join("manual_topics_public.jsonl");
    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("公开索引文件不存在: {:?}", path));
    let total = content.lines().filter(|l| !l.trim().is_empty()).count();
    let empty = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["preview"].as_str().unwrap_or("").trim().is_empty())
        .count();
    assert!(total > 0, "公开索引不应为空");
    assert_eq!(empty, 0, "公开索引中有 {} 条 preview 为空", empty);
}

use crate::knowledge::is_safe_manual_path;

#[test]
fn safe_path_rejects_traversal() {
    let root = std::path::Path::new("/tmp/kb");
    assert!(!is_safe_manual_path(root, "../../etc/passwd"), "reject ..");
    assert!(
        !is_safe_manual_path(root, "../../../private.txt"),
        "reject deep .."
    );
    assert!(!is_safe_manual_path(root, "/etc/passwd"), "reject absolute");
    assert!(
        !is_safe_manual_path(root, "C:/Windows/system32/config.md"),
        "reject drive prefix"
    );
    assert!(
        !is_safe_manual_path(root, "\\\\server\\share\\secret.md"),
        "reject UNC"
    );
}

#[test]
fn safe_path_rejects_non_md() {
    let root = std::path::Path::new("/tmp/kb");
    assert!(
        !is_safe_manual_path(root, "../../secret.txt"),
        "reject non-md with .."
    );
    assert!(
        !is_safe_manual_path(root, "config.yaml"),
        "reject non-md extension"
    );
    assert!(!is_safe_manual_path(root, ""), "reject empty");
}

#[test]
fn safe_path_accepts_valid_relative_md() {
    // 创建临时知识包目录和一个 .md 文件
    let tmp = std::env::temp_dir().join("kb_traversal_test");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("valid.md"), "# test").unwrap();
    assert!(
        is_safe_manual_path(&tmp, "valid.md"),
        "accept valid relative .md"
    );
    std::fs::remove_dir_all(&tmp).ok();
}

// ===== 知识包验证集成测试 =====
use crate::app_paths::AppPaths;
use crate::knowledge::validate_pack_internal;
use parking_lot::RwLock;

/// 构建测试用 AppPaths（资源目录指向项目根 data/)
fn test_paths() -> AppPaths {
    let manifest = env!("CARGO_MANIFEST_DIR");
    AppPaths {
        resource_data_dir: std::path::PathBuf::from(manifest).join("..").join("data"),
        config_dir: std::env::temp_dir(),
        user_data_dir: std::env::temp_dir(),
        legacy_data_dir: None,
    }
}

fn test_manual_state() -> ManualIndex {
    ManualIndex(std::sync::Arc::new(RwLock::new(None)))
}

fn write_pack(dir: &std::path::Path, manifest_json: &str, topic_paths: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("manifest.json"), manifest_json).unwrap();
    std::fs::write(dir.join("topic_paths.jsonl"), topic_paths).unwrap();
}

fn make_manual_state_with_ids(ids: &[&str]) -> ManualIndex {
    let topics: Vec<TopicMeta> = ids
        .iter()
        .map(|id| TopicMeta {
            topic_id: id.to_string(),
            title: format!("Topic {}", id),
            manual: "Test Manual".into(),
            kind: "guide".into(),
            preview: "test preview".into(),
            models: vec!["ML307C".into()],
            variants: vec![],
            ..Default::default()
        })
        .collect();
    ManualIndex(std::sync::Arc::new(RwLock::new(Some(std::sync::Arc::new(
        topics,
    )))))
}

#[test]
fn validate_rejects_missing_manifest() {
    let dir = std::env::temp_dir().join("kb_test_no_manifest");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("manifest")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_wrong_schema() {
    let dir = std::env::temp_dir().join("kb_test_wrong_schema");
    std::fs::remove_dir_all(&dir).ok();
    let m = "{\"schema\":\"wrong@1\",\"id\":\"test\",\"version\":\"1.0\",\"models\":[],\"generatedAt\":\"\",\"topicCount\":1}";
    let tp = "{\"topic_id\":\"abc\",\"path\":\"x.md\"}";
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("schema")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_empty_id() {
    let dir = std::env::temp_dir().join("kb_test_empty_id");
    std::fs::remove_dir_all(&dir).ok();
    let m = "{\"schema\":\"modulink/knowledge-pack@1\",\"id\":\"   \",\"version\":\"1.0\",\"models\":[],\"generatedAt\":\"\",\"topicCount\":1}";
    let tp = "{\"topic_id\":\"abc\",\"path\":\"x.md\"}";
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("id")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_zero_topic_count() {
    let dir = std::env::temp_dir().join("kb_test_zero_count");
    std::fs::remove_dir_all(&dir).ok();
    let m = "{\"schema\":\"modulink/knowledge-pack@1\",\"id\":\"test\",\"version\":\"1.0\",\"models\":[],\"generatedAt\":\"\",\"topicCount\":0}";
    let tp = "{\"topic_id\":\"abc\",\"path\":\"x.md\"}";
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("topicCount")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_mismatched_topic_count() {
    let dir = std::env::temp_dir().join("kb_test_mismatch");
    std::fs::remove_dir_all(&dir).ok();
    let m = "{\"schema\":\"modulink/knowledge-pack@1\",\"id\":\"test\",\"version\":\"1.0\",\"models\":[],\"generatedAt\":\"\",\"topicCount\":99}";
    let tp = "{\"topic_id\":\"abc\",\"path\":\"x.md\"}";
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("不一致")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_unsafe_path_in_map() {
    let dir = std::env::temp_dir().join("kb_test_unsafe_path");
    std::fs::remove_dir_all(&dir).ok();
    let m = "{\"schema\":\"modulink/knowledge-pack@1\",\"id\":\"test\",\"version\":\"1.0\",\"models\":[],\"generatedAt\":\"\",\"topicCount\":1}";
    let tp = "{\"topic_id\":\"abc\",\"path\":\"../../etc/passwd.md\"}";
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result
        .errors
        .iter()
        .any(|e| e.contains("不安全") || e.contains("格式")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_corrupted_jsonl() {
    let dir = std::env::temp_dir().join("kb_test_corrupt_jsonl");
    std::fs::remove_dir_all(&dir).ok();
    let m = "{\"schema\":\"modulink/knowledge-pack@1\",\"id\":\"test\",\"version\":\"1.0\",\"models\":[],\"generatedAt\":\"\",\"topicCount\":1}";
    let tp = "this is not json";
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_nonexistent_dir() {
    let result = validate_pack_internal(&test_paths(), &test_manual_state(), "Z:/no/such/dir/xyz");
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("目录")));
}

#[test]
fn validate_rejects_empty_version() {
    let dir = std::env::temp_dir().join("kb_test_empty_version");
    std::fs::remove_dir_all(&dir).ok();
    let m = "{\"schema\":\"modulink/knowledge-pack@1\",\"id\":\"test\",\"version\":\"   \",\"models\":[],\"generatedAt\":\"\",\"topicCount\":1}";
    let tp = "{\"topic_id\":\"abc\",\"path\":\"x.md\"}";
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("version")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_duplicate_topic_id() {
    let dir = std::env::temp_dir().join("kb_test_dup_id");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.md"), "# A").unwrap();
    std::fs::write(dir.join("b.md"), "# B").unwrap();
    // topicCount 设为 1，避免“计数不一致”掩盖重复 ID 检测回归。
    let m = r#"{"schema":"modulink/knowledge-pack@1","id":"test","version":"1.0","models":[],"generatedAt":"","topicCount":1}"#;
    let tp = r#"{"topic_id":"abc","path":"a.md"}
{"topic_id":"abc","path":"b.md"}"#;
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["abc"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("重复 topic_id")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_unknown_topic_id() {
    let dir = std::env::temp_dir().join("kb_test_unknown_id");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t1.md"), "# T1").unwrap();
    std::fs::write(dir.join("unknown.md"), "# Unknown").unwrap();
    let m = r#"{"schema":"modulink/knowledge-pack@1","id":"test","version":"1.0","models":[],"generatedAt":"","topicCount":2}"#;
    let tp = r#"{"topic_id":"t1","path":"t1.md"}
{"topic_id":"unknown","path":"unknown.md"}"#;
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("未知 topic_id")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_rejects_missing_mapped_file() {
    let dir = std::env::temp_dir().join("kb_test_missing_file");
    std::fs::remove_dir_all(&dir).ok();
    let m = r#"{"schema":"modulink/knowledge-pack@1","id":"test","version":"1.0","models":[],"generatedAt":"","topicCount":1}"#;
    let tp = r#"{"topic_id":"t1","path":"missing.md"}"#;
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(!result.valid);
    assert!(result.errors.iter().any(|e| e.contains("文件不存在")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_accepts_valid_pack() {
    let dir = std::env::temp_dir().join("kb_test_valid");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let ids = ["t1", "t2", "t3"];
    let mut tp_lines = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let fname = format!("topic{}.md", i);
        std::fs::write(dir.join(&fname), "# Test").unwrap();
        tp_lines.push(format!(r#"{{"topic_id":"{}","path":"{}"}}"#, id, fname));
    }
    let tp = tp_lines.join("\n");
    let m = format!(
        r#"{{"schema":"modulink/knowledge-pack@1","id":"test.valid","version":"1.0","models":["ML307C"],"generatedAt":"2026-07-31","topicCount":{}}}"#,
        ids.len()
    );
    write_pack(&dir, &m, &tp);
    let state = make_manual_state_with_ids(&ids);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(
        result.valid,
        "valid pack should pass, errors: {:?}",
        result.errors
    );
    assert_eq!(result.topic_paths_count, ids.len());
    assert!(result.manifest.is_some());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_accepts_complete_pack_with_less_than_three_topics() {
    let dir = std::env::temp_dir().join("kb_test_valid_small");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t1.md"), "# T1").unwrap();
    let m = r#"{"schema":"modulink/knowledge-pack@1","id":"test.small","version":"1.0","models":[],"generatedAt":"","topicCount":1}"#;
    let tp = r#"{"topic_id":"t1","path":"t1.md"}"#;
    write_pack(&dir, m, tp);
    let state = make_manual_state_with_ids(&["t1"]);
    let result = validate_pack_internal(&test_paths(), &state, dir.to_str().unwrap());
    assert!(
        result.valid,
        "small complete pack should pass: {:?}",
        result.errors
    );
    assert_eq!(result.sample_readable, 1);
    assert_eq!(result.sample_total, 1);
    std::fs::remove_dir_all(&dir).ok();
}

fn make_scoped_manual_state() -> ManualIndex {
    let topics = vec![
        TopicMeta {
            topic_id: "exact".into(),
            title: "Exact".into(),
            manual: "ML307C Manual".into(),
            kind: "guide".into(),
            preview: "exact".into(),
            models: vec!["ML307C".into()],
            variants: vec![],
            ..Default::default()
        },
        TopicMeta {
            topic_id: "series".into(),
            title: "Series".into(),
            manual: "ML307A Manual".into(),
            kind: "guide".into(),
            preview: "series".into(),
            models: vec!["ML307A".into()],
            variants: vec![],
            ..Default::default()
        },
        TopicMeta {
            topic_id: "generic".into(),
            title: "Generic".into(),
            manual: "Common Manual".into(),
            kind: "guide".into(),
            preview: "generic".into(),
            models: vec![],
            variants: vec![],
            ..Default::default()
        },
        TopicMeta {
            topic_id: "other".into(),
            title: "Other".into(),
            manual: "MR880A Manual".into(),
            kind: "guide".into(),
            preview: "other".into(),
            models: vec!["MR880A".into()],
            variants: vec![],
            ..Default::default()
        },
    ];
    ManualIndex(std::sync::Arc::new(RwLock::new(Some(std::sync::Arc::new(
        topics,
    )))))
}

fn scoped_manifest(topic_count: usize) -> String {
    format!(
        r#"{{"schema":"modulink/knowledge-pack@2","id":"test.scoped","version":"1.0","models":["ML307C"],"generatedAt":"2026-07-31","topicCount":{},"includeSeries":true,"includeGeneric":true,"topicCounts":{{"exact":1,"series":1,"generic":1}}}}"#,
        topic_count
    )
}

#[test]
fn validate_v2_accepts_complete_model_scope() {
    let dir = std::env::temp_dir().join("kb_test_v2_valid_scope");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    for id in ["exact", "series", "generic"] {
        std::fs::write(dir.join(format!("{id}.md")), format!("# {id}")).unwrap();
    }
    let mappings = ["exact", "series", "generic"]
        .iter()
        .map(|id| format!(r#"{{"topic_id":"{id}","path":"{id}.md"}}"#))
        .collect::<Vec<_>>()
        .join("\n");
    write_pack(&dir, &scoped_manifest(3), &mappings);

    let result = validate_pack_internal(
        &test_paths(),
        &make_scoped_manual_state(),
        dir.to_str().unwrap(),
    );
    assert!(result.valid, "scoped pack should pass: {:?}", result.errors);
    assert_eq!(result.topic_paths_count, 3);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_v2_rejects_missing_scope_topic() {
    let dir = std::env::temp_dir().join("kb_test_v2_missing_scope");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    for id in ["exact", "generic"] {
        std::fs::write(dir.join(format!("{id}.md")), format!("# {id}")).unwrap();
    }
    let mappings = r#"{"topic_id":"exact","path":"exact.md"}
{"topic_id":"generic","path":"generic.md"}"#;
    write_pack(&dir, &scoped_manifest(2), mappings);

    let result = validate_pack_internal(
        &test_paths(),
        &make_scoped_manual_state(),
        dir.to_str().unwrap(),
    );
    assert!(!result.valid);
    assert!(result
        .errors
        .iter()
        .any(|error| error.contains("缺少路径映射")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_reports_stale_pack_instead_of_burying_it_in_coverage_errors() {
    // 旧编译器按路径哈希生成 topic_id，新索引按正文哈希——整包 id 空间不相交。
    let dir = std::env::temp_dir().join("kb_test_stale_id_space");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    for id in ["legacy_a", "legacy_b"] {
        std::fs::write(dir.join(format!("{id}.md")), format!("# {id}")).unwrap();
    }
    let mappings = ["legacy_a", "legacy_b"]
        .iter()
        .map(|id| format!(r#"{{"topic_id":"{id}","path":"{id}.md"}}"#))
        .collect::<Vec<_>>()
        .join("\n");
    write_pack(&dir, &scoped_manifest(2), &mappings);

    let result = validate_pack_internal(
        &test_paths(),
        &make_scoped_manual_state(),
        dir.to_str().unwrap(),
    );
    assert!(!result.valid);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("重新导出知识包")),
        "应给出可操作的重新导出提示: {:?}",
        result.errors
    );
    assert!(
        !result
            .errors
            .iter()
            .any(|error| error.contains("缺少路径映射")),
        "id 空间整体不匹配时不应再叠加覆盖率错误: {:?}",
        result.errors
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_v2_rejects_out_of_scope_topic() {
    let dir = std::env::temp_dir().join("kb_test_v2_out_of_scope");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    for id in ["exact", "series", "generic", "other"] {
        std::fs::write(dir.join(format!("{id}.md")), format!("# {id}")).unwrap();
    }
    let mappings = ["exact", "series", "generic", "other"]
        .iter()
        .map(|id| format!(r#"{{"topic_id":"{id}","path":"{id}.md"}}"#))
        .collect::<Vec<_>>()
        .join("\n");
    write_pack(&dir, &scoped_manifest(4), &mappings);

    let result = validate_pack_internal(
        &test_paths(),
        &make_scoped_manual_state(),
        dir.to_str().unwrap(),
    );
    assert!(!result.valid);
    assert!(result
        .errors
        .iter()
        .any(|error| error.contains("超出声明的型号范围")));
    std::fs::remove_dir_all(&dir).ok();
}
