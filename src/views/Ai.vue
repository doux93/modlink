<script setup lang="ts">
import {
  NButton,
  NInput,
  NInputNumber,
  NModal,
  NPopconfirm,
  useDialog,
  useMessage,
} from "naive-ui";
import { ref, computed, nextTick, onMounted, onActivated, onBeforeUnmount, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Settings } from "@lucide/vue";
import AppIcon from "../components/AppIcon.vue";
import AiToolCard from "../components/ai/AiToolCard.vue";
import { useWorkspace } from "../composables/useWorkspace";
import { useUiPrefs } from "../composables/useUiPrefs";
import { useSerialStore } from "../stores/serial";
import { useExecutionStore } from "../stores/execution";
import { applicableAtCommands } from "../knowledge";
import { usePendingTools } from "../composables/usePendingTools";
import { useAiRequestGate, AiRequestTimeoutError, isRequestCancelled, type AiRequestContext } from "../composables/useAiRequestGate";
import { deviceContextKey } from "../utils/deviceContext";
import type { AiExecutionTarget, UiItem } from "../types/ai";
import { splitAtArguments } from "../utils/atArgs";
import { requestAiChat } from "../utils/aiTransport";
import type {
  AtCommandDoc,
  DiagReport,
  ExecutionTarget,
  KnowledgeCapabilities,
  ManualContentLevel,
} from "../types";

const store = useSerialStore();
const execStore = useExecutionStore();
const message = useMessage();
const dialog = useDialog();
const { connectDevice } = useWorkspace();
const { activeView } = useUiPrefs();

// ---------------- 配置 ----------------
interface AiSettings {
  base_url: string;
  api_key: string;
  api_key_configured: boolean;
  model: string;
  recent_log_lines: number;
}
const settings = ref<AiSettings>({
  base_url: "",
  api_key: "",
  api_key_configured: false,
  model: "auto",
  recent_log_lines: 80,
});
const showSettings = ref(false);
// API key 默认隐藏，需要用户主动点"显示"才会明文呈现（防肩窥）
const revealApiKey = ref(false);
const capabilitiesLoading = ref(true);
let capabilitiesRequestVersion = 0;
const CAPABILITIES_TIMEOUT_MS = 5000;

onMounted(async () => {
  const capabilityLoad = refreshCapabilities();
  await store.init();
  const s = (await invoke<any>("get_settings")) ?? {};
  if (s.ai) settings.value = { ...settings.value, ...s.ai };
  await capabilityLoad;
});

onActivated(async () => {
  await refreshCapabilities();
  const pending = execStore.consumeAiPrompt();
  if (pending) {
    input.value = pending;
    ui.value.push({ kind: "note", text: "已载入执行分析内容，请确认后发送。" });
    await nextTick();
    await scrollBottom();
  }
});

async function saveSettings() {
  const ai: Record<string, unknown> = {
    base_url: settings.value.base_url,
    model: settings.value.model,
    recent_log_lines: settings.value.recent_log_lines,
  };
  if (settings.value.api_key.trim()) ai.api_key = settings.value.api_key.trim();
  await invoke("save_settings", {
    settings: { ai },
  });
  if (settings.value.api_key.trim()) settings.value.api_key_configured = true;
  settings.value.api_key = "";
  showSettings.value = false;
  message.success("AI 配置已保存");
  // 手册目录可能变动，立即刷新能力状态以同步 manualMode
  await refreshCapabilities();
}

async function clearApiKey() {
  await invoke("save_settings", { settings: { ai: { api_key: null } } });
  settings.value.api_key = "";
  settings.value.api_key_configured = false;
  message.success("API key 已从系统凭据中删除");
}

// ---------------- 会话模型 ----------------

const ui = ref<UiItem[]>([]);
const llmMessages = ref<any[]>([]); // OpenAI messages（不含 system，system 每次现算）
const input = ref("");
const foregroundOperations = ref(0);
let conversationVersion = 0;
const requestGate = useAiRequestGate(
  captureTarget,
  () => deviceContextKey(captureTarget(), store.activeProfileId),
  () => conversationVersion,
  () => ui.value.length,
  (request) => {
    llmMessages.value.splice(request.historyCheckpoint);
    ui.value.splice(request.uiCheckpoint);
  },
);
const { requestStage, activeRequestId, requestInFlight, requestStageLabel, beginRequest,
  assertRequestCurrent, setRequestStage, waitForRequest, finishRequest } = requestGate;
const busy = computed(() => requestInFlight.value || foregroundOperations.value > 0);
function cancelActiveRequest(showNote = true) {
  const hadRequest = requestInFlight.value;
  requestGate.cancelActiveRequest();
  if (showNote && hadRequest) {
    ui.value.push({ kind: "note", text: "已取消本次 AI 请求。" });
    void scrollBottom();
  }
}
onBeforeUnmount(() => cancelActiveRequest(false));

const capabilities = ref<KnowledgeCapabilities | null>(null);
const persistedCapabilityEvidence = ref("（当前没有持久化实测记录。）");

async function refreshCapabilities() {
  const requestVersion = ++capabilitiesRequestVersion;
  const requestedProfileId = store.activeProfileId || null;
  capabilitiesLoading.value = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const result = await Promise.race([
        invoke<KnowledgeCapabilities>("get_knowledge_capabilities", {
          profileId: requestedProfileId,
        }),
        new Promise<never>((_, reject) => {
          timer = setTimeout(
            () => reject(new Error("知识能力查询超时")),
            CAPABILITIES_TIMEOUT_MS
          );
        }),
      ]);
    if (requestVersion === capabilitiesRequestVersion) capabilities.value = result;
  } catch (error) {
    if (requestVersion === capabilitiesRequestVersion) {
      capabilities.value = null;
      console.warn("知识能力状态加载失败，AI 将按无手册模式运行", error);
    }
  } finally {
    if (timer) clearTimeout(timer);
    if (requestVersion === capabilitiesRequestVersion) {
      capabilitiesLoading.value = false;
    }
  }
}
watch(() => store.activeProfileId, () => void refreshCapabilities());
const ruleMode = ref(false);
const listRef = ref<HTMLElement>();

interface PendingToolBatch {
  question: string;
  manualEvidence: ManualSearchResult[];
}
const pendingTools = usePendingTools<PendingToolBatch>();
const { awaitingConfirmation } = pendingTools;

function captureTarget(): AiExecutionTarget | undefined {
  const session = store.activeSession;
  if (!session?.connected) return undefined;
  return {
    ...store.executionTarget(session.port),
    profileId: session.profileId || store.defaultProfileId,
  };
}

function targetAvailable(target?: ExecutionTarget): boolean {
  if (!target) return false;
  const session = store.sessions.find((item) => item.port === target.port);
  return Boolean(session?.connected && session.generation === target.sessionGeneration);
}

