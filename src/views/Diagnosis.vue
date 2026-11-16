<script setup lang="ts">
import {
  NButton,
  NSelect,
  NTag,
  useDialog,
  useMessage,
} from "naive-ui";
import { ref, computed, watch, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  CheckCircle2,
  AlertTriangle,
  XCircle,
  Minus,
  Loader,
  Circle,
  Cable,
  Lightbulb,
} from "@lucide/vue";
import EmptyState from "../components/EmptyState.vue";
import { useWorkspace } from "../composables/useWorkspace";
import { useUiPrefs } from "../composables/useUiPrefs";
import { useExecutionStore } from "../stores/execution";
import AppIcon from "../components/AppIcon.vue";
import { useSerialStore } from "../stores/serial";
import type { ExecutionTarget, DiagFlow, DiagFlowFull, DiagReport, DiagnosticStepEvent, StepReport } from "../types";
import UsecaseEditor from "./UsecaseEditor.vue";
import FlowStepList from "./FlowStepList.vue";

const store = useSerialStore();
const message = useMessage();
const dialog = useDialog();
const { connectDevice, openHistory } = useWorkspace();
const { activeView } = useUiPrefs();
const executions = useExecutionStore();
const confirming = ref(false);
interface RunContext { target: ExecutionTarget; flow: DiagFlow; profileId: string; profileName: string }
const runContext = ref<RunContext | null>(null);

const editorShow = ref(false);
const editorMode = ref<"create" | "edit" | "clone">("create");
const editorBase = ref<DiagFlowFull | null>(null);
function openEditor(mode: "create" | "edit" | "clone") {
  editorMode.value = mode;
  editorBase.value = mode === "create" ? null : (store.knowledge?.flows.find((f) => f.id === flowId.value) as unknown as DiagFlowFull) ?? null;
  editorShow.value = true;
}
function onFlowSaved(id: string) {
  if (id) flowId.value = id;
}

function onEditorSaved(id: string) {
  if (id) flowId.value = id;
}

const flowId = ref("network_attach");
const running = ref(false);
const steps = ref<StepReport[]>([]);
const report = ref<DiagReport | null>(null);
const expanded = ref<Set<string>>(new Set());
const runId = ref("");
const diagPort = computed(() => store.activeSession?.connected ? store.activePort : "");
const startedAt = ref(0);
const now = ref(Date.now());
const diagProfile = computed(() => {
  const profileId = store.sessions.find((session) => session.port === diagPort.value)?.profileId;
  return store.knowledge?.profiles.find((profile) => profile.id === profileId);
});

const CATEGORY_LABELS: Record<string, string> = {
  socket: "Socket 数据",
  mqtt: "MQTT",
  http: "HTTP",
  ftp: "FTP",
  network: "网络",
  power: "功耗/睡眠",
  sms: "短信",
  hardware: "外设/硬件",
  fota: "FOTA",
  misc: "其他",
};

/**
 * 适用流程 id 集合——由后端 list_applicable_flows 判定（与执行命令
 * 共用 modulink-core 同一实现，"能看到的一定能跑"）。null = 加载中，全放行。
 */
const applicableIds = ref<Set<string> | null>(null);

type FlowBucket = "recommended" | "standard" | "example" | "mine";
function flowBucket(flow: DiagFlow): FlowBucket {
  if (flow.kind !== "usecase") return "recommended";
  if (flow.id.startsWith("my_")) return "mine";
  if (flow.verified && flow.applies_to.length) return "standard";
  return "example";
}

let applicabilityVersion = 0;
async function refreshApplicable() {
  const version = ++applicabilityVersion;
  try {
    const ids = await invoke<string[]>("list_applicable_flows", {
      profileId: diagProfile.value?.id ?? null,
    });
    if (version === applicabilityVersion) applicableIds.value = new Set(ids);
  } catch {
    if (version === applicabilityVersion) applicableIds.value = null; // 查询失败时不过滤，后端执行时仍会兜底校验
  }
}

