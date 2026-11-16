<script setup lang="ts">
import {
  NButton,
  NInput,
  NRadioButton,
  NRadioGroup,
  NSelect,
  useMessage,
} from "naive-ui";
import { ref, computed, onMounted, onActivated } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronRight } from "@lucide/vue";
import AppIcon from "../components/AppIcon.vue";
import { useSerialStore } from "../stores/serial";
import { useExecutionStore } from "../stores/execution";
import type { LogAnalysisReport } from "../types";

const store = useSerialStore();
const execStore = useExecutionStore();
const message = useMessage();

const report = ref<LogAnalysisReport | null>(null);
const analyzing = ref(false);
const sourceMode = ref<"serial" | "file">("serial");
const atPort = ref("");
const logPort = ref("");
const filePath = ref("");
const expandedAnomalies = ref<Set<number>>(new Set());
const expandedEvents = ref<Set<number>>(new Set());

onMounted(async () => {
  await store.init();
});

onActivated(() => {
  const pendingPort = execStore.consumeLogPort();
  if (pendingPort) atPort.value = pendingPort;
});

const portOptions = computed(() =>
  store.sessions.filter((s) => s.connected).map((s) => ({
    label: `${s.port} @${s.baud}`,
    value: s.port,
  }))
);

const logPortOptions = computed(() => {
  const grouped = store.deviceGroups.flatMap((g) => (g.logPort ? [g.logPort] : []));
  return store.sessions
    .filter((s) => s.connected && grouped.includes(s.port))
    .map((s) => ({ label: `${s.port} (log)`, value: s.port }));
});

async function pickFile() {
  const path = await openDialog({
    multiple: false,
    filters: [{ name: "日志文件", extensions: ["txt", "log"] }],
  });
  if (path && typeof path === "string") {
    filePath.value = path;
  }
}

async function analyze() {
  analyzing.value = true;
  report.value = null;
  expandedAnomalies.value.clear();
  expandedEvents.value.clear();
  try {
    let port = "";
    let logPortArg: string | null = null;
    let file: string | null = null;

    if (sourceMode.value === "file") {
      if (!filePath.value) { message.warning("请选择日志文件"); return; }
      file = filePath.value;
      port = "__file__";
    } else {
      if (!atPort.value) { message.warning("请选择 AT 口"); return; }
      port = atPort.value;
      logPortArg = (logPort.value as string) || null;
    }

    report.value = await invoke<LogAnalysisReport>("analyze_log", {
      port,
      logPort: logPortArg,
      filePath: file,
    });
    if (!report.value.at && !report.value.raw) {
      message.info("未获取到日志数据");
    }
  } catch (e: any) {
    message.error(`分析失败：${e}`);
  } finally {
    analyzing.value = false;
  }
}

function toggleAnomaly(idx: number) {
  const next = new Set(expandedAnomalies.value);
  if (next.has(idx)) next.delete(idx);
  else next.add(idx);
  expandedAnomalies.value = next;
}
function toggleEvent(idx: number) {
  const next = new Set(expandedEvents.value);
  if (next.has(idx)) next.delete(idx);
  else next.add(idx);
  expandedEvents.value = next;
}

function fmtTime(ts: number) {
  if (!ts) return "-";
  return new Date(ts).toLocaleTimeString("zh-CN", { hour12: false });
}

function severityColor(sev: string) {
  return sev === "error" ? "var(--status-danger)" : sev === "warning" ? "var(--status-warning)" : "var(--text-tertiary)";
}
</script>