async function scrollBottom() {
  await nextTick();
  listRef.value?.scrollTo({ top: listRef.value.scrollHeight });
}

// ---------------- 上下文构造 ----------------
const HIGH_RISK = /CFUN\s*=\s*1\s*,\s*1|CFUN\s*=\s*0|\+MRST|\+CRST|FACTORY|FORMAT|QPRTPARA/i;

function recentLog(lines: number, filter?: string): string {
  let fs = store.activeFrames;
  if (filter) {
    try {
      const re = new RegExp(filter, "i");
      fs = fs.filter((f) => re.test(f.text));
    } catch {
      fs = fs.filter((f) => f.text.toLowerCase().includes(filter.toLowerCase()));
    }
  }
  return fs
    .slice(-lines)
    .map((f) => `${f.dir === "tx" ? ">>" : "<<"} ${f.hex ?? f.text}`)
    .join("\n");
}

/** 轻量检索：从画像 + 知识库批量文档里挑与问题/日志相关的指令文档 */
function retrieveDocs(question: string): string {
  const kb = [
    ...(store.activeProfile?.at_commands ?? []).map((command) => ({
      command,
      applicability: "profile" as const,
    })),
    ...applicableAtCommands(store.knowledge, store.activeProfile),
  ];
  const q = question.toUpperCase();
  const log = recentLog(40).toUpperCase();
  const inventoryIntent = /(?:哪些|所有|完整|清单|一览|汇总).*(?:指令|命令)|(?:指令|命令).*(?:哪些|所有|完整|清单|一览|汇总)/i.test(question);
  const protocol = ["HTTPS", "HTTP", "MQTT", "FTP", "FOTA", "TCP", "UDP", "SSL", "GNSS", "NTP"]
    .find((candidate) => q.includes(candidate));
  if (inventoryIntent && protocol) {
    const rank = (applicability: string) =>
      applicability === "profile" || applicability === "exact" ? 3 : applicability === "generic" ? 2 : 1;
    const commands = new Map<string, { command: AtCommandDoc; applicability: string }>();
    for (const item of kb) {
      if (item.applicability === "series") continue;
      const commandName = item.command.cmd.toUpperCase();
      const category = item.command.category?.toUpperCase() ?? "";
      if (!commandName.includes(protocol) && !category.includes(protocol)) continue;
      const existing = commands.get(commandName);
      if (!existing || rank(item.applicability) > rank(existing.applicability)) {
        commands.set(commandName, item);
      }
    }
    if (commands.size) {
      const CATALOG_LIMIT = 30;
      const sorted = [...commands.values()].sort((left, right) =>
        left.command.cmd.localeCompare(right.command.cmd)
      );
      const lines = sorted
        .slice(0, CATALOG_LIMIT)
        .map(({ command, applicability }) =>
          `- ${command.cmd} | 适用性: ${applicability === "profile" ? "exact" : applicability} | ${command.title ?? ""}`
        );
      // 证据块只放数据：怎么用这份目录由系统提示词的规则约束，别混进来。
      const header = sorted.length > CATALOG_LIMIT
        ? `### ${protocol} AT 命令目录（共 ${sorted.length} 条，按名称排序显示前 ${CATALOG_LIMIT} 条）`
        : `### ${protocol} AT 命令目录（共 ${sorted.length} 条，已完整列出）`;
      return `${header}\n${lines.join("\n")}`;
    }
  }
  const scored = kb
    .map(({ command: c, applicability }) => {
      let score = 0;
      const name = c.cmd.toUpperCase().replace(/^AT\+?/, "");
      if (name && q.includes(name)) score += 5;
      if (name && log.includes(name)) score += 3;
      for (const w of q.split(/[^A-Z0-9]+/)) {
        if (w.length >= 4 && (c.doc?.toUpperCase().includes(w) || c.title?.toUpperCase().includes(w))) {
          score += 1;
        }
      }
      if (score > 0) {
        score +=
          applicability === "profile" || applicability === "exact"
            ? 4
            : applicability === "series"
              ? 2
              : 0;
      }
      return { c, applicability, score };
    })
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, 3);
  if (!scored.length) return "";
  return scored
    .map(
      ({ c, applicability }) =>
        `### ${c.cmd} ${c.title ?? ""}\n适用性: ${applicability}\n语法: ${c.syntax}\n${(c.doc ?? "").slice(0, 800)}`
    )
    .join("\n\n");
}

function activeManualMode(): ManualContentLevel {
  if (capabilities.value?.effectiveManualMode) {
    return capabilities.value.effectiveManualMode;
  }
  const mode = capabilities.value?.manualMode ?? "none";
  if (mode !== "fulltext") return mode;
  const coverage = capabilities.value?.fulltextModelCoverage;
  // null/undefined 表示兼容 @1 全量包；@2 必须精确命中声明型号。
  if (coverage == null) return "fulltext";
  const activeModels = store.activeProfile?.model_codes ?? [];
  return activeModels.some((model) =>
    coverage.some((covered) => covered.toUpperCase() === model.toUpperCase())
  )
    ? "fulltext"
    : "preview";
}

interface ManualSearchResult {
  title?: string;
  manual?: string;
  models?: string[];
  applicability?: "exact" | "series" | "generic" | "unknown";
  content?: string;
  contentLevel?: ManualContentLevel;
  packId?: string;
  packVersion?: string;
  error?: string;
  modelRestrictions?: string[];
}

interface AtCommandAuthorization {
  command: string;
  authorized: boolean;
  applicability?: "exact" | "generic" | "standard" | "runtime";
  manual?: string;
  topicId?: string;
  packId?: string;
  packVersion?: string;
  reasonCode: string;
  reason: string;
}

function evidenceScopeLabel(result: ManualSearchResult): string {
  if (result.applicability === "exact") return "exact（当前精确型号资料）";
  if (result.applicability === "series") return "series（同系列参考资料）";
  if (result.applicability === "unknown") return "unknown（参考资料，不可用于执行授权）";
  if (result.applicability === "generic") {
    return result.contentLevel === "fulltext" && result.packId !== "cmiot.core.builtin"
      ? "generic（当前型号包明确包含的通用资料）"
      : "generic（通用资料）";
  }
  return "unknown（参考资料，不可用于执行授权）";
}

const MANUAL_INTENT = /\b(?:AT|HTTP|HTTPS|MQTT|FTP|FOTA|TCP|UDP|SSL|GNSS|NTP)\b|指令|命令|配置|设置|使用|用法|流程|步骤|参数|手册|怎么|如何/i;

async function prefetchManualEvidence(
  question: string,
  request: AiRequestContext
): Promise<ManualSearchResult[]> {
  if (activeManualMode() === "none" || !MANUAL_INTENT.test(question)) return [];
  setRequestStage(request, "searching");
  try {
    const results = await waitForRequest(
      invoke<ManualSearchResult[]>("search_manuals", {
        query: question,
        topK: 4,
        profileId: store.activeProfileId || null,
      }),
      request
    );
    return results.filter((result) => !result.error);
  } catch (error) {
    if (isRequestCancelled(error) || error instanceof AiRequestTimeoutError) throw error;
    return [];
  }
}