const flowOptions = computed(() => {
  const all = store.knowledge?.flows ?? [];
  const flows = applicableIds.value ? all.filter((f) => applicableIds.value!.has(f.id)) : all;
  const recommended = flows.filter((f) => flowBucket(f) === "recommended");
  const standard = flows.filter((f) => flowBucket(f) === "standard");
  const examples = flows.filter((f) => flowBucket(f) === "example");
  const mine = flows.filter((f) => flowBucket(f) === "mine");
  const groups: any[] = [
    {
      type: "group",
      label: "推荐检查",
      key: "recommended",
      children: recommended.map((f) => ({ label: f.name, value: f.id })),
    },
  ];
  const byCategory = (list: DiagFlow[], prefix: string, tag: string) => {
    const byCat = new Map<string, DiagFlow[]>();
    for (const f of list) {
      const bucket = byCat.get(f.category) ?? [];
      bucket.push(f);
      byCat.set(f.category, bucket);
    }
    for (const [cat, bucket] of [...byCat.entries()].sort()) {
      groups.push({
        type: "group",
        label: `${tag} · ${CATEGORY_LABELS[cat] ?? cat}（${bucket.length}）`,
        key: `${prefix}-${cat}`,
        children: bucket.map((f) => ({
          label: (f.verified ? "" : "◇ ") + f.name,
          value: f.id,
        })),
      });
    }
  };
  byCategory(standard, "standard", "标准测试用例");
  byCategory(mine, "mine", "我的流程");
  byCategory(examples, "example", "手册示例（未复核）");
  return groups.filter((g) => g.children.length);
});
const selectedFlow = computed(() => store.knowledge?.flows.find((flow) => flow.id === flowId.value));

// 切换流程时清空上一次的运行结果，避免旧步骤卡片残留挡住新流程的步骤列表
watch(flowId, () => {
  if (running.value || confirming.value) return; // 运行中不清（结果归属 runId，不会串）
  steps.value = [];
  report.value = null;
  expanded.value = new Set();
});

// 适用流程集合刷新后，若当前选中项已不适用（如切了端口/画像），自动回落
watch(
  () => applicableIds.value,
  (ids) => {
    if (!ids || running.value) return;
    if (flowId.value && !ids.has(flowId.value)) {
      flowId.value = ids.has("network_attach") ? "network_attach" : [...ids][0] ?? "";
    }
  }
);
const displaySteps = computed(() =>
  ((running.value || report.value ? runContext.value?.flow : selectedFlow.value)?.steps ?? []).map((planned, index) => {
    const completed = steps.value.find((step) => step.id === planned.id);
    if (completed) return completed;
    return {
      id: planned.id,
      title: planned.title,
      result: running.value && index === steps.value.length ? "running" : "pending",
      detail: "",
      reason: "",
      suggestion: "",
      raw: [] as string[],
      collected: [] as [string, string[]][],
      elapsed_ms: 0,
    };
  })
);
const progress = computed(() =>
  displaySteps.value.length ? (steps.value.length / displaySteps.value.length) * 100 : 0
);
const summary = computed(() => ({
  pass: steps.value.filter((step) => step.result === "pass").length,
  warn: steps.value.filter((step) => step.result === "warn").length,
  fail: steps.value.filter((step) => step.result === "fail").length,
  skip: steps.value.filter((step) => step.result === "skip").length,
}));
const elapsedMs = computed(() =>
  running.value && startedAt.value
    ? now.value - startedAt.value
    : steps.value.reduce((total, step) => total + step.elapsed_ms, 0)
);
const sameTarget = (context: RunContext) => store.activePort === context.target.port
  && store.activeSession?.connected === true && store.activeSession.generation === context.target.sessionGeneration
  && store.activeProfileId === context.profileId;
