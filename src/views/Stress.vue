<script setup lang="ts">
import {
  NAlert,
  NButton,
  NCheckbox,
  NInput,
  NInputNumber,
  NRadioButton,
  NRadioGroup,
  NSelect,
  NTab,
  NTabs,
  NTag,
  useDialog,
  useMessage,
} from "naive-ui";
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Play, Square } from "@lucide/vue";
import AppIcon from "../components/AppIcon.vue";
import { useSerialStore } from "../stores/serial";
import TrendChart from "./TrendChart.vue";
import FlowStepList from "./FlowStepList.vue";
import UsecaseEditor from "./UsecaseEditor.vue";
import type {
  DiagFlow,
  DiagFlowFull,
  ScenarioRoundEvent,
  ScenarioStressReport,
  StressProgressEvent,
  StressReport,
  StressSample,
} from "../types";

const store = useSerialStore();
const message = useMessage();
const dialog = useDialog();

const mode = ref<"single" | "scenario">("single");

// ---- 用例编辑器（与体检页共用组件；压测场景选中的用例可就地编辑/另存） ----
const editorShow = ref(false);
const editorMode = ref<"create" | "edit" | "clone">("create");
const editorBase = ref<DiagFlowFull | null>(null);
function openEditor(m: "create" | "edit" | "clone") {
  editorMode.value = m;
  editorBase.value =
    m === "create"
      ? null
      : ((store.knowledge?.flows.find((f) => f.id === scFlowId.value) as unknown as DiagFlowFull) ?? null);
  editorShow.value = true;
}
function onEditorSaved(id: string) {
  if (id) scFlowId.value = id;
  refreshScApplicable();
}
function onFlowSaved(id: string) {
  if (id) scFlowId.value = id;
  refreshScApplicable();
}

// ---- shared ----
const port = ref("");
const running = ref(false);
const stopping = ref(false);
const runId = ref("");
const startedAt = ref(0);
const now = ref(Date.now());

// ---- single-command mode ----
const preset = ref("alive");
const command = ref("AT");
const iterations = ref(100);
const intervalMs = ref(100);
const timeoutMs = ref(3000);
const expectedPattern = ref("");
const stopOnFailure = ref(false);
const completed = ref(0);
const passed = ref(0);
const failed = ref(0);
const report = ref<StressReport | null>(null);
const recentSamples = ref<StressSample[]>([]);

const presets = [
  { label: "AT 通道稳定性", value: "alive", command: "AT", pattern: "" },
  { label: "信号查询", value: "signal", command: "AT+CSQ", pattern: "\\+CSQ:" },
  { label: "网络注册查询", value: "registration", command: "AT+CEREG?", pattern: "\\+CEREG:" },
  { label: "自定义", value: "custom", command: "", pattern: "" },
];

// ---- scenario mode ----
const scFlowId = ref("");
const scIterations = ref(100);
const scInterval = ref(200);
const scDurationMode = ref<"count" | "time">("count");
const scHours = ref(1);
const scFailurePolicy = ref<"continue" | "stop" | "recover">("recover");
const scRecoveryAuto = ref(true);
const scRecoveryText = ref("");
const scWarnAsPass = ref(true);
const scReport = ref<ScenarioStressReport | null>(null);
const scRounds = ref<ScenarioRoundEvent["round"][]>([]);
const scTrend = ref<{ i: number; ms: number; ok: boolean }[]>([]);
const scApplicableIds = ref<Set<string> | null>(null);
const scFlowSelectOpen = ref(false);

const CATEGORY_LABELS: Record<string, string> = {
  socket: "Socket 数据", mqtt: "MQTT", http: "HTTP", ftp: "FTP",
  network: "网络", power: "功能/睡眠", sms: "短信", hardware: "外设/硬件",
  fota: "FOTA", misc: "其他", custom: "自定义",
};

const portOptions = computed(() =>
  store.sessions.filter((session) => session.connected).map((session) => ({
    label: `${session.port} @${session.baud}`,
    value: session.port,
  }))
);
const stressProfile = computed(() => {
  const profileId = store.sessions.find((session) => session.port === port.value)?.profileId;
  return store.knowledge?.profiles.find((profile) => profile.id === profileId);
});

async function refreshScApplicable() {
  try {
    const ids = await invoke<string[]>("list_applicable_flows", {
      profileId: stressProfile.value?.id ?? null,
    });
    scApplicableIds.value = new Set(ids);
  } catch {
    scApplicableIds.value = null;
  }
}

const scFlowOptions = computed(() => {
  const all = store.knowledge?.flows ?? [];
  const flows = scApplicableIds.value ? all.filter((f) => scApplicableIds.value!.has(f.id)) : all;
  const recommended = flows.filter((f) => f.kind !== "usecase");
  const standard = flows.filter((f) => f.kind === "usecase" && !f.id.startsWith("my_") && f.verified && f.applies_to.length);
  const mine = flows.filter((f) => f.id.startsWith("my_"));
  const examples = flows.filter((f) => f.kind === "usecase" && !f.id.startsWith("my_") && !(f.verified && f.applies_to.length));
  const groups: any[] = [];
  const byCat = (list: DiagFlow[], tag: string) => {
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
        key: `${tag}-${cat}`,
        children: bucket.map((f) => ({ label: (f.verified ? "" : "●") + f.name, value: f.id })),
      });
    }
  };
  byCat(recommended, "推荐检查");
  byCat(standard, "标准测试用例");
  byCat(mine, "我的流程");
  byCat(examples, "手册示例（需另存）");
  return groups.filter((g) => g.children.length);
});