function levenshtein(a: string, b: string): number {
  const previous = Array.from({ length: b.length + 1 }, (_, index) => index);
  for (let i = 1; i <= a.length; i++) {
    let diagonal = previous[0];
    previous[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const above = previous[j];
      previous[j] = Math.min(
        previous[j] + 1,
        previous[j - 1] + 1,
        diagonal + (a[i - 1] === b[j - 1] ? 0 : 1)
      );
      diagonal = above;
    }
  }
  return previous[b.length];
}

function modelCorrectionContext(question: string): string {
  const known = capabilities.value?.modelCoverage ?? [];
  const knownSet = new Set(known.map((model) => model.toUpperCase()));
  const mentioned = question.toUpperCase().match(/\b(?:ML|MN|MR)\d{3,4}(?:-[A-Z0-9]+|[A-Z])?\b/g) ?? [];
  const corrections = [...new Set(mentioned)]
    .filter((model) => !knownSet.has(model))
    .map((model) => {
      const nearest = known
        .map((candidate) => ({ candidate, distance: levenshtein(model, candidate.toUpperCase()) }))
        .sort((left, right) => left.distance - right.distance)[0];
      return nearest && nearest.distance <= 2
        ? `用户提到的 ${model} 不在型号目录中，最接近 ${nearest.candidate}；必须先纠正并请用户确认。`
        : `用户提到的 ${model} 不在型号目录中；必须明确提示无法识别，不能假设该型号存在。`;
    });
  return corrections.join("\n");
}

function manualEvidenceContext(evidence: ManualSearchResult[]): string {
  if (!evidence.length) return "（自动检索未命中，不得据此编造完整流程或参数。）";
  return evidence
    .map((result, index) =>
      `### [${index + 1}] ${result.title || "未命名章节"}\n` +
      `手册: ${result.manual || "-"} | 适用性: ${evidenceScopeLabel(result)} | 内容级别: ${result.contentLevel || "-"} | 知识包: ${result.packId || "-"}@${result.packVersion || "-"}\n` +
      `${result.modelRestrictions?.length ? `当前型号限制（必须优先遵守）:\n${result.modelRestrictions.map((note) => `- ${note}`).join("\n")}\n` : ""}` +
      `${(result.content || "").slice(0, 4000)}`
    )
    .join("\n\n");
}

function runtimeCapabilityContext(): string {
  const latest = new Map<string, { query: string; responses: string[] }>();
  let current: { name: string; query: string; responses: string[] } | null = null;
  for (const frame of store.activeFrames) {
    const text = frame.text.trim();
    if (frame.dir === "tx") {
      const match = text.match(/^(AT(?:\+[A-Z0-9]+|[A-Z0-9]*))=\?$/i);
      current = match ? { name: match[1].toUpperCase(), query: text, responses: [] } : null;
      if (current) latest.set(current.name, current);
    } else if (current && text && text.toUpperCase() !== current.query.toUpperCase()) {
      current.responses.push(text);
    }
  }
  if (!latest.size) return "（当前会话没有参数能力查询记录。）";
  return [...latest.values()]
    .slice(-10)
    .map((probe) => `>> ${probe.query}\n${probe.responses.slice(0, 16).map((line) => `<< ${line}`).join("\n")}`)
    .join("\n\n");
}

