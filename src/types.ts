export interface PortInfo {
  name: string;
  description: string;
}

export interface OpenPortInfo {
  port: string;
  baud: number;
  generation: number;
}

export type ExecutionSource =
  | "console"
  | "quick_command"
  | "profile_detection"
  | "ai"
  | "diagnostic"
  | "stress"
  | "scenario_stress";

export type ExecutionRisk = "read_only" | "state_change" | "destructive" | "unknown";

export interface ExecutionTarget {
  port: string;
  sessionGeneration: number;
}

export interface ExecutionStep {
  cmd: string;
  timeoutMs?: number;
  delayMs?: number;
}

export interface ExecutionReport {
  executionId: string;
  target: ExecutionTarget;
  source: ExecutionSource;
  risk: ExecutionRisk;
  startedAt: number;
  elapsedMs: number;
  stoppedOnFailure: boolean;
  cancelled: boolean;
  results: AtResult[];
}

export type ExecutionStatus = "running" | "succeeded" | "failed" | "cancelled";

export type ExecutionKind =
  | "at_sequence"
  | "diagnostic"
  | "stress"
  | "scenario_stress";

export interface ExecutionSummary {
  planned: number;
  completed: number;
  passed: number;
  failed: number;
  cancelled: boolean;
  stoppedOnFailure: boolean;
  extra?: Record<string, unknown>;
}

export interface ExecutionLifecycleEvent {
  executionId: string;
  kind: ExecutionKind;
  target: ExecutionTarget;
  source: ExecutionSource;
  risk: ExecutionRisk;
  status: ExecutionStatus;
  stepCount: number;
  startedAt: number;
  elapsedMs: number;
  error?: string | null;
  summary?: ExecutionSummary | null;
  results?: AtResult[] | null;
  resultsTruncated?: boolean;
}

export type ManualContentLevel = "none" | "preview" | "fulltext";

export interface KnowledgePackInfo {
  id: string;
  version: string;
  contentLevel: ManualContentLevel;
  models: string[] | null;
}

export interface KnowledgeCapabilities {
  coreAvailable: boolean;
  /** 包级手册状态：描述已配置知识包是否可用，不代表当前画像一定命中。 */
  manualMode: ManualContentLevel;
  /** 当前设备画像的实际手册能力；旧后端未返回时前端回退到 manualMode。 */
  effectiveManualMode?: ManualContentLevel | null;
  /** 后端用于计算 effectiveManualMode 的画像 ID。 */
  effectiveProfileId?: string | null;
  /** 当前画像发生降级时的原因。 */
  effectiveDegradationReason?: string | null;
  installedPacks: KnowledgePackInfo[];
  modelCoverage: string[];
  /** null 表示 @1 全量包；数组表示型号包明确覆盖的精确型号。 */
  fulltextModelCoverage: string[] | null;
  topicCount: number;
  degradationReason: string | null;
}

export interface KnowledgePackManifest {
  schema: string;
  id: string;
  version: string;
  models: string[];
  generatedAt: string;
  topicCount: number;
  includeSeries: boolean;
  /** @1/@2 兼容字段；@3 使用 includeUnknownReference。 */
  includeGeneric?: boolean;
  includeUnknownReference?: boolean;
  topicCounts?: {
    exact: number;
    series: number;
    generic?: number;
    unknown?: number;
  };
}

export interface KnowledgePackValidation {
  valid: boolean;
  manifest: KnowledgePackManifest | null;
  topicPathsCount: number;
  sampleReadable: number;
  sampleTotal: number;
  errors: string[];
}

export interface Frame {
  generation: number;
  seq: number;
  ts: number;
  port: string;
  dir: "tx" | "rx";
  kind: "echo" | "response" | "final" | "urc" | "raw" | "tx";
  text: string;
  hex?: string;
}

export interface AtResult {
  request: string;
  lines: string[];
  final_code: string;
  elapsed_ms: number;
}