<template>
  <div class="log-analysis">
    <header class="la-head">
      <div>
        <h2>日志分析</h2>
        <p class="sub">解析 AT 口交互 + log 口底层事件，自动关联错误码知识库</p>
      </div>
    </header>

    <div class="la-controls">
      <n-radio-group v-model:value="sourceMode" size="small" :disabled="analyzing">
        <n-radio-button value="serial">实时串口</n-radio-button>
        <n-radio-button value="file">导入文件</n-radio-button>
      </n-radio-group>

      <template v-if="sourceMode === 'serial'">
        <n-select v-model:value="atPort" :options="portOptions" size="small" placeholder="AT 口" style="width: 160px" />
        <n-select v-model:value="logPort" :options="logPortOptions" size="small" placeholder="log 口（可选）" clearable style="width: 180px" />
      </template>
      <template v-else>
        <n-input v-model:value="filePath" size="small" placeholder="选择导出的日志文件" readonly style="width: 300px" />
        <n-button size="small" secondary @click="pickFile">浏览…</n-button>
      </template>

      <n-button type="primary" size="small" :loading="analyzing" @click="analyze">分析</n-button>
    </div>

    <div v-if="report" class="la-results">
      <!-- AT 口分析 -->
      <div v-if="report.at" class="section">
        <div class="section-head">
          <h3>AT 口分析</h3>
          <span class="summary">{{ report.at.summary }}</span>
        </div>
        <div class="stat-row">
          <div class="stat"><b>{{ report.at.total_interactions }}</b><span>交互总数</span></div>
          <div class="stat err"><b>{{ report.at.error_interactions }}</b><span>失败</span></div>
          <div class="stat ok"><b>{{ report.at.total_interactions - report.at.error_interactions }}</b><span>成功</span></div>
        </div>
        <div v-if="report.at.anomalies.length" class="anomaly-list">
          <div v-for="(a, i) in report.at.anomalies" :key="i" class="anomaly-card" :class="`ak-${a.kind}`">
            <div class="anomaly-head" @click="toggleAnomaly(i)">
              <span class="anomaly-idx">#{{ a.index }}</span>
              <span class="anomaly-req">{{ a.request }}</span>
              <span class="anomaly-kind" :style="{ color: severityColor(a.error_code?.severity ?? 'error') }">{{ a.final_code }}</span>
              <span class="anomaly-time">{{ fmtTime(a.timestamp) }}</span>
              <span class="expand">
                <AppIcon :icon="expandedAnomalies.has(i) ? ChevronDown : ChevronRight" :size="11" />
              </span>
            </div>
            <div v-if="a.error_code" class="error-code-info">
              <span class="ec-meaning">{{ a.error_code.meaning }}</span>
              <div v-if="a.error_code.causes.length" class="ec-causes">
                可能原因：{{ a.error_code.causes.join("；") }}
              </div>
              <div v-if="a.error_code.suggestions.length" class="ec-sugg">
                建议：{{ a.error_code.suggestions.join("；") }}
              </div>
            </div>
            <div v-if="expandedAnomalies.has(i)" class="anomaly-raw">
              <pre v-for="(line, li) in a.raw_lines" :key="li">{{ line }}</pre>
            </div>
          </div>
        </div>
      </div>

      <!-- log 口分析 -->
      <div v-if="report.raw" class="section">
        <div class="section-head">
          <h3>log 口分析</h3>
          <span class="summary">{{ report.raw.summary }}</span>
        </div>
        <div class="stat-row">
          <div class="stat"><b>{{ report.raw.total_lines }}</b><span>日志行数</span></div>
          <div class="stat err"><b>{{ report.raw.events.filter(e => e.severity === 'error').length }}</b><span>错误事件</span></div>
          <div class="stat warn"><b>{{ report.raw.events.filter(e => e.severity === 'warning').length }}</b><span>警告事件</span></div>
        </div>
        <div v-if="report.raw.events.length" class="event-list">
          <div v-for="(e, i) in report.raw.events" :key="i" class="event-card" :class="`ev-${e.severity}`">
            <div class="event-head" @click="toggleEvent(i)">
              <span class="event-module">{{ e.module_name }}</span>
              <span class="event-level" :style="{ color: severityColor(e.severity) }">{{ e.level.toUpperCase() }}</span>
              <span class="event-msg">{{ e.message }}</span>
              <span class="event-time">{{ fmtTime(e.timestamp) }}</span>
              <span class="expand">
                <AppIcon :icon="expandedEvents.has(i) ? ChevronDown : ChevronRight" :size="11" />
              </span>
            </div>
            <div v-if="expandedEvents.has(i)" class="event-raw">
              <pre>{{ e.raw_line }}</pre>
            </div>
          </div>
        </div>
      </div>

      <div v-if="!report.at && !report.raw" class="empty">未获取到日志数据</div>
    </div>
    <div v-else-if="!analyzing" class="empty">选择数据源后点击「分析」</div>
  </div>