function systemPrompt(question: string, manualEvidence: ManualSearchResult[]): string {
  const profile = store.activeProfile;
  const conn = store.activeSession?.connected
    ? `已连接 ${store.activePort} @ ${store.activeSession.baud}`
    : "串口未连接";
  const docs = retrieveDocs(question);
  const mode = activeManualMode();
  const modelCorrection = modelCorrectionContext(question);

  const manualRule = mode === "fulltext"
    ? "6. 涉及功能配置流程、指令详细用法、专题功能（5G/蓝牙/HTTP/FOTA 等）时，先调 search_manuals 检索官方手册再回答，可以表述“根据手册正文”，并注明出处手册名和章节。"
    : mode === "preview"
      ? "6. 涉及功能配置流程、指令详细用法时，可调 search_manuals 检索内置摘要。但摘要可能不完整，涉及具体参数和完整步骤时必须提示用户核对原手册，不得声称已根据手册全文。"
      : "6. 当前未安装手册知识包，不得调用 search_manuals，也不得宣称有任何手册依据。涉及手册内容时建议用户查阅官方文档。";

  const modeLabel = mode === "fulltext" ? "全文手册" : mode === "preview" ? "摘要手册" : "无手册";
  const coreStatus = capabilities.value?.coreAvailable ? "已就绪" : "未就绪";

  return `你是蜂窝通信模组 AT 调试专家助手，嵌入在 ModuLink Studio 调试工具中，帮助用户（嵌入式工程师/FAE/测试）排查模组问题。

规则：
1. 需要执行 AT 指令时，必须调用 propose_at_sequence 工具提议，绝不让用户手敲，也绝不假设已执行；工具会经用户确认后执行并把结果返回给你。
2. 重启/擦除/恢复出厂类指令必须在 steps 里标 risk 字段（reboot/erase）。
3. 不得虚构 AT 指令；不确定的指令先说明依据或建议查询手册。
4. 回答用中文，简洁、结论先行；错误码优先调 lookup_error_code。
5. 需要更多日志时调 get_recent_log；怀疑入网问题时可提议 run_diagnostic 跑"入网连通性推荐检查"。
${manualRule}
7. 证据适用性必须准确表达：exact 是当前精确型号资料；series 只代表同系列参考，不得声称已在当前型号验证；unknown 是适用范围未知的参考资料，不得用于 AT 执行授权；generic 若来自当前已验证型号包的全文，是该型号包明确携带的通用适用资料，可以据此回答和提议指令，但不得描述成当前型号专属或实测资料。禁止把这种 generic 全文误报为“知识库缺少当前型号依据”。
8. AI 提议 AT 指令前，应结合内置 AT 卡片校验指令格式和风险，不能只凭手册摘要生成。上下文若给出"AT 命令目录"，它就是当前型号的权威命令清单：清单类问题以它为准，不得把目录中已有的命令说成未检索到，也不得跨命令族混用。
9. 下方“自动检索手册证据”是本轮问题的优先依据。涉及完整流程时必须按证据回答；未命中时明确说明，禁止用常识补齐指令。某次 AT 提议被策略拒绝，只表示所提议的具体命令缺少依据，不代表整个功能没有手册；此时应重新检索协议专用手册并改用正文中实际出现的命令，禁止宣称该功能“无手册资料”。
10. 型号目录校验提示必须执行：未知或疑似拼错的型号先纠正并确认，不能静默接受。
11. “命令存在”不等于“该型号支持该命令的全部参数值”。当前型号限制说明必须高于通用参数表；串口 AT+命令=? 的当前模组实测范围优先级最高。若实测只返回 0，不得建议值 1/2，也不得据参数名推断功能受支持。

## 当前环境
模组画像: ${profile?.name ?? "未选择"} (model: ${profile?.model_codes?.join(", ") || "-"}; series: ${profile?.series || "-"}; support: ${profile?.support_level || "-"})
连接状态: ${conn}
知识能力: 核心包${coreStatus} | 手册: ${modeLabel}
${modelCorrection ? `型号目录校验:\n${modelCorrection}` : ""}
${docs ? `\n## 相关手册摘录\n${docs}` : ""}

## 自动检索手册证据
${manualEvidenceContext(manualEvidence)}

## 当前模组参数能力实测（优先级高于手册通用范围）
${persistedCapabilityEvidence.value}

## 当前会话参数查询（尚未持久化的即时证据）
${runtimeCapabilityContext()}`;
}

const TOOLS = [
  {
    type: "function",
    function: {
      name: "propose_at_sequence",
      description: "提议一组 AT 指令序列，由用户确认后执行，执行结果会返回给你",
      parameters: {
        type: "object",
        properties: {
          purpose: { type: "string", description: "这组指令的目的（给用户看）" },
          steps: {
            type: "array",
            items: {
              type: "object",
              properties: {
                cmd: { type: "string" },
                expect: { type: "string", description: "期望响应的正则，可选" },
                delay_ms: { type: "number" },
                risk: { type: "string", enum: ["none", "reboot", "erase", "config"] },
              },
              required: ["cmd"],
            },
          },
        },
        required: ["purpose", "steps"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "lookup_error_code",
      description: "查询本地错误码知识库（+CME/+CMS ERROR 码的含义、原因、建议）",
      parameters: {
        type: "object",
        properties: {
          code_type: { type: "string", enum: ["cme", "cms"] },
          code: { type: "number" },
        },
        required: ["code_type", "code"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "get_recent_log",
      description: "获取最近的串口日志（可选正则过滤），用于你主动翻日志分析",
      parameters: {
        type: "object",
        properties: {
          lines: { type: "number", description: "行数，默认 80，最大 300" },
          filter: { type: "string", description: "正则过滤，可选" },
        },
      },
    },
  },
  {
    type: "function",
    function: {
      name: "run_diagnostic",
      description: "运行推荐检查或测试用例（会经用户确认），返回结构化结果",
      parameters: {
        type: "object",
        properties: { flow_id: { type: "string", description: "默认 network_attach" } },
      },
    },
  },
  {
    type: "function",
    function: {
      name: "search_manuals",
      description: "DYNAMIC_MANUAL_DESC",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "检索关键词，如 '5G LAN 配置' 或 'MHTTPCFG'" },
          top_k: { type: "number", description: "返回条数，默认 3" },
        },
        required: ["query"],
      },
    },
  },
];

// ---------------- LLM 调用与工具编排 ----------------
async function callLLM(
  question: string,
  manualEvidence: ManualSearchResult[],
  request: AiRequestContext
): Promise<any> {
  if (requestStage.value !== "resuming") setRequestStage(request, "preparing");
  try {
    persistedCapabilityEvidence.value = await waitForRequest(
      invoke<string>("get_capability_evidence_context", {
        profileId: store.activeProfileId || null,
        port: store.activePort || null,
      }),
      request
    );
  } catch (error) {
    if (isRequestCancelled(error) || error instanceof AiRequestTimeoutError) throw error;
    assertRequestCurrent(request);
    persistedCapabilityEvidence.value = "（持久化实测能力读取失败。）";
  }
  const mode = activeManualMode();
  let tools = mode === "none"
    ? TOOLS.filter((t) => t.function.name !== "search_manuals")
    : TOOLS.map((t) => {
        if (t.function.name === "search_manuals") {
          return {
            ...t,
            function: {
              ...t.function,
              description:
                mode === "fulltext"
                  ? "在本地手册知识库中全文检索。当问题涉及功能配置流程、指令详细用法、5G/蓝牙/HTTP 等专题时使用"
                  : "在内置手册摘要知识库中检索。返回的是摘要而非全文，涉及具体参数时需提示用户核对原手册",
            },
          };
        }
        return t;
      });
  const body = {
    messages: [{ role: "system", content: systemPrompt(question, manualEvidence) }, ...llmMessages.value],
    tools: tools,
    temperature: 0.2,
  };
  setRequestStage(request, "waiting");
  const resp = await waitForRequest(
    requestAiChat<any>(
      { base_url: settings.value.base_url, model: settings.value.model },
      body,
      { signal: request.abortController.signal, deadline: request.deadline },
    ),
    request
  );
  return resp.choices?.[0]?.message;
}

function localRisk(cmd: string): string | undefined {
  if (HIGH_RISK.test(cmd)) return "reboot";
  const body = cmd.trim().toUpperCase().split(/[=?]/)[0];
  const docs = [
    ...(store.activeProfile?.at_commands ?? []),
    ...applicableAtCommands(store.knowledge, store.activeProfile).map((item) => item.command),
  ];
  const risk = docs.find((doc) => doc.cmd.toUpperCase() === body)?.risk;
  return risk && risk !== "none" ? risk : undefined;
}

async function authorizeCommands(commands: string[], profileId?: string): Promise<AtCommandAuthorization[]> {
  const decisions = await invoke<AtCommandAuthorization[]>("authorize_at_commands", {
    commands,
    profileId: profileId || store.activeProfileId || null,
    port: store.activePort || null,
  });
  return decisions.map((decision, index) => {
    if (!decision.authorized) return decision;
    const denial = runtimeProbeDenial(commands[index] ?? "");
    return denial
      ? { ...decision, authorized: false, reasonCode: "runtime_capability_restricted", reason: denial }
      : decision;
  });
}

function runtimeProbeDenial(command: string): string | undefined {
  const name = command.trim().toUpperCase().match(/^(AT(?:\+[A-Z0-9]+|[A-Z0-9]*))/)?.[1];
  if (!name || command.trim().endsWith("=?")) return undefined;
  let response: string[] = [];
  let collecting = false;
  for (const frame of store.activeFrames) {
    const text = frame.text.trim();
    if (frame.dir === "tx") {
      collecting = text.toUpperCase() === `${name}=?`;
      if (collecting) response = [];
    } else if (collecting && text.toUpperCase() !== `${name}=?`) {
      response.push(text);
    }
  }
  if (!response.length) return undefined;
  const args = splitAtArguments(command);
  for (let index = 0; index + 1 < args.length; index++) {
    const selector = args[index].replace(/^"|"$/g, "");
    if (!/^[a-z_][a-z0-9_]*$/i.test(selector)) continue;
    const proposed = Number(args[index + 1]);
    if (!Number.isInteger(proposed)) continue;
    const line = response.find((item) =>
      new RegExp(`["']${selector}["']`, "i").test(item)
    );
    if (!line) continue;
    const tail = line.slice(line.search(new RegExp(`["']${selector}["']`, "i")) + selector.length + 2);
    const range = tail.match(/\(?\s*(-?\d+)\s*(?:-\s*(-?\d+))?\s*\)?/);
    if (!range) continue;
    const minimum = Number(range[1]);
    const maximum = range[2] == null ? minimum : Number(range[2]);
    if (proposed < minimum || proposed > maximum) {
      return `当前模组 ${name}=? 实测显示参数 ${selector} 仅支持 ${minimum === maximum ? minimum : `${minimum}-${maximum}`}，已拒绝值 ${proposed}`;
    }
  }
  return undefined;
}