export interface QuickCommand {
  text: string;
  format: "text" | "hex";
  append_newline: boolean;
  delay_ms?: number | null;
  selected: boolean;
  note: string;
  legacy_slot?: number;
}

export interface QuickCommandGroup {
  name: string;
  items: QuickCommand[];
}

export interface QuickCmdsFile {
  schema: string;
  name: string;
  groups: QuickCommandGroup[];
  loop: { times: number; interval_ms: number };
  warnings: string[];
}

// ---------------- 知识层（M2） ----------------

export interface UrcPattern {
  id: string;
  pattern: string;
  label: string;
  severity?: string;
  severity_field?: string;
  severity_map: Record<string, string>;
  severity_default?: string;
}

export interface AtCommandDoc {
  cmd: string;
  syntax: string;
  category: string;
  risk: string;
  doc: string;
  examples: string[];
  title?: string;
  kind?: string;
  source?: string;
  models?: string[];
  sources?: string[];
  variants?: string[];
  topic_id?: string;
  map_id?: string;
  scope_source?: "explicit_table" | "map_name_fallback" | "profile_lookup" | "unknown";
  language?: string;
  content_hash?: string;
}

export interface AtDocsFile {
  schema: string;
  protocol: string;
  applies_to: string | string[];
  source: string;
  commands: AtCommandDoc[];
}

export interface ModuleProfile {
  schema: string;
  id: string;
  name: string;
  vendor: string;
  family: string;
  model_codes: string[];
  series: string;
  capabilities: string[];
  support_level: string;
  detect?: { command: string; match: string; priority: number };
  ports: { role: string; default_baud: number }[];
  line_ending: string;
  urc_patterns: UrcPattern[];
  at_commands: AtCommandDoc[];
  diagnostics: string[];
  error_code_tables: string[];
  intents: { phrases: string[]; sequence: string[] }[];
}

export interface ErrorCodeEntry {
  code: number;
  meaning: string;
  causes: string[];
  suggestions: string[];
  severity: string;
  related_at: string[];
}

export interface KnowledgeStore {
  profiles: ModuleProfile[];
  /** key = "cme:10" */
  error_codes: Record<string, ErrorCodeEntry>;
  profile_error_codes: Record<string, Record<string, ErrorCodeEntry>>;
  flows: DiagFlowFull[];
  at_docs: AtDocsFile[];
  warnings: string[];
}

// ---------------- 诊断（M3） ----------------

export interface DiagFlow {
  id: string;
  name: string;
  applies_to: string[];
  /** 兼容字段：推荐检查仍使用 checkup，产品入口统一按测试用例展示。 */
  kind: "checkup" | "usecase";
  category: string;
  source: string;
  /** 自动转换的用例未复核前为 false，UI 标注提示 */
  verified: boolean;
  execution_mode: "readonly" | "mutating" | "unknown";
  steps: { id: string; title: string }[];
}

export interface StepReport {
  id: string;
  title: string;
  result: "pass" | "warn" | "fail" | "skip";
  detail: string;
  reason: string;
  suggestion: string;
  raw: string[];
  collected: [string, string[]][];
  elapsed_ms: number;
}

export interface DiagReport {
  flow_id: string;
  flow_name: string;
  overall: "pass" | "warn" | "fail";
  aborted: boolean;
  cancelled: boolean;
  steps: StepReport[];
}

export interface DiagnosticStepEvent {
  run_id: string;
  port: string;
  flow_id: string;
  step: StepReport;
}

export interface StressSample {
  index: number;
  success: boolean;
  final_code: string;
  elapsed_ms: number;
  lines: string[];
  error: string;
}

export interface StressReport {
  command: string;
  planned: number;
  completed: number;
  passed: number;
  failed: number;
  cancelled: boolean;
  stopped_on_failure: boolean;
  elapsed_ms: number;
  min_latency_ms: number;
  max_latency_ms: number;
  avg_latency_ms: number;
  p95_latency_ms: number;
  failures: StressSample[];
  failures_truncated: boolean;
}