const selectedScFlow = computed(() =>
  store.knowledge?.flows.find((flow) => flow.id === scFlowId.value)
);

const scenarioBlockReason = computed(() => {
  const flow = selectedScFlow.value;
  if (!flow || flow.kind !== "usecase") return "";
  if (!flow.id.startsWith("my_") && !flow.verified) {
    return "手册示例不能直接用于重复压测，请先“另存”为我的流程并核对参数。";
  }
  if (!flow.applies_to.length) return "该流程未标适用型号，请编辑后补齐再运行。";
  if (flow.execution_mode === "unknown") return "该流程的执行影响范围尚未确认，请编辑后选择只读或可能修改设备。";
  const hasPlaceholder = flow.steps.some((step) =>
    step.send.some((cmd) => /<[^>]+>|\{\{[^}]+\}\}/.test(cmd))
  );
  if (hasPlaceholder) return "流程中仍有待填写参数，请另存或编辑并替换占位内容。";
  return "";
});

function selectScenarioFlow(value: string | null) {
  scFlowId.value = value ?? "";
  scFlowSelectOpen.value = false;
}

const scProgress = computed(() => {
  if (scDurationMode.value === "time") {
    const planned = (scHours.value ?? 0) * 3600 * 1000;
    if (planned <= 0) return 0;
    return Math.min(100, (elapsedMs.value / planned) * 100);
  }
  const planned = scIterations.value ?? 0;
  return planned > 0 ? Math.min(100, (completed.value / planned) * 100) : 0;
});

// ---- shared computeds ----
const progress = computed(() =>
  mode.value === "scenario" ? scProgress.value
    : (iterations.value ?? 0) > 0 ? Math.min(100, (completed.value / (iterations.value ?? 1)) * 100) : 0
);
const plannedDisplay = computed(() => {
  if (mode.value === "single") return (iterations.value ?? 0).toLocaleString();
  if (scDurationMode.value === "time") return `${scHours.value ?? 0}h`;
  return (scIterations.value ?? 0).toLocaleString();
});
const successRate = computed(() =>
  completed.value ? `${((passed.value / completed.value) * 100).toFixed(2)}%` : "-"
);
const elapsedMs = computed(() => {
  if (mode.value === "scenario") {
    return scReport.value?.elapsed_ms ?? (running.value && startedAt.value ? now.value - startedAt.value : 0);
  }
  return report.value?.elapsed_ms ?? (running.value && startedAt.value ? now.value - startedAt.value : 0);
});
const statusText = computed(() => {
  if (stopping.value) return "正在停止";
  if (running.value) return "运行中";
  if (mode.value === "scenario") {
    const r = scReport.value;
    if (!r) return "准备就绪";
    if (r.cancelled) return "已停止";
    if (r.stopped_on_failure) return "失败后停止";
    if (r.time_limit_reached) return "到达时长";
    return r.failed ? "已完成，有失败" : "已完成";
  }
  if (!report.value) return "准备就绪";
  if (report.value.cancelled) return "已停止";
  if (report.value.stopped_on_failure) return "失败后停止";
  return report.value.failed ? "已完成，有失败" : "已完成";
});

/** 失败聚类结论：一句话总结主要失败模式 */
const failureConclusion = computed(() => {
  const r = scReport.value;
  if (!r || !r.failed) return "";
  if (!r.failure_clusters.length) return `${r.failed} 次失败，未归类到具体步骤`;
  const top = r.failure_clusters[0];
  const pct = Math.round((top.count / r.failed) * 100);
  return `${r.failed} 次失败中 ${top.count} 次（${pct}%）发生在「${top.step_title || top.step_id}」步骤，错误类型：${top.error_kind}`;
});

let unlisten1: UnlistenFn | null = null;
let unlisten2: UnlistenFn | null = null;
let clock = 0;
onMounted(async () => {
  await store.init();
  watch(
    () => portOptions.value,
    (opts) => {
      if (!opts.some((o) => o.value === port.value)) {
        port.value = store.activePort || opts[0]?.value || "";
      }
    },
    { immediate: true }
  );
  watch(() => stressProfile.value?.id, () => { if (mode.value === "scenario") refreshScApplicable(); });
  unlisten1 = await listen<StressProgressEvent>("stress://progress", (event) => {
    if (event.payload.run_id !== runId.value) return;
    completed.value = event.payload.completed;
    passed.value = event.payload.passed;
    failed.value = event.payload.failed;
    if (mode.value === "single") {
      recentSamples.value = [event.payload.sample, ...recentSamples.value].slice(0, 20);
    }
  });
  unlisten2 = await listen<ScenarioRoundEvent>("stress://round", (event) => {
    if (event.payload.run_id !== runId.value) return;
    completed.value = event.payload.passed + event.payload.failed;
    passed.value = event.payload.passed;
    failed.value = event.payload.failed;
    if (mode.value === "scenario") {
      scRounds.value = [event.payload.round, ...scRounds.value].slice(0, 30);
      scTrend.value.push({ i: event.payload.round.index, ms: event.payload.round.elapsed_ms, ok: event.payload.round.success });
      // 长稳模式点数无界增长会导致趋势图全量重算、内存持续升高。
      // 超过上限时按偶数下标减半降采样，保留近期分辨率，点数始终有界。
      if (scTrend.value.length > 2000) {
        scTrend.value = scTrend.value.filter((_, idx) => idx % 2 === 0);
      }
    }
  });
  clock = window.setInterval(() => (now.value = Date.now()), 250);
});
onUnmounted(() => {
  unlisten1?.();
  unlisten2?.();
  window.clearInterval(clock);
});