/** 主循环：直到 assistant 给出纯文本答复或需要用户确认 */
async function agentLoop(
  question: string,
  request: AiRequestContext,
  initialEvidence?: ManualSearchResult[]
) {
  assertRequestCurrent(request);
  const manualEvidence = initialEvidence ?? await prefetchManualEvidence(question, request);
  assertRequestCurrent(request);
  if (!initialEvidence && manualEvidence.length) {
    ui.value.push({
      kind: "note",
      text: `已自动检索当前型号手册：${manualEvidence.map((item) => item.title).filter(Boolean).join("、")}`,
    });
  }
  for (let round = 0; round < 8; round++) {
    const msg = await callLLM(question, manualEvidence, request);
    assertRequestCurrent(request);
    if (!msg) throw new Error("空响应");
    llmMessages.value.push(msg);

    if (msg.content && !msg.tool_calls?.length) {
      ui.value.push({ kind: "assistant", text: msg.content });
      await scrollBottom();
      assertRequestCurrent(request);
      return;
    }

    const interactiveIds: string[] = [];
    for (const tc of msg.tool_calls ?? []) {
      let args: any;
      try {
        args = JSON.parse(tc.function.arguments || "{}");
      } catch (error) {
        llmMessages.value.push({
          role: "tool",
          tool_call_id: tc.id,
          content: JSON.stringify({ error: `工具参数不是合法 JSON: ${error}` }),
        });
        continue;
      }
      const name = tc.function.name;

      if (name === "propose_at_sequence") {
        const proposedSteps = Array.isArray(args.steps) ? args.steps : [];
        const authorizations = proposedSteps.length
          ? await waitForRequest(
              authorizeCommands(proposedSteps.map((step: any) => String(step?.cmd ?? ""))),
              request
            )
          : [];
        assertRequestCurrent(request);
        const unauthorized = authorizations.filter((item) => !item.authorized);
        if (!proposedSteps.length || unauthorized.length) {
          const reason = !proposedSteps.length
            ? "指令序列为空"
            : unauthorized.map((item) => `${item.command || "无效指令"}（${item.reason}）`).join("；");
          llmMessages.value.push({
            role: "tool",
            tool_call_id: tc.id,
            content: JSON.stringify({
              error: reason,
              policy: "精确型号资料、正文明确点名当前型号的全文资料，或当前已验证型号包明确包含的通用全文资料，才能授权执行；同系列资料仅供参考。",
            }),
          });
          ui.value.push({ kind: "note", text: `已阻止无当前型号包依据的 AT 执行提议：${reason}` });
          continue;
        }
        ui.value.push({
          kind: "card",
          card: {
            toolCallId: tc.id,
            purpose: args.purpose ?? "",
            steps: proposedSteps.map((s: any) => ({
              ...s,
              risk: s.risk && s.risk !== "none" ? s.risk : localRisk(s.cmd),
              checked: true,
            })),
            status: "pending",
            results: [],
            source: "llm",
            target: request.target,
          },
        });
        interactiveIds.push(tc.id);
        continue;
      }

      let result: any;
      if (name === "lookup_error_code") {
        result =
          store.activeErrorCodes[`${args.code_type}:${args.code}`] ??
          { error: "知识库中无此错误码" };
      } else if (name === "search_manuals") {
        setRequestStage(request, "searching");
        result = await waitForRequest(
          invoke("search_manuals", {
            query: args.query ?? "",
            topK: args.top_k ?? 3,
            profileId: store.activeProfileId || null,
          }),
          request
        );
        assertRequestCurrent(request);
        if (Array.isArray(result)) {
          for (const item of result as ManualSearchResult[]) {
            if (!item.error && !manualEvidence.some((existing) =>
              existing.title === item.title && existing.manual === item.manual
            )) {
              manualEvidence.push(item);
            }
          }
        }
        ui.value.push({ kind: "note", text: `AI 检索了手册知识库：${args.query}` });
      } else if (name === "get_recent_log") {
        result = recentLog(Math.min(args.lines ?? 80, 300), args.filter) || "(无日志)";
        ui.value.push({ kind: "note", text: `AI 查看了最近日志${args.filter ? `（过滤: ${args.filter}）` : ""}` });
      } else if (name === "run_diagnostic") {
        ui.value.push({
          kind: "diagnostic",
          diagnostic: {
            toolCallId: tc.id,
            flowId: args.flow_id ?? "network_attach",
            purpose: "AI 请求运行推荐检查",
            status: "pending",
            result: "",
            details: [],
            target: request.target,
          },
        });
        interactiveIds.push(tc.id);
        continue;
      } else {
        result = { error: "未知工具" };
      }
      llmMessages.value.push({
        role: "tool",
        tool_call_id: tc.id,
        content: JSON.stringify(result).slice(0, 6000),
      });
    }
    await scrollBottom();
    assertRequestCurrent(request);
    if (interactiveIds.length) {
      pendingTools.begin({ question, manualEvidence }, interactiveIds);
      return;
    }
    if (!msg.tool_calls?.length && !msg.content) throw new Error("模型返回了空消息");
  }
  ui.value.push({ kind: "note", text: "对话轮次过多，已停止" });
}

async function completeInteractiveTool(toolCallId: string) {
  const batch = pendingTools.complete(toolCallId);
  if (!batch) return;
  const request = beginRequest("resuming", llmMessages.value.length);
  try {
    await agentLoop(batch.question, request, batch.manualEvidence);
  } catch (error) {
    if (!isRequestCancelled(error)) fallbackNote(error);
  } finally {
    finishRequest(request);
  }
}