export interface StressProgressEvent {
  run_id: string;
  port: string;
  completed: number;
  passed: number;
  failed: number;
  sample: StressSample;
}

// ---------------- 场景压测 (L2/L3) ----------------

export interface ScenarioStressConfig {
  iterations: number;
  intervalMs: number;
  failurePolicy: "continue" | "stop" | "recover";
  recoveryCommands: string[] | null;
  recoveryTimeoutMs: number;
  warnAsPass: boolean;
  maxDurationSec: number;
}

export interface ScenarioStepSample {
  id: string;
  title: string;
  result: string;
  final_code: string;
  elapsed_ms: number;
  error_kind: string;
  reason: string;
}

export interface ScenarioRound {
  index: number;
  success: boolean;
  overall: string;
  elapsed_ms: number;
  steps: ScenarioStepSample[];
}

export interface FailureCluster {
  step_id: string;
  step_title: string;
  error_kind: string;
  count: number;
  sample_rounds: number[];
}

export interface StepLatencyStat {
  step_id: string;
  step_title: string;
  count: number;
  min_ms: number;
  max_ms: number;
  avg_ms: number;
  p95_ms: number;
}

export interface ScenarioStressReport {
  flow_id: string;
  flow_name: string;
  planned: number;
  completed: number;
  passed: number;
  failed: number;
  cancelled: boolean;
  stopped_on_failure: boolean;
  time_limit_reached: boolean;
  elapsed_ms: number;
  min_latency_ms: number;
  max_latency_ms: number;
  avg_latency_ms: number;
  p95_latency_ms: number;
  step_stats: StepLatencyStat[];
  failure_clusters: FailureCluster[];
  failures: ScenarioRound[];
  failures_truncated: boolean;
  recovery_attempts: number;
  recovery_failures: number;
}

export interface ScenarioRoundEvent {
  run_id: string;
  port: string;
  round: ScenarioRound;
  passed: number;
  failed: number;
}

// ---------------- 用例编辑器 ----------------

export interface UrcExpect {
  pattern: string;
  timeout_ms: number;
}

export interface EvalRule {
  when: string | null;
  default: boolean;
  result: "pass" | "warn" | "fail";
  detail: string | null;
  reason: string | null;
  suggestion: string | null;
  collect: string[];
  abort: boolean;
}

export interface DiagStepFull {
  id: string;
  title: string;
  send: string[];
  timeout_ms: number;
  parse: string | null;
  retry: { times: number; interval_ms: number; until: string } | null;
  optional: boolean;
  expect_urc: UrcExpect | null;
  evaluate: EvalRule[];
}

export interface DiagFlowFull {
  schema: string;
  id: string;
  name: string;
  applies_to: string[];
  kind: "checkup" | "usecase";
  category: string;
  source: string;
  verified: boolean;
  execution_mode: "readonly" | "mutating" | "unknown";
  steps: DiagStepFull[];
}

// ---------------- 日志分析 ----------------
export interface LogFrame {
  ts: number;
  dir: string;
  text: string;
}

export interface AtAnomaly {
  index: number;
  timestamp: number;
  request: string;
  kind: string;
  final_code: string;
  raw_lines: string[];
  error_code: ErrorCodeEntry | null;
}

export interface AtLogReport {
  total_interactions: number;
  error_interactions: number;
  anomalies: AtAnomaly[];
  summary: string;
}

export interface LogEvent {
  timestamp: number;
  module_id: string;
  module_name: string;
  level: string;
  severity: string;
  message: string;
  raw_line: string;
}

export interface RawLogReport {
  total_lines: number;
  matched_events: number;
  events: LogEvent[];
  summary: string;
}

export interface LogAnalysisReport {
  at: AtLogReport | null;
  raw: RawLogReport | null;
}