const canReviewCurrent = computed(() => !!runContext.value && sameTarget(runContext.value));
watch(() => [store.activePort, store.activeSession?.generation, store.activeProfileId], () => {
  if (!running.value && !confirming.value) { report.value = null; steps.value = []; runContext.value = null; }
});
let unlisten: UnlistenFn | null = null;
let clock = 0;
onMounted(async () => {
  await store.init();
  // 画像变化（切端口/自动识别/热重载知识库）时刷新适用流程集合
  watch(
    () => [diagProfile.value?.id, store.knowledge?.flows.length] as const,
    () => refreshApplicable(),
    { immediate: true }
  );
  unlisten = await listen<DiagnosticStepEvent>("diag://step", (e) => {
    if (e.payload.run_id === runId.value) steps.value.push(e.payload.step);
  });
  clock = window.setInterval(() => (now.value = Date.now()), 250);
});
onUnmounted(() => {
  unlisten?.();
  window.clearInterval(clock);
});

const ICONS = {
  pass: CheckCircle2,
  warn: AlertTriangle,
  fail: XCircle,
  skip: Minus,
  running: Loader,
  pending: Circle,
} as const;

function confirmUnverifiedFlow(): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false;
    const done = (value: boolean) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    dialog.warning({
      title: "该流程未经实机复核",
      content: "此流程由手册示例自动转换，参数可能需要按现场环境调整。继续后仅允许绕过手册证据不足；无效指令、当前型号限制和实测能力限制仍会被后端拦截。",
      positiveText: "确认并运行",
      negativeText: "取消",
      onPositiveClick: () => done(true),
      onNegativeClick: () => done(false),
      onMaskClick: () => done(false),
      onClose: () => done(false),
      // 兜底：万一 onClose 因 race condition 未触发，关闭动画结束后强制 settle
      onAfterLeave: () => done(false),
    });
  });
}

async function start() {
  if (running.value || confirming.value) return;
  if (!diagPort.value || !selectedFlow.value) { connectDevice(); return; }
  const context: RunContext = {
    target: store.executionTarget(diagPort.value),
    flow: { ...selectedFlow.value, steps: [...selectedFlow.value.steps] },
    profileId: store.activeProfileId,
    profileName: diagProfile.value?.name ?? "未识别型号",
  };
  confirming.value = true;
  try {
    const allowUnverified = !context.flow.verified && await confirmUnverifiedFlow();
    if (!context.flow.verified && !allowUnverified) return;
    if (!sameTarget(context)) { message.warning("当前设备已变化，请确认目标设备后重新开始检查"); return; }
    steps.value = []; report.value = null; expanded.value = new Set();
    runContext.value = context;
    runId.value = crypto.randomUUID(); startedAt.value = Date.now(); now.value = startedAt.value;
    running.value = true;
    report.value = await invoke<DiagReport>("run_diagnostic", {
      port: context.target.port, sessionGeneration: context.target.sessionGeneration,
      flowId: context.flow.id, runId: runId.value, profileId: context.profileId || null, allowUnverified,
    });
    steps.value = report.value.steps;
    expanded.value = new Set(report.value.steps.filter((step) => step.result === "fail" || step.result === "warn").map((step) => step.id));
  } catch (e) {
    if (String(e) === "CANCELLED") message.info("诊断已取消");
    else message.error(String(e));
  } finally { running.value = false; confirming.value = false; }
}

function analyzeReport() {
  if (!report.value || !runContext.value || !canReviewCurrent.value) return;
  const context = runContext.value;
  executions.pendingAiPrompt = `请分析这次设备诊断，区分已确认的问题与待验证原因，并建议下一项检查。\n设备：${context.target.port} · ${context.profileName}\n流程：${context.flow.name}\n结果：${overallText.value}\n\n`
    + report.value.steps.map((step) => `${step.title} [${step.result}]\n${step.detail}\n原因：${step.reason}\n建议：${step.suggestion}\n${step.raw.join("\n")}`).join("\n\n").slice(0, 12000);
  activeView.value = "ai";
}
function viewEvidence() {
  expanded.value = new Set(report.value?.steps.map((step) => step.id) ?? []);
  document.querySelector(".diag .steps")?.scrollIntoView({ behavior: "smooth", block: "start" });
}
function retest() {
  if (!runContext.value || !canReviewCurrent.value || running.value) return;
  flowId.value = runContext.value.flow.id;
  void start();
}

