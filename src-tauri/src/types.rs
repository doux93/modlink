use modulink_core::knowledge::KnowledgeStore;
use parking_lot::RwLock;

pub struct Knowledge(pub RwLock<KnowledgeStore>);

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtCommandAuthorization {
    pub command: String,
    pub authorized: bool,
    pub applicability: Option<String>,
    pub manual: Option<String>,
    pub topic_id: Option<String>,
    pub pack_id: Option<String>,
    pub pack_version: Option<String>,
    pub reason_code: String,
    pub reason: String,
}

/// 手册内容级别：决定 AI 可以相信到什么程度。
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManualContentLevel {
    /// 无手册索引——AI 不得宣称有手册依据
    None,
    /// 只有摘要——涉及参数和完整步骤时须提示核对原手册
    Preview,
    /// 有全文正文——可以表述"根据手册正文"
    FullText,
}

/// 知识中心能力快照：前端和 AI 据此决定可用能力。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCapabilities {
    pub core_available: bool,
    pub manual_mode: ManualContentLevel,
    /// 当前画像实际可用的手册级别；可能因型号包不覆盖而低于包级 manual_mode。
    pub effective_manual_mode: ManualContentLevel,
    pub effective_profile_id: Option<String>,
    pub effective_degradation_reason: Option<String>,
    pub installed_packs: Vec<KnowledgePackInfo>,
    pub model_coverage: Vec<String>,
    /// 当前外部全文包明确覆盖的型号。None 表示 @1 全量包，Some 表示 @2 型号包。
    pub fulltext_model_coverage: Option<Vec<String>>,
    /// 公开索引主题数
    pub topic_count: usize,
    /// 降级原因（当 manual_mode 不是 fulltext 时说明原因）
    pub degradation_reason: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePackInfo {
    pub id: String,
    pub version: String,
    pub content_level: ManualContentLevel,
    pub models: Option<Vec<String>>,
}

/// modulink/knowledge-pack manifest。@3 preserves owner scope and unknown references.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePackManifest {
    pub schema: String,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub topic_count: usize,
    #[serde(default)]
    pub include_series: bool,
    #[serde(default)]
    pub include_generic: bool,
    #[serde(default)]
    pub include_unknown_reference: bool,
    #[serde(default)]
    pub topic_counts: Option<KnowledgePackTopicCounts>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePackTopicCounts {
    #[serde(default)]
    pub exact: usize,
    #[serde(default)]
    pub series: usize,
    #[serde(default)]
    pub generic: usize,
    #[serde(default)]
    pub unknown: usize,
}

/// 知识包验证结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePackValidation {
    pub valid: bool,
    pub manifest: Option<KnowledgePackManifest>,
    pub topic_paths_count: usize,
    pub sample_readable: usize,
    pub sample_total: usize,
    pub errors: Vec<String>,
}