/** 用户确认执行 AT 序列卡片 */
async function executeCard(item: UiItem) {
  const version = conversationVersion;
  const card = item.card!;
  if (card.status !== "pending" || busy.value || !targetAvailable(card.target)) return;
  card.status = "running";
  foregroundOperations.value += 1;
  const results: any[] = [];
  try {
    if (!card.target) throw new Error("卡片未绑定设备会话，请重新生成");
    const selected = card.steps.filter((step) => step.checked);
    if (!selected.length) throw new Error("请至少选择一条 AT 指令");
    const report = await store.executeAtSequence(
      card.target,
      selected.map((step) => ({
        cmd: step.cmd,
        timeoutMs: 8000,
        delayMs: step.delay_ms ?? 0,
      })),
      "ai",
      "continue",
      card.target.profileId
    );
    const responses = report.results;
    for (let index = 0; index < responses.length; index++) {
      const step = selected[index];
      const response = responses[index];
      const ok = response.final_code === "OK" && (!step.expect || new RegExp(step.expect).test(response.lines.join("\n")));
      results.push({
        cmd: step.cmd,
        final: response.final_code,
        lines: response.lines,
        expect_matched: ok,
      });
      card.results.push(
        `${step.cmd} → ${response.final_code}${response.lines.length ? " | " + response.lines.join(" ") : ""}`
      );
    }
    card.status = report.cancelled ? "cancelled"
      : !report.stoppedOnFailure && results.length === selected.length && results.every((result) => result.expect_matched)
        ? "done" : "failed";
    if (version !== conversationVersion) return;
    if (card.source === "rule") {
      ui.value.push({ kind: "assistant", text: `【规则引擎】执行完成\n${card.results.join("\n")}` });
    } else {
      llmMessages.value.push({
        role: "tool",
        tool_call_id: card.toolCallId,
        content: JSON.stringify(results).slice(0, 6000),
      });
      await completeInteractiveTool(card.toolCallId);
    }
  } catch (e) {
    card.status = "failed";
    card.results.push(`序列执行失败: ${e}`);
    if (card.source === "llm" && version === conversationVersion) {
      llmMessages.value.push({
        role: "tool",
        tool_call_id: card.toolCallId,
        content: JSON.stringify({ error: String(e), partial_results: results }).slice(0, 6000),
      });
      try {
        await completeInteractiveTool(card.toolCallId);
      } catch (resumeError) {
        fallbackNote(resumeError);
      }
    }
  } finally {
    foregroundOperations.value = Math.max(0, foregroundOperations.value - 1);
  }
}

async function cancelCard(item: UiItem) {
  const card = item.card!;
  if (card.status !== "pending" || busy.value) return;
  card.status = "cancelled";
  if (card.source === "rule") {
    ui.value.push({ kind: "note", text: "已取消规则指令执行" });
    return;
  }
  llmMessages.value.push({
    role: "tool",
    tool_call_id: card.toolCallId,
    content: JSON.stringify({ declined: true, note: "用户取消了执行" }),
  });
  foregroundOperations.value += 1;
  try {
    await completeInteractiveTool(card.toolCallId);
  } catch (e) {
    fallbackNote(e);
  } finally {
    foregroundOperations.value = Math.max(0, foregroundOperations.value - 1);
  }
}

function confirmUnverifiedDiagnostic(flowName: string): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false;
    const done = (value: boolean) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    dialog.warning({
      title: "该流程未经实机复核",
      content: `“${flowName}”由手册示例自动转换，参数可能需要按现场环境调整。继续后仍会经过当前型号和实测能力限制校验。`,
      positiveText: "确认并运行",
      negativeText: "取消",
      onPositiveClick: () => done(true),
      onNegativeClick: () => done(false),
      onMaskClick: () => done(false),
      onClose: () => done(false),
    });
  });
}

/** 用户确认运行诊断流程 */
async function executeDiagnostic(item: UiItem) {
  const version = conversationVersion;
  const diag = item.diagnostic!;
  if (diag.status !== "pending" || busy.value || !targetAvailable(diag.target)) return;
  if (!diag.target) {
    message.error("卡片未绑定设备会话，请重新生成");
    return;
  }
  const flow = store.knowledge?.flows.find((candidate) => candidate.id === diag.flowId);
  let allowUnverified = false;
  if (flow && !flow.verified) {
    allowUnverified = await confirmUnverifiedDiagnostic(flow.name);
    if (!allowUnverified || version !== conversationVersion || diag.status !== "pending" || busy.value || !targetAvailable(diag.target)) return;
  }
  diag.status = "running";
  foregroundOperations.value += 1;
  let result: any;
  try {
    const runId = crypto.randomUUID();
    const report = await invoke<DiagReport>("run_diagnostic", {
      port: diag.target.port,
      sessionGeneration: diag.target.sessionGeneration,
      flowId: diag.flowId,
      runId,
      profileId: diag.target.profileId || null,
      allowUnverified,
    });
    result = report.steps.map((s) => ({
      step: s.title,
      result: s.result,
      reason: s.reason,
      detail: s.detail,
    }));
    diag.status = report.overall === "pass" ? "done" : "failed";
    diag.result = report.overall;
    diag.details = report.steps.map((s) => `${s.title}: ${s.result}${s.reason ? ` - ${s.reason}` : ""}`);
  } catch (e) {
    result = { error: String(e) };
    diag.status = "failed";
    diag.result = "error";
    diag.details = [String(e)];
  }
  if (version !== conversationVersion) {
    foregroundOperations.value = Math.max(0, foregroundOperations.value - 1);
    return;
  }
  llmMessages.value.push({
    role: "tool",
    tool_call_id: diag.toolCallId,
    content: JSON.stringify(result).slice(0, 6000),
  });
  try {
    await completeInteractiveTool(diag.toolCallId);
  } catch (e) {
    fallbackNote(e);
  } finally {
    foregroundOperations.value = Math.max(0, foregroundOperations.value - 1);
  }
}

async function cancelDiagnostic(item: UiItem) {
  const diag = item.diagnostic!;
  if (diag.status !== "pending" || busy.value) return;
  diag.status = "cancelled";
  llmMessages.value.push({
    role: "tool",
    tool_call_id: diag.toolCallId,
    content: JSON.stringify({ declined: true, note: "用户取消了诊断流程" }),
  });
  foregroundOperations.value += 1;
  try {
    await completeInteractiveTool(diag.toolCallId);
  } catch (e) {
    fallbackNote(e);
  } finally {
    foregroundOperations.value = Math.max(0, foregroundOperations.value - 1);
  }
}