watch(mode, (m) => {
  scFlowSelectOpen.value = false;
  if (m === "scenario") refreshScApplicable();
});

function applyPreset(value: string) {
  preset.value = value;
  const item = presets.find((candidate) => candidate.value === value);
  if (!item || value === "custom") return;
  command.value = item.command;
  expectedPattern.value = item.pattern;
}

function fmtDuration(ms: number) {
  if (ms < 1000) return `${ms} ms`;
  const seconds = Math.floor(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  if (hours) return `${hours}时 ${minutes % 60}分`;
  return minutes ? `${minutes}分 ${seconds % 60}秒` : `${seconds}秒`;
}

function resetCounters() {
  completed.value = 0;
  passed.value = 0;
  failed.value = 0;
  recentSamples.value = [];
  scRounds.value = [];
  scTrend.value = [];
  report.value = null;
  scReport.value = null;
}

async function startSingle() {
  if (!port.value) { message.warning("请先在控制台打开一个串口会话"); return; }
  if (!command.value.trim()) { message.warning("请输入 AT 指令"); return; }
  resetCounters();
  runId.value = crypto.randomUUID();
  startedAt.value = Date.now();
  now.value = startedAt.value;
  running.value = true;
  stopping.value = false;
  try {
    const target = store.executionTarget(port.value);
    report.value = await invoke<StressReport>("run_stress_test", {
      port: target.port, sessionGeneration: target.sessionGeneration, runId: runId.value,
      config: {
        command: command.value, iterations: iterations.value, intervalMs: intervalMs.value,
        timeoutMs: timeoutMs.value, expectedPattern: expectedPattern.value || null,
        stopOnFailure: stopOnFailure.value,
      },
    });
    completed.value = report.value.completed;
    passed.value = report.value.passed;
    failed.value = report.value.failed;
    if (report.value.cancelled) message.info("压力测试已停止");
    else if (report.value.failed) message.warning(`测试完成，失败 ${report.value.failed} 次`);
    else message.success("压力测试全部通过");
  } catch (error) { if (String(error) === "CANCELLED") message.info("已取消"); else message.error(String(error)); }
  finally { running.value = false; stopping.value = false; }
}

function confirmUnverifiedFlow(): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false;
    const done = (v: boolean) => { if (!settled) { settled = true; resolve(v); } };
    dialog.warning({
      title: "该用例未经实机复核",
      content: "该流程尚未完成实机复核。请确认已检查服务器、APN、证书等现场参数，并了解流程可能修改设备状态。",
      positiveText: "继续运行",
      negativeText: "取消",
      onPositiveClick: () => done(true),
      onNegativeClick: () => done(false),
      onMaskClick: () => done(false),
      onClose: () => done(false),
    });
  });
}
async function startScenario() {
  if (!port.value) { message.warning("请先在控制台打开一个串口会话"); return; }
  if (!scFlowId.value) { message.warning("请选择一个测试用例/流程"); return; }
  if (scenarioBlockReason.value) { message.warning(scenarioBlockReason.value); return; }
  // 未复核用例（自动转换生成）可能包含修改设备状态的高危指令或占位凭据，
  // 首次运行前要求用户明确确认。
  const flow = store.knowledge?.flows.find((f) => f.id === scFlowId.value);
  let allowUnverified = false;
  if (flow && !flow.verified) {
    const ok = await confirmUnverifiedFlow();
    if (!ok) return;
    allowUnverified = true;
  }
  resetCounters();
  runId.value = crypto.randomUUID();
  startedAt.value = Date.now();
  now.value = startedAt.value;
  running.value = true;
  stopping.value = false;
  const recoveryCommands = scFailurePolicy.value !== "recover"
    ? null
    : scRecoveryAuto.value ? null : scRecoveryText.value.split("\n").map((s) => s.trim()).filter(Boolean);
  try {
    const target = store.executionTarget(port.value);
    scReport.value = await invoke<ScenarioStressReport>("run_scenario_stress_test", {
      port: target.port, sessionGeneration: target.sessionGeneration, runId: runId.value, flowId: scFlowId.value,
      profileId: stressProfile.value?.id ?? null,
      allowUnverified,
      config: {
        iterations: scDurationMode.value === "time" ? 0 : scIterations.value,
        intervalMs: scInterval.value,
        failurePolicy: scFailurePolicy.value,
        recoveryCommands,
        recoveryTimeoutMs: 2000,
        warnAsPass: scWarnAsPass.value,
        maxDurationSec: scDurationMode.value === "time" ? Math.round(scHours.value * 3600) : 0,
      },
    });
    completed.value = scReport.value.completed;
    passed.value = scReport.value.passed;
    failed.value = scReport.value.failed;
    if (scReport.value.cancelled) message.info("场景压测已停止");
    else if (scReport.value.time_limit_reached) message.success("长稳测试到达设定时长");
    else if (scReport.value.failed) message.warning(`完成，失败 ${scReport.value.failed} 次`);
    else message.success("场景压测全部通过");
  } catch (error) { if (String(error) === "CANCELLED") message.info("已取消"); else message.error(String(error)); }
  finally { running.value = false; stopping.value = false; }
}