</template>

<style scoped>
.log-analysis { flex: 1; min-width: 0; min-height: 0; overflow-y: auto; padding: 16px 20px; max-width: 980px; }
.la-head { margin-bottom: 16px; }
.la-head h2 { font-size: 18px; margin-bottom: 4px; color: var(--text-primary); }
.sub { font-size: 13px; color: var(--text-secondary); }
.la-controls { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-bottom: 16px; }

.la-results { display: flex; flex-direction: column; gap: 20px; }
.section { border: 1px solid var(--border-subtle); border-radius: 8px; padding: 14px; background: var(--surface-raised); }
.section-head { display: flex; align-items: baseline; gap: 12px; margin-bottom: 10px; }
.section-head h3 { font-size: 15px; color: var(--text-primary); }
.summary { font-size: 12px; color: var(--text-tertiary); }

.stat-row { display: flex; gap: 24px; margin-bottom: 14px; }
.stat { display: flex; flex-direction: column; align-items: center; }
.stat b { font-size: 22px; color: var(--text-primary); }
.stat span { font-size: 11px; color: var(--text-tertiary); }
.stat.err b { color: var(--status-danger); }
.stat.ok b { color: var(--status-success); }
.stat.warn b { color: var(--status-warning); }

.anomaly-card, .event-card { border: 1px solid var(--border-subtle); border-radius: 6px; margin-bottom: 6px; overflow: hidden; background: var(--surface-base); }
.anomaly-head, .event-head { display: flex; align-items: center; gap: 8px; padding: 8px 10px; cursor: pointer; font-size: 13px; }
.anomaly-head:hover, .event-head:hover { background: var(--surface-hover); }
.anomaly-idx { font-weight: 600; color: var(--brand-primary); min-width: 32px; }
.anomaly-req { font-family: monospace; flex: 1; color: var(--text-primary); }
.anomaly-kind { font-family: monospace; font-size: 12px; }
.anomaly-time, .event-time { font-size: 11px; color: var(--text-tertiary); }
.expand { opacity: 0.4; font-size: 10px; }

.error-code-info { padding: 6px 10px; background: var(--status-danger-soft); border-top: 1px solid var(--status-danger); font-size: 12px; }
.ec-meaning { font-weight: 600; color: var(--text-primary); }
.ec-causes, .ec-sugg { margin-top: 3px; color: var(--text-secondary); }

.anomaly-raw, .event-raw { padding: 6px 10px; border-top: 1px solid var(--border-subtle); }
.anomaly-raw pre, .event-raw pre { font-family: monospace; font-size: 12px; white-space: pre-wrap; word-break: break-all; margin: 0; color: var(--text-secondary); }

.event-module { font-weight: 600; min-width: 60px; color: var(--text-primary); }
.event-level { font-size: 11px; font-weight: 600; min-width: 40px; }
.event-msg { flex: 1; font-size: 12px; color: var(--text-secondary); }

.ak-timeout { border-left: 3px solid var(--status-warning); }
.ak-cme_error, .ak-cms_error, .ak-error { border-left: 3px solid var(--status-danger); }
.ak-disconnect { border-left: 3px solid var(--status-info); }
.ev-error { border-left: 3px solid var(--status-danger); }
.ev-warning { border-left: 3px solid var(--status-warning); }

.empty { padding: 40px 0; text-align: center; color: var(--text-tertiary); font-size: 14px; }
</style>