// ---------------- 规则引擎降级 ----------------
function ruleAnswer(question: string): boolean {
  const q = question.trim();
  // 1. 意图短语匹配 → 出确认卡
  for (const it of store.activeProfile?.intents ?? []) {
    if (it.phrases.some((p) => q.includes(p))) {
      ui.value.push({
        kind: "card",
        card: {
          toolCallId: "",
          purpose: `规则匹配：${it.phrases[0]}`,
          steps: it.sequence.map((cmd) => ({ cmd, checked: true })),
          status: "pending",
          results: [],
          source: "rule",
          target: captureTarget(),
        },
      });
      return true;
    }
  }
  // 2. 错误码提取 → 查知识库
  const m = /(CME|CMS)\s*ERROR:?\s*(\d+)|错误码\s*(\d+)/i.exec(q);
  if (m && store.knowledge) {
    const code = m[2] ?? m[3];
    const type = (m[1] ?? "cme").toLowerCase();
    const e = store.activeErrorCodes[`${type}:${code}`];
    if (e) {
      ui.value.push({
        kind: "assistant",
        text: `【规则引擎】+${type.toUpperCase()} ERROR: ${code} — ${e.meaning}\n\n可能原因：\n${e.causes.map((c) => "· " + c).join("\n")}\n\n建议：\n${e.suggestions.map((s) => "· " + s).join("\n")}`,
      });
      return true;
    }
  }
  return false;
}

function fallbackNote(e: unknown) {
  if (isRequestCancelled(e)) return;
  ruleMode.value = true;
  ui.value.push({
    kind: "note",
    text: `AI 服务不可用（${String(e).slice(0, 120)}）— 已切换规则引擎模式：支持错误码解读和常用指令意图（如"查信号"）`,
  });
}

// ---------------- 发送 ----------------
async function send(preset?: string) {
  const q = (preset ?? input.value).trim();
  if (!q || busy.value || awaitingConfirmation.value) return;
  if (!capabilities.value) {
    message.warning("知识能力状态暂不可用，已按无手册模式继续");
  }
  const historyCheckpoint = llmMessages.value.length;
  input.value = "";
  ui.value.push({ kind: "user", text: q });
  llmMessages.value.push({
    role: "user",
    content: `${q}\n\n[自动附带·最近串口日志]\n${recentLog(settings.value.recent_log_lines) || "(暂无)"}`,
  });
  if (ruleMode.value) {
    await scrollBottom();
    if (!ruleAnswer(q)) {
      ui.value.push({ kind: "note", text: "规则模式下暂不支持此问题；恢复网络后点右上角「重连 AI」" });
    }
    return;
  }
  const request = beginRequest("preparing", historyCheckpoint);
  try {
    await scrollBottom();
    assertRequestCurrent(request);
    await agentLoop(q, request);
  } catch (e) {
    if (!isRequestCancelled(e)) {
      fallbackNote(e);
      ruleAnswer(q);
    }
  } finally {
    finishRequest(request);
  }
}

function retryOnline() {
  ruleMode.value = false;
  ui.value.push({ kind: "note", text: "已切回 AI 模式" });
}

function clearChat() {
  conversationVersion += 1;
  cancelActiveRequest(false);
  pendingTools.clear();
  input.value = "";
  foregroundOperations.value = 0;
  requestStage.value = "idle";
  activeRequestId.value = null;
  ui.value = [];
  llmMessages.value = [];
}

watch(
  () => deviceContextKey(captureTarget(), store.activeProfileId),
  () => {
    conversationVersion += 1;
    cancelActiveRequest(false);
    pendingTools.clear();
    for (const item of ui.value) {
      if (item.card?.status === "pending") item.card.status = "cancelled";
      if (item.diagnostic?.status === "pending") item.diagnostic.status = "cancelled";
    }
    llmMessages.value = [];
    if (ui.value.length) ui.value.push({ kind: "note", text: "设备上下文已变化，已开始新的设备对话；旧记录保留供查看。" });
  },
  { flush: "sync" },
);

const QUICK_ASKS = [
  "分析一下本次会话的日志，哪里有问题？",
  "为什么连不上网？帮我排查",
  "帮我测试信号质量和网络注册状态",
];

const connected = computed(() => store.activeSession?.connected ?? false);
const attachedLogCount = computed(() => Math.min(store.activeFrames.length, settings.value.recent_log_lines));
const manualLabel = computed(() => capabilitiesLoading.value ? "正在检查资料" : activeManualMode() === "fulltext" ? "可检索全文手册" : activeManualMode() === "preview" ? "仅有摘要参考" : "暂无可用手册");
</script>

