pub mod authorization;
pub mod capabilities;
pub mod catalog;
pub mod manual;
pub mod pack;

// Re-exports: keep crate::knowledge::X paths working for ipc modules and tests.
#[allow(unused_imports)]
pub(crate) use authorization::{
    applicable_at_doc_commands, authorize_at_commands_blocking, authorize_flow_execution,
    flow_at_commands, flow_command_allowed, prepare_authorized_flow,
    require_strict_command_authorization, KnowledgeSnapshot,
};
#[allow(unused_imports)]
pub(crate) use capabilities::{build_knowledge_capabilities, determine_manual_mode};
#[allow(unused_imports)]
pub(crate) use catalog::{
    at_command_name, catalog_variant_compatible, command_catalog_restriction, command_pattern,
    load_command_catalog, profile_matches_any_model, split_at_arguments, CommandCatalogEntry,
    CommandParameterConstraint,
};
#[allow(unused_imports)]
pub(crate) use manual::{
    clean_manual_body, determine_effective_manual_mode, is_manual_path_syntax_safe,
    is_safe_manual_path, load_topic_path_map, manual_domain_bonus, manual_index,
    manual_search_terms, manual_workflow_bonus, model_restriction_notes, model_series,
    normalize_model_code, owner_applicability, pack_supports_profile, sanitize_preview,
    topic_applicability, topic_matches_pack_scope, topic_pack_scope, topic_variant_compatible,
    CachedPackValidation, ManualIndex, PackTopicScope, PackValidationCache, PackValidationReceipt,
    SearchTerms, TopicMeta, TopicOwnerMeta, KNOWLEDGE_PACK_V1, KNOWLEDGE_PACK_V2,
    KNOWLEDGE_PACK_V3,
};
#[allow(unused_imports)]
pub(crate) use pack::{
    file_stamp, load_validation_receipt, save_validation_receipt, validate_pack_cached,
    validate_pack_internal,
};

// Tauri commands must be `pub` (generate_handler! macro requirement).
#[allow(unused_imports)]
pub use authorization::authorize_at_commands;
#[allow(unused_imports)]
pub use capabilities::get_knowledge_capabilities;
#[allow(unused_imports)]
pub use manual::search_manuals;
#[allow(unused_imports)]
pub use pack::{validate_and_activate_knowledge_pack, validate_knowledge_pack};