function start() { mode.value === "single" ? startSingle() : startScenario(); }

async function stop() {
  if (!running.value || !runId.value) return;
  stopping.value = true;
  await invoke("cancel_stress_test", { runId: runId.value });
}

function esc(value: string) {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

async function exportReport() {
  if (mode.value === "scenario") { exportScenario(); return; }
  if (!report.value) return;
  const path = await saveDialog({
    defaultPath: `压力测试_${port.value}_${new Date().toISOString().slice(0, 10)}.html`,
    filters: [{ name: "HTML 报告", extensions: ["html"] }],
  });
  if (!path) return;
  const r = report.value;
  const failures = r.failures.map((sample) => `<tr><td>${sample.index}</td><td>${sample.elapsed_ms} ms</td><td>${esc(sample.final_code)}</td><td>${esc(sample.error)}</td><td><pre>${esc(sample.lines.join("\n"))}</pre></td></tr>`).join("");
  const html = `<!doctype html><html lang="zh"><head><meta charset="utf-8"><title>压力测试报告</title><style>body{font-family:"Segoe UI","Microsoft YaHei",sans-serif;max-width:960px;margin:32px auto;padding:0 20px;color:#222}h1{font-size:22px}.meta{color:#666;margin-bottom:18px}.metrics{display:flex;gap:24px;flex-wrap:wrap;padding:14px 0;border-block:1px solid #ddd}.metrics b{display:block;font-size:20px}table{width:100%;border-collapse:collapse;margin-top:18px}th,td{border-bottom:1px solid #ddd;padding:8px;text-align:left;vertical-align:top}pre{white-space:pre-wrap;word-break:break-all;margin:0;font-size:12px}</style></head><body>
<h1>AT 压力测试报告</h1><div class="meta">端口：${esc(port.value)} ｜ 指令：${esc(r.command)} ｜ 时间：${new Date().toLocaleString("zh-CN")}</div>
<div class="metrics"><span><b>${r.completed}/${r.planned}</b>完成</span><span><b>${r.passed}</b>通过</span><span><b>${r.failed}</b>失败</span><span><b>${r.avg_latency_ms.toFixed(1)} ms</b>平均延迟</span><span><b>${r.p95_latency_ms} ms</b>P95 延迟</span><span><b>${fmtDuration(r.elapsed_ms)}</b>总耗时</span></div>
<h2>失败明细</h2>${failures ? `<table><thead><tr><th>轮次</th><th>延迟</th><th>终结码</th><th>原因</th><th>响应</th></tr></thead><tbody>${failures}</tbody></table>` : "<p>无失败记录</p>"}
</body></html>`;
  await invoke("save_text_file", { path, content: html });
  message.success("压力测试报告已导出");
}

async function exportScenario() {
  if (!scReport.value) return;
  const path = await saveDialog({
    defaultPath: `场景压测_${scReport.value.flow_id}_${new Date().toISOString().slice(0, 10)}.html`,
    filters: [{ name: "HTML 报告", extensions: ["html"] }],
  });
  if (!path) return;
  const r = scReport.value;
  const stepRows = r.step_stats.map((s) => `<tr><td>${esc(s.step_title)}</td><td>${s.count}</td><td>${s.min_ms}</td><td>${s.avg_ms.toFixed(0)}</td><td>${s.max_ms}</td><td>${s.p95_ms}</td></tr>`).join("");
  const clusterRows = r.failure_clusters.map((c) => `<tr><td>${esc(c.step_title)}</td><td>${esc(c.error_kind)}</td><td>${c.count}</td><td>${c.sample_rounds.join(", ")}</td></tr>`).join("");
  const failRows = r.failures.map((f) => `<tr><td>${f.index}</td><td>${esc(f.overall)}</td><td>${f.elapsed_ms} ms</td><td>${f.steps.map((s) => esc(s.title + ":" + s.reason)).join("<br>")}</td></tr>`).join("");
  const html = `<!doctype html><html lang="zh"><head><meta charset="utf-8"><title>场景压测报告</title><style>body{font-family:"Segoe UI","Microsoft YaHei",sans-serif;max-width:960px;margin:32px auto;padding:0 20px;color:#222}h1{font-size:22px}h2{margin-top:28px}.meta{color:#666}.metrics{display:flex;gap:24px;flex-wrap:wrap;padding:14px 0;border-block:1px solid #ddd}.metrics b{display:block;font-size:20px}.concl{padding:12px 16px;background:#fff7e6;border-radius:8px;margin:14px 0}table{width:100%;border-collapse:collapse}th,td{border-bottom:1px solid #ddd;padding:8px;text-align:left}pre{white-space:pre-wrap}</style></head><body>
<h1>场景压测报告</h1><div class="meta">用例：${esc(r.flow_name)} ｜ 端口：${esc(port.value)} ｜ 时间：${new Date().toLocaleString("zh-CN")}</div>
<div class="metrics"><span><b>${r.completed}/${r.planned}</b>完成</span><span><b>${r.passed}</b>通过</span><span><b>${r.failed}</b>失败</span><span><b>${successRate.value}</b>成功率</span><span><b>${r.avg_latency_ms.toFixed(0)} ms</b>平均</span><span><b>${r.p95_latency_ms} ms</b>P95</span><span><b>${fmtDuration(r.elapsed_ms)}</b>总耗时</span></div>
${failureConclusion.value ? `<div class="concl"><b>结论：</b>${esc(failureConclusion.value)}</div>` : ""}
<h2>分步延迟分解</h2><table><thead><tr><th>步骤</th><th>次数</th><th>最小</th><th>平均</th><th>最大</th><th>P95</th></tr></thead><tbody>${stepRows}</tbody></table>
<h2>失败聚类</h2>${clusterRows ? `<table><thead><tr><th>步骤</th><th>错误类型</th><th>次数</th><th>样本轮次</th></tr></thead><tbody>${clusterRows}</tbody></table>` : "<p>无失败</p>"}
<h2>失败明细（最多 100 条）</h2>${failRows ? `<table><thead><tr><th>轮次</th><th>结果</th><th>耗时</th><th>失败步骤</th></tr></thead><tbody>${failRows}</tbody></table>` : "<p>无失败记录</p>"}
</body></html>`;
  await invoke("save_text_file", { path, content: html });
  message.success("场景压测报告已导出");
}
</script>

<template>
  <div class="stress">
    <header class="stress-head">
      <div>
        <h2>稳定性验证</h2>
        <p class="sub">单指令通道稳定性 · 已复核场景循环 · 长稳模式</p>
      </div>
      <!-- 操作按钮固定在页头：任何模式、任何滚动位置都可见 -->
      <div class="head-actions">
        <n-tag size="small" :type="running ? 'warning' : 'default'" :bordered="false">{{ statusText }}</n-tag>
        <n-button v-if="(mode === 'single' ? report : scReport) && !running" size="small" secondary @click="exportReport">导出报告</n-button>
        <n-button v-if="!running" type="primary" :disabled="!portOptions.length || (mode === 'scenario' && (!scFlowId || !!scenarioBlockReason))" @click="start">
          <template #icon><AppIcon :icon="Play" :size="14" /></template>
          {{ mode === "single" ? "开始测试" : "开始压测" }}
        </n-button>
        <n-button v-else type="error" :loading="stopping" @click="stop">
          <span class="btn-label-row">
            <AppIcon :icon="Square" :size="13" />
            <span class="gap-4">停止</span>
          </span>
        </n-button>
      </div>
    </header>

    <section class="control-panel">
      <div class="mode-row">
        <n-tabs v-model:value="mode" type="segment" size="small" class="mode-tabs">
          <n-tab name="single" :disabled="running">单指令重复</n-tab>
          <n-tab name="scenario" :disabled="running">场景序列（多步用例）</n-tab>
        </n-tabs>
        <label class="port-pick"><span>串口会话</span>
          <n-select v-model:value="port" :options="portOptions" size="small" placeholder="选择端口" />
        </label>
        <div class="session-summary">
          <span class="session-dot" :class="{ online: !!port }" />
          {{ port ? (stressProfile?.name || "未识别画像") : "等待连接" }}
        </div>
      </div>

      <!-- 单指令模式 -->
      <div v-if="mode === 'single'" class="setup">
        <label><span>预设</span>
          <n-select v-model:value="preset" :options="presets" size="small" @update:value="applyPreset" />
        </label>
        <label class="command"><span>AT 指令</span>
          <n-input v-model:value="command" :disabled="running" placeholder="如 AT+CSQ" />
        </label>
        <label><span>测试次数</span>
          <n-input-number v-model:value="iterations" :min="1" :max="100000" :step="100" :disabled="running" />
        </label>
        <label><span>轮间隔</span>
          <n-input-number v-model:value="intervalMs" :min="0" :max="60000" :step="100" :disabled="running"><template #suffix>ms</template></n-input-number>
        </label>
        <label><span>超时</span>
          <n-input-number v-model:value="timeoutMs" :min="100" :max="120000" :step="500" :disabled="running"><template #suffix>ms</template></n-input-number>
        </label>
        <label class="pattern"><span>响应成功条件</span>
          <n-input v-model:value="expectedPattern" :disabled="running" placeholder="可选正则，如 \\+CSQ:" />
        </label>
        <div class="run-options">
          <n-checkbox v-model:checked="stopOnFailure" :disabled="running">首次失败后停止</n-checkbox>
        </div>
      </div>

      <!-- 场景模式 -->
      <div v-else class="scenario-layout">
        <div class="scenario-flow-block">
          <label class="field-label">验证流程</label>
          <div class="sc-flow-row">
            <n-select
              :value="scFlowId"
              :show="scFlowSelectOpen"
              :options="scFlowOptions"
              size="small"
              filterable
              clearable
              :disabled="running"
              placeholder="选择推荐检查、标准用例或我的流程"
              @update:show="scFlowSelectOpen = $event"
              @update:value="selectScenarioFlow"
            >
              <template #action>
                <button type="button" class="select-collapse" @click="scFlowSelectOpen = false">收起列表</button>
              </template>
            </n-select>
            <n-button size="small" secondary :disabled="running" @click="openEditor('create')">＋ 新建</n-button>
            <n-button v-if="scFlowId" size="small" secondary :disabled="running" @click="openEditor('clone')">另存</n-button>
            <n-button v-if="scFlowId && scFlowId.startsWith('my_')" size="small" secondary :disabled="running" @click="openEditor('edit')">编辑</n-button>
          </div>
          <div v-if="selectedScFlow" class="flow-summary">
            <span>{{ selectedScFlow.category ? (CATEGORY_LABELS[selectedScFlow.category] ?? selectedScFlow.category) : "未分类" }}</span>
            <span>{{ selectedScFlow.steps.length }} 步</span>
            <span :class="selectedScFlow.verified ? 'verified' : 'unverified'">{{ selectedScFlow.verified ? "已复核" : "未复核" }}</span>
            <span :class="selectedScFlow.execution_mode === 'readonly' ? 'verified' : selectedScFlow.execution_mode === 'mutating' ? 'unverified' : ''">
              {{ selectedScFlow.execution_mode === "readonly" ? "只读" : selectedScFlow.execution_mode === "mutating" ? "可能改设备" : "影响范围未标注" }}
            </span>
          </div>
          <n-alert v-if="scenarioBlockReason" type="warning" :bordered="false" class="flow-warning">
            {{ scenarioBlockReason }}
          </n-alert>
          <FlowStepList v-if="selectedScFlow" :flow="selectedScFlow" :disabled="running" @saved="onFlowSaved" />
        </div>

        <div class="scenario-params">
          <label class="duration-option"><span>运行方式</span>
            <n-radio-group v-model:value="scDurationMode" size="small" :disabled="running" class="duration-switch">
              <n-radio-button value="count">按轮数</n-radio-button>
              <n-radio-button value="time">长稳时长</n-radio-button>
            </n-radio-group>
          </label>
          <label><span>{{ scDurationMode === 'count' ? '测试轮数' : '持续时长' }}</span>
            <n-input-number v-if="scDurationMode === 'count'" v-model:value="scIterations" :min="1" :max="10000" :step="50" :disabled="running" />
            <n-input-number v-else v-model:value="scHours" :min="0.5" :max="24" :step="0.5" :disabled="running"><template #suffix>小时</template></n-input-number>
          </label>
          <label><span>轮间隔</span>
            <n-input-number v-model:value="scInterval" :min="0" :max="60000" :step="100" :disabled="running"><template #suffix>ms</template></n-input-number>
          </label>
          <label><span>失败策略</span>
            <n-select v-model:value="scFailurePolicy" :options="[
              { label: '恢复后继续', value: 'recover' },
              { label: '继续（不恢复）', value: 'continue' },
              { label: '停止', value: 'stop' },
            ]" size="small" :disabled="running" />
          </label>
          <label class="warn-option"><span>结果判定</span>
            <n-checkbox v-model:checked="scWarnAsPass" :disabled="running">warn 计为成功</n-checkbox>
          </label>
        </div>

        <div v-if="scFailurePolicy === 'recover'" class="recovery-block">
          <span class="field-label">恢复指令</span>
          <div class="recovery-controls">
            <n-radio-group v-model:value="scRecoveryAuto" size="small" :disabled="running">
              <n-radio-button :value="true">自动推断</n-radio-button>
              <n-radio-button :value="false">手动</n-radio-button>
            </n-radio-group>
            <n-input v-if="!scRecoveryAuto" v-model:value="scRecoveryText" type="textarea" :autosize="{ minRows: 1, maxRows: 3 }" :disabled="running" placeholder="每行一条，如 AT+MIPCLOSE=0" />
          </div>
        </div>
      </div>
    </section>

    <div class="progress-line">
      <div class="progress-caption"><span>执行进度</span><b>{{ progress.toFixed(0) }}%</b></div>
      <div class="progress-track"><span :style="{ width: `${progress}%` }" /></div>
    </div>
    <section class="metrics">
      <div><b>{{ completed.toLocaleString() }}</b><span>已完成 / {{ plannedDisplay }}</span></div>
      <div class="pass"><b>{{ passed.toLocaleString() }}</b><span>通过</span></div>
      <div class="fail"><b>{{ failed.toLocaleString() }}</b><span>失败</span></div>
      <div><b>{{ successRate }}</b><span>成功率</span></div>
      <div><b>{{ (mode === 'single' ? report?.avg_latency_ms : scReport?.avg_latency_ms)?.toFixed(1) ?? '-' }} ms</b><span>平均延迟</span></div>
      <div><b>{{ (mode === 'single' ? report?.p95_latency_ms : scReport?.p95_latency_ms) ?? '-' }} ms</b><span>P95 延迟</span></div>
      <div><b>{{ fmtDuration(elapsedMs) }}</b><span>总耗时</span></div>
    </section>

    <!-- 场景模式：失败聚类结论 + 分步延迟 -->
    <template v-if="mode === 'scenario'">
      <section v-if="failureConclusion" class="conclusion">
        <b>结论</b> {{ failureConclusion }}
      </section>
      <TrendChart v-if="scTrend.length > 1" :data="scTrend" />
      <section v-if="scReport && scReport.step_stats.length" class="step-stats">
        <div class="section-head"><b>分步延迟分解</b><span>定位慢在哪一步</span></div>
        <table>
          <thead><tr><th>步骤</th><th>次数</th><th>最小</th><th>平均</th><th>最大</th><th>P95</th></tr></thead>
          <tbody>
            <tr v-for="s in scReport.step_stats" :key="s.step_id">
              <td>{{ s.step_title }}</td><td>{{ s.count }}</td><td>{{ s.min_ms }}</td>
              <td>{{ s.avg_ms.toFixed(0) }}</td><td>{{ s.max_ms }}</td><td>{{ s.p95_ms }}</td>
            </tr>
          </tbody>
        </table>
      </section>
      <section v-if="scReport && scReport.failure_clusters.length" class="step-stats">
        <div class="section-head"><b>失败聚类</b><span>按步骤+错误类型分组</span></div>
        <table>
          <thead><tr><th>步骤</th><th>错误类型</th><th>次数</th><th>样本轮次</th></tr></thead>
          <tbody>
            <tr v-for="(c, i) in scReport.failure_clusters" :key="i">
              <td>{{ c.step_title || c.step_id }}</td><td>{{ c.error_kind }}</td><td>{{ c.count }}</td>
              <td>{{ c.sample_rounds.join(', ') }}</td>
            </tr>
          </tbody>
        </table>
      </section>
    </template>

    <section class="activity">
      <div class="section-head"><b>运行动态</b>
        <span v-if="mode === 'single' && report">延迟范围 {{ report.min_latency_ms }}–{{ report.max_latency_ms }} ms</span>
      </div>

      <div v-if="mode === 'single' && recentSamples.length" class="sample-list">
        <div v-for="sample in recentSamples" :key="`${sample.index}-${sample.elapsed_ms}`" class="sample" :class="{ bad: !sample.success }">
          <span>#{{ sample.index }}</span><b>{{ sample.success ? "通过" : "失败" }}</b><span>{{ sample.elapsed_ms }} ms</span><code>{{ sample.final_code }}</code><span class="sample-detail">{{ sample.error || sample.lines.join(" · ") || "OK" }}</span>
        </div>
      </div>
      <div v-else-if="mode === 'scenario' && scRounds.length" class="sample-list">
        <div v-for="r in scRounds" :key="r.index" class="sample" :class="{ bad: !r.success }">
          <span>#{{ r.index }}</span><b>{{ r.success ? "通过" : "失败" }}</b><span>{{ r.elapsed_ms }} ms</span><code>{{ r.overall }}</code><span class="sample-detail">{{ r.steps.map((s) => s.title + (s.reason ? ":" + s.reason : "")).join(" · ") || "OK" }}</span>
        </div>
      </div>
      <div v-else class="empty">暂无运行数据</div>
    </section>

    <UsecaseEditor v-model:show="editorShow" :mode="editorMode" :base-flow="editorBase" @saved="onEditorSaved" />
  </div>