<template>
  <div class="ai">
    <header class="ai-head">
      <div>
        <h2>AI 调试助手</h2>
        <p class="sub">
          结合设备证据分析问题 ·
          <b :class="ruleMode ? 'mode-rule' : settings.base_url ? 'mode-ai' : 'warn'">{{ ruleMode ? "规则引擎模式（离线）" : settings.base_url ? "AI 模式" : "未配置 AI 服务" }}</b>
          <template v-if="connected"> · {{ store.activePort }}</template>
          <template v-else> · <span class="warn">串口未连接，AI 无法替你执行指令</span></template>
        </p>
      </div>
      <div class="ops">
        <n-button v-if="ruleMode" size="small" type="primary" @click="retryOnline">
          <span class="btn-label-row">
            <AppIcon :icon="RefreshCw" :size="13" />
            <span class="gap-4">重连 AI</span>
          </span>
        </n-button>
        <n-button size="small" quaternary :disabled="foregroundOperations > 0" @click="clearChat">清空对话</n-button>
        <n-button size="small" quaternary @click="showSettings = true">
          <span class="btn-label-row">
            <AppIcon :icon="Settings" :size="13" />
            <span class="gap-4">配置</span>
          </span>
        </n-button>
      </div>
    </header>

    <div class="ai-context" aria-label="本轮分析上下文">
      <span>目标设备 <b>{{ connected ? store.activePort : '未连接，可咨询或分析已有材料' }}</b></span>
      <span>自动附带日志 <b>{{ attachedLogCount }} 条</b></span><span>资料 <b>{{ manualLabel }}</b></span>
      <n-button v-if="!connected" size="small" secondary @click="connectDevice">连接设备</n-button>
      <n-button v-if="!settings.base_url" size="small" secondary @click="showSettings = true">配置 AI 服务</n-button>
      <n-button v-if="connected" size="small" text @click="activeView = 'diag'">返回诊断与复测</n-button>
    </div>
    <details v-if="attachedLogCount" class="context-preview"><summary>查看随问题发送的最近串口日志</summary><pre>{{ recentLog(settings.recent_log_lines) }}</pre></details>
    <div ref="listRef" class="chat">
      <div v-if="!ui.length" class="welcome">
        <div class="welcome-head">
          <div class="welcome-eyebrow">AI 助手</div>
          <p>{{ connected ? "可以分析当前设备的日志，也可以从诊断结果带入异常。AI 给出的执行建议会先请你确认。" : "连接设备后可分析现场问题；也可以先描述故障，或粘贴已有日志。可用资料范围显示在上方。" }}</p>
        </div>
        <div class="quick-asks">
          <n-button v-for="q in QUICK_ASKS" :key="q" size="small" secondary @click="send(q)">
            {{ q }}
          </n-button>
        </div>
      </div>

      <template v-for="(item, i) in ui" :key="i">
        <div v-if="item.kind === 'user'" class="msg user">{{ item.text }}</div>
        <div v-else-if="item.kind === 'assistant'" class="msg assistant">{{ item.text }}</div>
        <div v-else-if="item.kind === 'note'" class="msg note">{{ item.text }}</div>

        <AiToolCard
          v-else-if="item.kind === 'card' || item.kind === 'diagnostic'"
          :item="item"
          :busy="busy"
          :target-available="targetAvailable(item.card?.target ?? item.diagnostic?.target)"
          @execute="item.kind === 'card' ? executeCard(item) : executeDiagnostic(item)"
          @cancel="item.kind === 'card' ? cancelCard(item) : cancelDiagnostic(item)"
          @toggle-step="(index, checked) => { if (item.card?.status === 'pending' && !busy) item.card.steps[index].checked = checked; }"
        />
      </template>

      <div v-if="requestInFlight" class="msg note request-status">
        <span>{{ requestStageLabel }}</span>
        <n-button size="tiny" text type="warning" @click="cancelActiveRequest()">取消请求</n-button>
      </div>
      <div v-else-if="busy" class="msg note">正在执行操作…</div>
      <div v-else-if="awaitingConfirmation" class="msg note" role="status">请先执行或取消上方的待确认卡片，再继续提问。</div>
    </div>

    <div class="input-row">
      <n-input
        v-model:value="input"
        type="textarea"
        :rows="2"
        placeholder="描述你的问题，如：为什么 MQTT 连不上 OneNET？（Enter 发送）"
        :disabled="busy || awaitingConfirmation"
        @keydown.enter.exact.prevent="send()"
      />
      <n-button v-if="requestInFlight" type="warning" secondary @click="cancelActiveRequest()">取消请求</n-button>
      <n-button v-else type="primary" :loading="busy" :disabled="busy || awaitingConfirmation" @click="send()">发送</n-button>
    </div>

    <!-- 配置弹窗 -->
    <n-modal v-model:show="showSettings" preset="card" class="cfg-modal" title="AI 与知识库配置">
      <div class="cfg">
        <n-input v-model:value="settings.base_url" placeholder="Base URL，如 https://onerouter.cmaiot.cn/v1" />
        <div class="cfg-key-row">
          <n-input
            v-model:value="settings.api_key"
            :type="revealApiKey ? 'text' : 'password'"
            :placeholder="settings.api_key_configured ? 'API Key 已配置，留空表示不修改' : 'API Key'"
            class="cfg-key-input"
          />
          <n-button
            size="small"
            quaternary
            :title="revealApiKey ? '隐藏 API Key' : '明文显示 API Key（请确认环境安全）'"
            @click="revealApiKey = !revealApiKey"
          >
            {{ revealApiKey ? "隐藏" : "显示" }}
          </n-button>
        </div>
        <small v-if="settings.api_key_configured" class="cfg-note">API key 保存在 Windows 系统凭据中</small>
        <div class="cfg-row">
          <n-input v-model:value="settings.model" placeholder="模型，如 auto" class="cfg-input-model" />
          <n-input-number v-model:value="settings.recent_log_lines" :min="20" :max="300" class="cfg-input-num">
            <template #suffix>行日志</template>
          </n-input-number>
        </div>
        <div class="cfg-note">
          手册知识包管理已迁移至「知识中心」页面，当前型号模式：{{ activeManualMode() === "fulltext" ? "全文" : activeManualMode() === "preview" ? "摘要" : "不可用" }}
        </div>
        <small class="cfg-note">连接地址和模型保存在应用配置目录，API key 不写入配置文件</small>
      </div>
      <template #footer>
        <div class="cfg-actions">
          <n-popconfirm v-if="settings.api_key_configured" @positive-click="clearApiKey">
            <template #trigger><n-button type="error" tertiary>清除 API Key</n-button></template>
            确认删除系统凭据中的 API key？
          </n-popconfirm>
          <n-button @click="showSettings = false">取消</n-button>
          <n-button type="primary" @click="saveSettings">保存</n-button>
        </div>
      </template>
    </n-modal>
  </div>
</template>

<style scoped>
.ai { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden; padding: 18px 24px; }
.ai-head { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: flex-start; gap: 10px 16px; margin-bottom: 12px; }
h2 { font-size: 18px; color: var(--text-primary); }
.sub { color: var(--text-secondary); font-size: 13px; margin-top: 4px; }
.mode-ai { color: var(--status-success); }
.mode-rule { color: var(--status-warning); }
.warn { color: var(--status-warning); }
.ops { display: flex; flex-wrap: wrap; gap: 6px; }

.chat {
  flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: 10px;
  padding: 6px 2px; width: 100%; max-width: 860px;
}
.welcome { color: var(--text-secondary); padding: 30px 0; }
.quick-asks { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 12px; }

.msg { padding: 10px 14px; border-radius: 12px; white-space: pre-wrap; word-break: break-word; line-height: 1.65; color: var(--text-primary); }
.msg.user { align-self: flex-end; background: var(--brand-primary-soft); max-width: 75%; }
.msg.assistant { align-self: flex-start; background: var(--surface-hover); max-width: 85%; }
.msg.note { align-self: center; font-size: 12px; color: var(--text-tertiary); padding: 2px 10px; }
.request-status { display: flex; align-items: center; gap: 10px; color: var(--text-secondary); }

.input-row { display: flex; gap: 10px; margin-top: 12px; max-width: 860px; }
.input-row > :first-child { flex: 1; }
.cfg { display: flex; flex-direction: column; gap: 12px; }

@media (max-width: 720px) {
  .ai { padding: 14px 16px; }
  .msg.user, .msg.assistant { max-width: 100%; }
}

/* === 常用工具类（替代 inline style） === */


.cfg-modal { width: 520px; }
.cfg-note { color: var(--text-tertiary); font-size: 13px; }
.cfg-row { display: flex; gap: 10px; }
.cfg-row > :first-child { flex: 1; }
.cfg-actions { display: flex; justify-content: flex-end; gap: 8px; }
.cfg-key-row { display: flex; align-items: center; gap: 8px; }
.cfg-key-row .cfg-key-input { flex: 1; min-width: 0; }
.cfg-input-model { flex: 1; }
.cfg-input-num { width: 160px; }

.ai-context { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 20px; padding: 14px 16px; margin-bottom: 10px; border: 1px solid var(--border-subtle); border-radius: 10px; background: var(--surface-raised); font-size: 12px; color: var(--text-secondary); }
.ai-context b { margin-left: 5px; color: var(--text-primary); font-weight: 500; }
.context-preview { margin: 0 0 8px; color: var(--text-secondary); font-size: 12px; }
.context-preview summary { cursor: pointer; }
.context-preview pre { max-height: 140px; overflow: auto; white-space: pre-wrap; padding: 12px; background: var(--surface-raised); }
</style>