async function cancel() {
  if (!running.value || !runId.value) return;
  await invoke("cancel_diagnostic", { runId: runId.value });
}

function toggle(id: string) {
  expanded.value.has(id) ? expanded.value.delete(id) : expanded.value.add(id);
  expanded.value = new Set(expanded.value);
}

const overallText = computed(() => {
  if (!report.value) return "";
  return { pass: "体检通过，未发现阻断问题", warn: "基本正常，但有需要注意的项", fail: "发现故障，请按建议处理" }[
    report.value.overall
  ];
});

function fmtDuration(ms: number) {
  if (ms < 1000) return `${ms} ms`;
  const seconds = Math.floor(ms / 1000);
  return seconds >= 60 ? `${Math.floor(seconds / 60)}分 ${seconds % 60}秒` : `${seconds}秒`;
}

async function exportReport() {
  if (!report.value) return;
  const content = buildHtml();
  const path = await saveDialog({
    defaultPath: `诊断报告_${runContext.value?.target.port}_${new Date().toISOString().slice(0, 10)}.html`,
    filters: [{ name: "HTML 报告", extensions: ["html"] }],
  });
  if (!path) return;
  await invoke("save_text_file", { path, content });
  message.success("报告已导出，可直接发给 FAE/客户");
}

function esc(s: string) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function buildHtml() {
  const r = report.value!;
  const color: Record<string, string> = { pass: "#18a058", warn: "#f0a020", fail: "#d03050", skip: "#999" };
  // 导出 HTML 里不能用 Vue 组件对象（会渲染为 [object Object]），用文本符号
  const ICON_GLYPH: Record<string, string> = {
    pass: "✔",
    warn: "!",
    fail: "✘",
    skip: "-",
    running: "…",
    pending: "○",
  };
  const rows = r.steps
    .map(
      (s) => `
    <div class="step">
      <div class="head" style="color:${color[s.result]}">${ICON_GLYPH[s.result] ?? "?"} ${esc(s.title)}
        <small>${s.elapsed_ms} ms</small></div>
      ${s.detail ? `<div class="detail">${esc(s.detail)}</div>` : ""}
      ${s.reason ? `<div class="reason">原因：${esc(s.reason)}</div>` : ""}
      ${s.suggestion ? `<div class="sugg">建议：${esc(s.suggestion)}</div>` : ""}
      ${s.raw.length ? `<pre>${esc(s.raw.join("\n"))}</pre>` : ""}
      ${s.collected
        .map(([c, lines]) => `<pre class="collect">${esc(c)}\n${esc(lines.join("\n"))}</pre>`)
        .join("")}
    </div>`
    )
    .join("");
  return `<!doctype html><html lang="zh"><head><meta charset="utf-8"><title>诊断报告</title>
<style>
body{font-family:"Segoe UI","Microsoft YaHei",sans-serif;max-width:820px;margin:32px auto;padding:0 20px;color:#222}
h1{font-size:22px} .meta{color:#666;font-size:13px;margin-bottom:20px}
.overall{padding:12px 16px;border-radius:8px;font-weight:600;margin:16px 0;background:${
    { pass: "#e8f5ec", warn: "#fdf3e3", fail: "#fbebee" }[r.overall]
  };color:${color[r.overall]}}
.step{border:1px solid #e5e7eb;border-radius:8px;padding:12px 16px;margin-bottom:10px}
.head{font-weight:600} .head small{color:#999;font-weight:400;margin-left:8px}
.detail{color:#555;margin-top:4px} .reason{color:#d03050;margin-top:4px} .sugg{color:#0369a1;margin-top:4px}
pre{background:#f6f8fa;border-radius:6px;padding:8px 10px;font-size:12px;overflow-x:auto;margin-top:8px}
.collect{border-left:3px solid #f0a020}

.task-path { display: flex; align-items: center; gap: 18px; padding: 16px 18px; margin-bottom: 20px; background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 10px; color: var(--text-tertiary); font-size: 13px; }
.task-path span { display: flex; align-items: center; gap: 8px; }
.task-path i { display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; background: var(--surface-hover); font-style: normal; }
.task-path .current { color: var(--brand-primary); font-weight: 600; }
.task-path .current i { background: var(--brand-primary-soft); }
.task-path .done { color: var(--status-success); }
.task-path > b { font-weight: normal; }
.run-target { padding: 12px 0; color: var(--text-secondary); font-size: 13px; }
.target-changed { display: block; margin-top: 8px; color: var(--status-warning); }
.result-review { border: 1px solid var(--border-default); border-left: 4px solid var(--status-warning); background: var(--surface-raised); border-radius: 10px; padding: 20px; margin: 4px 0 18px; }
.result-pass { border-left-color: var(--status-success); }
.result-fail { border-left-color: var(--status-danger); }
.result-heading { display: flex; align-items: center; gap: 12px; }
.result-heading h3 { margin: 0 0 4px; font-size: 17px; }
.result-heading p, .result-next { color: var(--text-secondary); font-size: 13px; }
.result-next { margin: 14px 0; }
.result-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; }
.check-plan { border: 1px solid var(--border-subtle); border-radius: 10px; padding: 16px; background: var(--surface-raised); }
.check-plan summary { cursor: pointer; color: var(--text-secondary); font-size: 13px; }
.check-plan[open] summary { margin-bottom: 16px; }
@media (max-width: 850px) { .task-path { gap: 8px; padding: 12px; font-size: 12px; } }
</style></head><body>
<h1>设备诊断报告</h1>
<div class="meta">工具：ModuLink Studio ｜ 流程：${esc(r.flow_name)} ｜ 端口：${esc(runContext.value?.target.port ?? "-")}
 ｜ 型号：${esc(runContext.value?.profileName ?? "-")} ｜ 时间：${new Date(startedAt.value).toLocaleString("zh-CN")}</div>
<div class="overall">${ICON_GLYPH[r.overall] ?? ""} ${esc(overallText.value)}</div>
<div class="meta">检查项：${r.steps.length} ｜ 通过：${summary.value.pass} ｜ 警告：${summary.value.warn} ｜ 失败：${summary.value.fail} ｜ 耗时：${fmtDuration(elapsedMs.value)}</div>
${rows}
</body></html>`;
}
</script>

