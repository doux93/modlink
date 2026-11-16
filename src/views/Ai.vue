<script setup lang="ts">
import {
  NButton,
  NInput,
  useDialog,
  useMessage,
} from "naive-ui";
import { ref, computed, nextTick, onMounted, onActivated, onBeforeUnmount, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Settings } from "@lucide/vue";
import AppIcon from "../components/AppIcon.vue";
import AiToolCard from "../components/ai/AiToolCard.vue";
import AiSettingsDialog from "../components/ai/AiSettingsDialog.vue";
import { useAiSettings } from "../composables/useAiSettings";
import { useWorkspace } from "../composables/useWorkspace";
import { useUiPrefs } from "../composables/useUiPrefs";
import { useSerialStore } from "../stores/serial";
import { useExecutionStore } from "../stores/execution";
import { usePendingTools } from "../composables/usePendingTools";
import { useAiRequestGate, AiRequestTimeoutError, isRequestCancelled, type AiRequestContext } from "../composables/useAiRequestGate";
import { deviceContextKey } from "../utils/deviceContext";
import type { AiExecutionTarget, UiItem, ManualSearchResult, AtCommandAuthorization } from "../types/ai";
import { requestAiChat } from "../utils/aiTransport";
import { createAiEvidence } from "../services/aiEvidence";
import { toolsForMode } from "../services/aiTools";
import type {
  DiagReport,
  ExecutionTarget,
  KnowledgeCapabilities,
} from "../types";

const store = useSerialStore();
const execStore = useExecutionStore();
const message = useMessage();
const dialog = useDialog();
const { connectDevice } = useWorkspace();
const { activeView } = useUiPrefs();

// ---------------- 配置 ----------------
const aiSettings = useAiSettings((text) => message.success(text));
const { settings, load: loadSettings, open: openSettings } = aiSettings;
const capabilitiesLoading = ref(true);
let capabilitiesRequestVersion = 0;
const CAPABILITIES_TIMEOUT_MS = 5000;

onMounted(async () => {
  const capabilityLoad = refreshCapabilities();
  await store.init();
  await loadSettings();
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
const { recentLog, activeManualMode, systemPrompt, localRisk, runtimeProbeDenial } = createAiEvidence(() => ({
  profile: store.activeProfile, session: store.activeSession, knowledge: store.knowledge,
  frames: store.activeFrames, capabilities: capabilities.value, persistedEvidence: persistedCapabilityEvidence.value,
}));

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
  const body = {
    messages: [{ role: "system", content: systemPrompt(question, manualEvidence) }, ...llmMessages.value],
    tools: toolsForMode(mode),
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

function onInputKey(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) return;
  if (event.key === "Enter" && !event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey) {
    event.preventDefault();
    void send();
  }
}
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
        <n-button size="small" quaternary @click="openSettings">
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
      <n-button v-if="!settings.base_url" size="small" secondary @click="openSettings">配置 AI 服务</n-button>
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
        @keydown="onInputKey"
      />
      <n-button v-if="requestInFlight" type="warning" secondary @click="cancelActiveRequest()">取消请求</n-button>
      <n-button v-else type="primary" :loading="busy" :disabled="busy || awaitingConfirmation" @click="send()">发送</n-button>
    </div>

    <AiSettingsDialog :controller="aiSettings" :manual-mode="activeManualMode()" />
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

@media (max-width: 720px) {
  .ai { padding: 14px 16px; }
  .msg.user, .msg.assistant { max-width: 100%; }
}

/* === 常用工具类（替代 inline style） === */



.ai-context { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 20px; padding: 14px 16px; margin-bottom: 10px; border: 1px solid var(--border-subtle); border-radius: 10px; background: var(--surface-raised); font-size: 12px; color: var(--text-secondary); }
.ai-context b { margin-left: 5px; color: var(--text-primary); font-weight: 500; }
.context-preview { margin: 0 0 8px; color: var(--text-secondary); font-size: 12px; }
.context-preview summary { cursor: pointer; }
.context-preview pre { max-height: 140px; overflow: auto; white-space: pre-wrap; padding: 12px; background: var(--surface-raised); }
</style>