</template>

<style scoped>
.stress { flex: 1; min-width: 0; overflow-y: auto; padding: 20px 28px 32px; }
.stress-head { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 16px; max-width: 1180px; margin-bottom: 16px; }
h2 { font-size: 18px; }
.sub { margin-top: 4px; font-size: 13px; opacity: 0.6; }
.head-actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.control-panel { max-width: 1180px; padding: 16px 18px 18px; border: 1px solid var(--border-subtle); border-radius: 8px; background: var(--surface-raised); }
.mode-row { display: flex; align-items: flex-end; gap: 16px; margin-bottom: 16px; padding-bottom: 14px; border-bottom: 1px solid var(--border-subtle); flex-wrap: wrap; }
.mode-tabs { width: 320px; flex-shrink: 0; }
.port-pick { display: flex; flex-direction: column; gap: 6px; width: 180px; }
.port-pick > span { font-size: 12px; opacity: 0.65; }
.session-summary { display: flex; align-items: center; gap: 7px; min-height: 34px; margin-left: auto; font-size: 12px; opacity: .68; }
.session-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-tertiary); box-shadow: 0 0 0 3px var(--surface-hover); }
.session-dot.online { background: var(--status-success); box-shadow: 0 0 0 3px var(--status-success-soft); }
.setup-row { display: flex; gap: 12px; margin-bottom: 12px; }
.setup-row label { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.setup-row label > span { font-size: 12px; opacity: 0.65; }
.setup { display: grid; grid-template-columns: repeat(4, minmax(140px, 1fr)); gap: 14px 16px; }
.setup label { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.setup label > span { font-size: 12px; opacity: 0.65; }
.command, .pattern { grid-column: span 2; }
.scenario-layout { display: flex; flex-direction: column; gap: 16px; }
.scenario-flow-block { min-width: 0; }
.field-label { display: block; margin-bottom: 6px; font-size: 12px; opacity: .65; }
.sc-flow-row { display: flex; gap: 6px; align-items: center; }
.sc-flow-row .n-select { flex: 1; min-width: 0; }
.select-collapse { width: 100%; padding: 6px 10px; border: 0; background: transparent; color: var(--brand-primary); text-align: center; cursor: pointer; font: inherit; }
.select-collapse:hover { background: rgba(76,141,255,.1); }
.flow-summary { display: flex; gap: 12px; min-height: 18px; padding: 1px 2px 0; font-size: 11px; opacity: .62; }
.flow-summary .verified { color: var(--status-success); opacity: 1; }
.flow-summary .unverified { color: var(--status-warning); opacity: 1; }
.scenario-params { display: grid; grid-template-columns: minmax(180px, 1.3fr) repeat(4, minmax(120px, 1fr)); gap: 14px 16px; padding-top: 15px; border-top: 1px solid var(--border-subtle); }
.scenario-params label { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.scenario-params label > span { font-size: 12px; opacity: .65; }
.recovery-block { padding-top: 15px; border-top: 1px solid var(--border-subtle); }
.recovery-controls { display: flex; align-items: flex-start; gap: 10px; }
.recovery-controls .n-input { flex: 1; min-width: 0; }
.duration-switch { display: grid !important; grid-template-columns: repeat(2, minmax(0, 1fr)); width: 100%; }
.duration-switch :deep(.n-radio-button) { width: 100%; min-width: 0; justify-content: center; }
.duration-switch :deep(.n-radio-button__label) { width: 100%; padding-inline: 8px; overflow: hidden; text-align: center; text-overflow: ellipsis; white-space: nowrap; }
.recovery-wrap { display: flex; flex-direction: column; gap: 6px; }
.warn-option { justify-content: flex-start; }
.warn-option .n-checkbox { min-height: 34px; align-items: center; }
.run-options { display: flex; align-items: flex-end; justify-content: space-between; gap: 12px; padding-bottom: 4px; flex-wrap: wrap; }
.run-options .hint { font-size: 11px; opacity: 0.45; }
.progress-line { max-width: 1180px; margin: 18px 0 10px; }
.progress-caption { display: flex; justify-content: space-between; align-items: baseline; margin-bottom: 7px; font-size: 11px; opacity: .58; }
.progress-caption b { font-variant-numeric: tabular-nums; font-weight: 600; }
.status-tag { display: inline-block; font-size: 12px; opacity: .6; margin-bottom: 6px; }
.progress-track { height: 5px; overflow: hidden; border-radius: 3px; background: var(--border-subtle); }
.progress-track span { display: block; height: 100%; border-radius: inherit; background: var(--status-success); transition: width .2s ease; }
.metrics { display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); max-width: 1180px; margin-top: 14px; padding: 0 14px; border: 1px solid var(--border-subtle); border-radius: 8px; background: var(--surface-raised); }
.metrics > div { position: relative; min-width: 0; padding: 15px 12px; }
.metrics > div + div::before { position: absolute; top: 15px; bottom: 15px; left: 0; width: 1px; background: var(--border-subtle); content: ""; }
.metrics b { display: block; font-size: 19px; font-weight: 650; }
.metrics span { display: block; margin-top: 2px; font-size: 12px; opacity: .6; }
.metrics .pass b { color: var(--status-success); }
.metrics .fail b { color: var(--status-danger); }
.conclusion { max-width: 1180px; margin: 14px 0; padding: 12px 16px; border-left: 3px solid var(--status-warning); background: var(--status-warning-soft); color: var(--status-warning); font-size: 13px; }
.conclusion b { color: var(--status-warning); }
.step-stats { max-width: 1180px; margin-top: 20px; }
.step-stats table { width: 100%; border-collapse: collapse; font-size: 12px; }
.step-stats th, .step-stats td { border-bottom: 1px solid var(--border-subtle); padding: 7px 8px; text-align: left; color: var(--text-primary); }
.step-stats th { opacity: 0.55; font-weight: 500; }
.step-stats td:nth-child(n+2) { font-variant-numeric: tabular-nums; }
.activity { max-width: 1180px; margin-top: 22px; }
.section-head { display: flex; justify-content: space-between; gap: 12px; margin-bottom: 8px; align-items: center; }
.section-head span { font-size: 12px; opacity: 0.55; }
.sample-list { overflow: hidden; border: 1px solid var(--border-subtle); border-radius: 8px; }
.sample { display: grid; grid-template-columns: 58px 48px 72px 90px minmax(0,1fr); gap: 10px; align-items: baseline; padding: 8px 10px; border-bottom: 1px solid var(--border-subtle); font-size: 12px; color: var(--text-secondary); }
.sample:last-child { border-bottom: 0; }
.sample:nth-child(even) { background: var(--surface-hover); }
.sample b { color: var(--status-success); }
.sample.bad b, .sample.bad code { color: var(--status-danger); }
.sample code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sample-detail { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-tertiary); }
.empty { padding: 36px 0; text-align: center; opacity: .45; }
@media (max-width: 900px) {
  .stress { padding: 16px 18px 28px; }
  .setup { grid-template-columns: repeat(2, minmax(150px, 1fr)); }
  .command, .pattern { grid-column: span 2; }
  .scenario-params { grid-template-columns: repeat(2, minmax(150px, 1fr)); }
  .metrics { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  .metrics > div:nth-child(5)::before { display: none; }
  .sample { grid-template-columns: 50px 42px 64px minmax(0,1fr); }
  .sample-detail { grid-column: 1 / -1; }
}
@media (max-width: 620px) {
  .stress { padding: 14px 12px 24px; }
  .stress-head { align-items: flex-start; }
  .head-actions { width: 100%; }
  .control-panel { padding: 12px; }
  .mode-tabs, .port-pick { width: 100%; }
  .session-summary { width: 100%; margin-left: 0; }
  .setup { grid-template-columns: minmax(0, 1fr); }
  .command, .pattern { grid-column: span 1; }
  .scenario-params { grid-template-columns: minmax(0, 1fr); }
  .sc-flow-row { flex-wrap: wrap; }
  .sc-flow-row .n-select { flex-basis: 100%; }
  .recovery-controls { flex-direction: column; }
  .recovery-controls .n-radio-group, .recovery-controls .n-input { width: 100%; }
  .metrics { grid-template-columns: repeat(2, minmax(0, 1fr)); padding: 0 8px; }
  .metrics > div:nth-child(odd)::before { display: none; }
  .sample { grid-template-columns: 42px 42px minmax(0,1fr); }
  .sample code, .sample-detail { grid-column: 1 / -1; }
}

/* === 工具类 === */
.gap-4 { margin-left: var(--space-1); }
.btn-label-row { display: inline-flex; align-items: center; }
.btn-label-row > :not(:first-child) { margin-left: var(--space-1); }
</style>