<template>
  <div class="diag">
    <header class="diag-head">
      <div>
        <div class="page-kicker">排障工作台</div><h2>设备诊断</h2>
        <p class="sub">从基础检查定位异常，查看证据，处理后复测。</p>
      </div>
      <div class="controls">
        <n-select
          v-model:value="flowId"
          :disabled="running || confirming"
          :options="flowOptions"
          size="small"
          filterable
          style="width: 240px"
          placeholder="选择验证流程"
        />
        <n-button v-if="!running" type="primary" :disabled="!diagPort || !selectedFlow || confirming" :loading="confirming" @click="start">
          开始检查
        </n-button>
        <n-button v-else type="warning" @click="cancel">取消运行</n-button>
        <n-button secondary @click="openHistory">历史记录</n-button>

      </div>
    </header>

    <div class="task-path" aria-label="排障步骤">
      <span :class="{ done: !!diagPort }"><i>1</i> 连接设备</span><b>→</b>
      <span :class="{ current: !!diagPort && !report, done: !!report }"><i>2</i> 检查问题</span><b>→</b>
      <span :class="{ current: !!report }"><i>3</i> 处理与复测</span>
    </div>
    <EmptyState v-if="!diagPort && !running && !report" :icon="Cable" title="先连接需要检查的设备"
      description="连接后自动识别型号，并提供适用的检查流程。默认从入网连通性开始。" action-label="连接设备" @action="connectDevice" />
    <div v-if="(running || report) && runContext" class="run-target" role="status">
      {{ running ? '正在检查' : '本次结果' }}：<b>{{ runContext.target.port }} · {{ runContext.profileName }}</b> · {{ runContext.flow.name }}
      <span v-if="!canReviewCurrent" class="target-changed">当前设备已变化。本次结果仍属于上面的设备；复测与 AI 分析需使用同一设备会话。</span>
    </div>
    <section v-if="report" class="result-review" :class="`result-${report.overall}`">
      <div class="result-heading"><AppIcon :icon="ICONS[report.overall]" :size="22" /><div><h3>{{ overallText }}</h3><p>通过 {{ summary.pass }} 项 · 警告 {{ summary.warn }} 项 · 失败 {{ summary.fail }} 项 · {{ fmtDuration(elapsedMs) }}</p></div></div>
      <p class="result-next">{{ report.overall === 'pass' ? '可导出本次报告，或继续在控制台调试。' : '展开下方异常证据，按建议处理，再对当前设备复测。' }}</p>
      <div class="result-actions">
        <n-button type="primary" :disabled="!canReviewCurrent" @click="analyzeReport">{{ report.overall === 'pass' ? 'AI 解读结果' : 'AI 分析异常' }}</n-button>
        <n-button :disabled="!canReviewCurrent" @click="retest">处理后复测</n-button>
        <n-button @click="viewEvidence">查看原始证据</n-button><n-button @click="exportReport">导出报告</n-button>
        <n-button text :disabled="!canReviewCurrent" @click="activeView = 'console'">返回控制台</n-button>
      </div>
    </section>
    <div v-if="diagPort && selectedFlow && !running && !report" class="run-meta">
      <span>{{ diagProfile?.name ?? "通用画像" }}</span>
      <span>{{ selectedFlow.steps.length }} 个检查项</span>
      <span v-if="selectedFlow.source" class="meta-source">来源：{{ selectedFlow.source }}</span>
      <n-tag v-if="selectedFlow.kind === 'checkup'" size="tiny" type="success" :bordered="false">
        推荐检查 · 内置
      </n-tag>
      <n-tag v-else-if="selectedFlow.id.startsWith('my_')" size="tiny" type="info" :bordered="false">
        我的流程 · 可编辑
      </n-tag>
      <n-tag v-else-if="selectedFlow.verified && selectedFlow.applies_to.length" size="tiny" type="success" :bordered="false">
        标准测试用例 · 已复核
      </n-tag>
      <n-tag v-else size="tiny" type="warning" :bordered="false">
        手册示例 · 仅供参考
      </n-tag>
      <n-tag v-if="selectedFlow.kind === 'usecase' && !selectedFlow.applies_to.length" size="tiny" type="warning" :bordered="false">
        未标适用型号，指令集可能与当前模组不符
      </n-tag>
      <n-tag v-if="selectedFlow.kind === 'usecase' && !selectedFlow.verified" size="tiny" type="warning" :bordered="false">
        未实机复核，参数（如服务器地址）可能需按实际调整
      </n-tag>
      <n-tag v-if="selectedFlow.execution_mode === 'readonly'" size="tiny" type="success" :bordered="false">
        只读检查
      </n-tag>
      <n-tag v-else-if="selectedFlow.execution_mode === 'mutating'" size="tiny" type="error" :bordered="false">
        可能修改设备
      </n-tag>
      <n-tag v-else size="tiny" type="default" :bordered="false">
        影响范围未标注
      </n-tag>
      <span v-if="running || report">{{ fmtDuration(elapsedMs) }}</span>
    </div>

    <details v-if="diagPort && selectedFlow && !running && !report" class="check-plan">
      <summary>查看检查步骤与高级设置 · {{ selectedFlow.steps.length }} 项</summary>
      <FlowStepList :flow="selectedFlow" :disabled="running || confirming" @saved="onFlowSaved" />
      <div class="result-actions">
        <n-button size="small" :disabled="confirming" @click="openEditor('create')">新建流程</n-button>
        <n-button size="small" :disabled="confirming" @click="openEditor('clone')">另存流程</n-button>
        <n-button v-if="selectedFlow.id.startsWith('my_')" size="small" :disabled="confirming" @click="openEditor('edit')">编辑流程</n-button>
      </div>
    </details>
    <div v-if="diagPort && !selectedFlow && !running && !report" class="empty">当前型号没有可用检查流程，可在知识中心检查资料，或切换设备型号。</div>
    <div v-if="running || report" class="progress-area">
      <div class="progress-track"><span :style="{ width: `${progress}%` }" /></div>
      <div class="progress-summary">
        <span>{{ steps.length }} / {{ displaySteps.length }}</span>
        <span class="sum-pass">通过 {{ summary.pass }}</span>
        <span class="sum-warn">警告 {{ summary.warn }}</span>
        <span class="sum-fail">失败 {{ summary.fail }}</span>
        <span v-if="summary.skip">跳过 {{ summary.skip }}</span>
      </div>
    </div>

    <div v-if="running || report" class="steps">
      <div v-for="s in displaySteps" :key="s.id" class="step-card" :class="`st-${s.result}`">
        <div class="step-head" @click="s.result !== 'pending' && s.result !== 'running' && toggle(s.id)">
          <span class="step-icon">
            <AppIcon :icon="ICONS[s.result as keyof typeof ICONS] || Circle" :size="16" />
          </span>
          <span class="step-title">{{ s.title }}</span>
          <span v-if="s.detail" class="step-detail">{{ s.detail }}</span>
          <span v-if="s.result === 'running'" class="step-ms active">检测中</span>
          <span v-else-if="s.result === 'pending'" class="step-ms">等待</span>
          <span v-else class="step-ms">{{ s.elapsed_ms }} ms</span>
        </div>
        <div v-if="s.reason" class="step-reason">原因：{{ s.reason }}</div>
        <div v-if="s.suggestion" class="step-sugg">
          <AppIcon :icon="Lightbulb" :size="13" />
          <span>{{ s.suggestion }}</span>
        </div>
        <template v-if="expanded.has(s.id)">
          <pre v-if="s.raw.length" class="step-raw">{{ s.raw.join("\n") }}</pre>
          <pre v-for="[cmd, lines] in s.collected" :key="cmd" class="step-raw collect">补充采集 {{ cmd }}
{{ lines.join("\n") }}</pre>
        </template>
      </div>
      <div v-if="running && !steps.length" class="empty">正在检测…</div>
    </div>
  </div>
  <UsecaseEditor v-model:show="editorShow" :mode="editorMode" :base-flow="editorBase" @saved="onEditorSaved" />
</template>

<style scoped>
.page-kicker { color: var(--brand-primary); font-size: 12px; font-weight: 600; margin-bottom: 6px; }
.diag { flex: 1; display: flex; flex-direction: column; padding: 18px 24px; min-width: 0; overflow-y: auto; }
.diag-head { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: flex-start; gap: 12px 20px; margin-bottom: 16px; }
h2 { font-size: 18px; }
.sub { color: var(--text-secondary); font-size: 13px; margin-top: 4px; }
.controls { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
.empty { color: var(--text-tertiary); text-align: center; padding: 40px 0; }
.run-meta {
  display: flex; flex-wrap: wrap; align-items: center; gap: 6px 18px;
  margin: -7px 0 14px; font-size: 12px; color: var(--text-tertiary);
}
.meta-source { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 320px; }

.overall { padding: 12px 16px; border-radius: 10px; font-weight: 600; margin-bottom: 14px; }
.ov-pass { background: var(--status-success-soft); color: var(--status-success); }
.ov-warn { background: var(--status-warning-soft); color: var(--status-warning); }
.ov-fail { background: var(--status-danger-soft); color: var(--status-danger); }
.progress-area { max-width: 780px; margin-bottom: 12px; }
.progress-track { height: 4px; overflow: hidden; background: var(--border-subtle); border-radius: 2px; }
.progress-track span { display: block; height: 100%; background: var(--status-success); transition: width .2s ease; }
.progress-summary {
  display: flex; flex-wrap: wrap; gap: 6px 16px;
  margin-top: 7px; font-size: 12px; color: var(--text-secondary);
}
.sum-pass { color: var(--status-success); }
.sum-warn { color: var(--status-warning); }
.sum-fail { color: var(--status-danger); }

.steps { display: flex; flex-direction: column; gap: 10px; width: 100%; max-width: 780px; }
.step-card {
  border: 1px solid var(--border-default); border-radius: 10px;
  padding: 12px 16px; background: var(--surface-raised);
  border-left-width: 4px;
}
.st-pass { border-left-color: var(--status-success); }
.st-warn { border-left-color: var(--status-warning); }
.st-fail { border-left-color: var(--status-danger); }
.st-skip { border-left-color: var(--text-tertiary); opacity: 0.6; }
.st-running { border-left-color: var(--brand-primary); background: var(--brand-primary-soft); }
.st-pending { border-left-color: var(--border-default); opacity: .55; }

.step-head { display: flex; flex-wrap: wrap; align-items: baseline; gap: 6px 10px; cursor: pointer; }
.step-icon { display: inline-flex; align-items: center; }
.st-pass .step-icon { color: var(--status-success); }
.st-warn .step-icon { color: var(--status-warning); }
.st-fail .step-icon { color: var(--status-danger); }
.st-running .step-icon { color: var(--brand-primary); }
.step-title { font-weight: 600; color: var(--text-primary); }
.step-detail { color: var(--text-secondary); font-size: 13px; }
.step-ms { margin-left: auto; color: var(--text-tertiary); font-size: 12px; }
.step-ms.active { color: var(--brand-primary); }
.step-reason { color: var(--status-danger); font-size: 13px; margin-top: 6px; }
.step-sugg {
  display: flex; align-items: flex-start; gap: 6px;
  color: var(--status-info); font-size: 13px; margin-top: 4px; line-height: 1.6;
}
.step-raw {
  background: var(--surface-base); border-radius: 6px;
  padding: 8px 10px; margin-top: 8px;
  font-family: Consolas, monospace; font-size: 12px;
  white-space: pre-wrap; word-break: break-all;
  color: var(--text-secondary);
}
.step-raw.collect { border-left: 3px solid var(--status-warning); }

@media (max-width: 720px) {
  .diag { padding: 14px 16px; }
  .step-detail { flex-basis: calc(100% - 25px); margin-left: 25px; }
}

.task-path { display: flex; align-items: center; gap: 18px; padding: 16px 18px; margin-bottom: 20px; background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 10px; color: var(--text-tertiary); font-size: 13px; }
.task-path span { display: flex; align-items: center; gap: 8px; }
.task-path i { display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; background: var(--surface-hover); font-style: normal; }
.task-path .current { color: var(--brand-primary); font-weight: 600; }
.task-path .current i { background: var(--brand-primary-soft); }
.task-path .done { color: var(--status-success); }
.task-path > b { font-weight: normal; }
.run-target { padding: 12px 0; color: var(--text-secondary); font-size: 13px; }
.target-changed { display: block; margin-top: 8px; color: var(--status-warning); }
.result-review { border: 1px solid var(--border-default); border-left: 4px solid var(--status-warning); background: var(--surface-raised); border-radius: 10px; padding: 20px; margin: 4px 0 18px; }
.result-pass { border-left-color: var(--status-success); }
.result-fail { border-left-color: var(--status-danger); }
.result-heading { display: flex; align-items: center; gap: 12px; }
.result-heading h3 { margin: 0 0 4px; font-size: 17px; }
.result-heading p, .result-next { color: var(--text-secondary); font-size: 13px; }
.result-next { margin: 14px 0; }
.result-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; }
.check-plan { border: 1px solid var(--border-subtle); border-radius: 10px; padding: 16px; background: var(--surface-raised); }
.check-plan summary { cursor: pointer; color: var(--text-secondary); font-size: 13px; }
.check-plan[open] summary { margin-bottom: 16px; }
@media (max-width: 850px) { .task-path { gap: 8px; padding: 12px; font-size: 12px; } }
</style>
