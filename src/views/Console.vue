<script setup lang="ts">
import {
  NButton,
  NCheckbox,
  NInput,
  NTag,
  NTooltip,
  useMessage,
} from "naive-ui";
import { ref, computed, onMounted, onActivated, onDeactivated, onBeforeUnmount } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { X, Cable, FileText, AlertTriangle } from "@lucide/vue";
import EmptyState from "../components/EmptyState.vue";
import { useWorkspace } from "../composables/useWorkspace";
import AppIcon from "../components/AppIcon.vue";
import QuickCommandPanel from "../components/console/QuickCommandPanel.vue";
import TerminalLog from "../components/console/TerminalLog.vue";
import { useSerialStore } from "../stores/serial";
import { applicableAtCommands } from "../knowledge";
import { useUiPrefs } from "../composables/useUiPrefs";

const store = useSerialStore();
const message = useMessage();
const { connectDevice } = useWorkspace();
const { activeView, showQuickCommands } = useUiPrefs();
const visibleSessionCount = computed(() => store.deviceGroups.length + store.standaloneSessions.length);

onMounted(() => store.init());

async function closeTab(port: string) {
  try {
    await store.closeSession(port);
  } catch (e) {
    message.error(String(e));
  }
}

/** 关键词过滤（不区分大小写；数据全量保留，仅影响显示） */
const filterText = ref("");

// 日志口输出（分屏右侧，渲染上限防卡顿；同步应用 filterText）
const logFrames = computed(() => {
  const lp = store.activeDeviceGroup?.logPort;
  let fs = lp ? store.framesByPort[lp] ?? [] : [];
  const q = filterText.value.trim().toLowerCase();
  if (q) fs = fs.filter((f) => (f.hex ?? f.text).toLowerCase().includes(q));
  return fs.length > 1500 ? fs.slice(-1500) : fs;
});
const logAutoScroll = ref(true);
function clearLogPane() {
  const lp = store.activeDeviceGroup?.logPort;
  if (lp) store.framesByPort[lp] = [];
}

async function saveLogPane() {
  const lp = store.activeDeviceGroup?.logPort;
  if (!lp) return;
  const path = await saveDialog({
    defaultPath: `modulink_log_${lp}_${Date.now()}.log`,
    filters: [{ name: "日志", extensions: ["log", "txt"] }],
  });
  if (!path) return;
  await invoke("export_session_log", { port: lp, path });
  message.success("日志口数据已保存");
}

// ---------------- 布局：可拖动分栏（宽度持久化） ----------------
function readPanelWidth(key: string, fallback: number) {
  try {
    const value = Number(localStorage.getItem(key));
    return Number.isFinite(value) && value >= 200 && value <= 720 ? value : fallback;
  } catch { return fallback; }
}
const quickW = ref(readPanelWidth("ml.quickW", 280));
const logW = ref(readPanelWidth("ml.logW", 340));
const bodyEl = ref<HTMLElement>();
const bodyWidth = ref(960);
const availableWidth = computed(() => Math.max(0, bodyWidth.value - 16));
const quickWidth = computed(() => Math.min(quickW.value, Math.max(200,
  availableWidth.value - 320 - (store.activeDeviceGroup?.logPort ? 210 : 5))));
const logWidth = computed(() => Math.min(logW.value, Math.max(200,
  availableWidth.value - 320 - (showQuickCommands.value ? quickWidth.value + 10 : 5))));
let observer: ResizeObserver | undefined;
let finishDrag: (() => void) | undefined;
function measureBody() {
  const width = bodyEl.value?.getBoundingClientRect().width;
  if (width) bodyWidth.value = width;
}
onMounted(() => {
  measureBody();
  observer = new ResizeObserver(measureBody);
  if (bodyEl.value) observer.observe(bodyEl.value);
});
onActivated(measureBody);
onDeactivated(() => finishDrag?.());
onBeforeUnmount(() => { observer?.disconnect(); finishDrag?.(); });

function startDrag(which: "quick" | "log", e: MouseEvent) {
  e.preventDefault();
  finishDrag?.();
  const startX = e.clientX;
  const start = which === "quick" ? quickWidth.value : logWidth.value;
  const move = (ev: MouseEvent) => {
    const v = Math.min(720, Math.max(200, start + (startX - ev.clientX)));
    if (which === "quick") quickW.value = v;
    else logW.value = v;
  };
  const up = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    finishDrag = undefined;
    try {
      localStorage.setItem("ml.quickW", String(quickW.value));
      localStorage.setItem("ml.logW", String(logW.value));
    } catch { /* Layout remains usable when storage is unavailable. */ }
  };
  finishDrag = up;
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}

// ---------------- 输出区 ----------------
const showTime = ref(true);
const showHex = ref(false);
const autoScroll = ref(true);
/** 终端字号三档（持久化到 useUiPrefs） */
const { fontSize } = useUiPrefs();
const FONT_SIZES = [12, 13, 15];
function cycleFont() {
  const i = FONT_SIZES.indexOf(fontSize.value);
  fontSize.value = FONT_SIZES[(i + 1) % FONT_SIZES.length];
}
/** 输出区渲染上限：数据保留全量(2万行/口)，DOM 只渲染最近 N 行防卡顿 */
const RENDER_CAP = 2000;

const filteredFrames = computed(() => {
  const q = filterText.value.trim().toLowerCase();
  return q ? store.activeFrames.filter((f) => (f.hex ?? f.text).toLowerCase().includes(q)) : store.activeFrames;
});
const displayedFrames = computed(() => filteredFrames.value.slice(-RENDER_CAP));
const hiddenCount = computed(() => Math.max(0, filteredFrames.value.length - RENDER_CAP));

async function saveLog() {
  const path = await saveDialog({
    defaultPath: `modulink_${store.activePort}_${Date.now()}.log`,
    filters: [{ name: "日志", extensions: ["log", "txt"] }],
  });
  if (!path) return;
  await invoke("export_session_log", { port: store.activePort, path });
  message.success("日志已保存");
}

// ---------------- 发送区 ----------------
const input = ref("");
const sendHex = ref(false);
const appendNewline = ref(true);
const history = ref<string[]>([]);
let histIdx = -1;

const connected = computed(() => store.activeSession?.connected ?? false);

/** 输入实时匹配 AT 字典 → 显示语法 + 手册摘录（画像优先，回落到知识库批量文档） */
const docHint = computed(() => {
  const t = input.value.trim().toUpperCase();
  if (t.length < 3 || !t.startsWith("AT")) return null;
  const profileCmds = store.activeProfile?.at_commands ?? [];
  const kbCmds = applicableAtCommands(store.knowledge, store.activeProfile).map((item) => item.command);
  const all = [...profileCmds, ...kbCmds];
  // 取输入的指令主体（去掉 =?/? 及参数）
  const body = t.split(/[=?]/)[0];
  const exact = all.find((c) => c.cmd.toUpperCase() === body);
  const prefix = exact ?? all.find((c) => c.cmd.toUpperCase().startsWith(body));
  return prefix ?? null;
});

async function send() {
  const data = input.value;
  if (!data.trim() || !connected.value) return;
  try {
    await store.write(data, sendHex.value, appendNewline.value);
    if (history.value[0] !== data) history.value.unshift(data);
    if (history.value.length > 50) history.value.pop();
    histIdx = -1;
    input.value = "";
  } catch (e) {
    message.error(String(e));
  }
}

function onKey(e: KeyboardEvent) {
  if (e.isComposing || e.keyCode === 229) return;
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    send();
  } else if (e.key === "ArrowUp") {
    if (histIdx < history.value.length - 1) {
      histIdx++;
      input.value = history.value[histIdx];
    }
  } else if (e.key === "ArrowDown") {
    if (histIdx > 0) {
      histIdx--;
      input.value = history.value[histIdx];
    } else {
      histIdx = -1;
      input.value = "";
    }
  }
}

</script>

<template>
  <div class="console">
    <header class="workspace-heading">
      <h2>串口控制台</h2>
      <div class="heading-actions">
        <n-button size="small" quaternary :aria-pressed="showQuickCommands" @click="showQuickCommands = !showQuickCommands">{{ showQuickCommands ? '收起快捷指令' : '展开快捷指令' }}</n-button>
        <n-button size="small" secondary :disabled="!connected" @click="activeView = 'diag'">诊断当前设备</n-button>
      </div>
    </header>
    <div class="toolbar" aria-label="终端显示设置">
      <n-input
        v-model:value="filterText"
        size="small"
        placeholder="过滤关键词…"
        clearable
        style="width: 150px"
      />
      <n-checkbox v-model:checked="showTime" size="small">时间戳</n-checkbox>
      <n-checkbox v-model:checked="showHex" size="small">Hex</n-checkbox>
      <n-checkbox v-model:checked="autoScroll" size="small">自动滚动</n-checkbox>
      <n-tooltip trigger="hover">
        <template #trigger>
          <n-button size="small" quaternary @click="cycleFont">A{{ fontSize }}</n-button>
        </template>
        切换终端字号
      </n-tooltip>
      <n-button size="small" quaternary :disabled="!store.activePort" @click="saveLog">
        保存日志
      </n-button>
      <n-button size="small" quaternary :disabled="!store.activePort" @click="store.clear()">
        清屏
      </n-button>
    </div>

    <!-- 会话页签（多路串口：设备组在前，独立会话在后） -->
    <div v-if="visibleSessionCount > 1" class="session-tabs">
      <div
        v-for="g in store.deviceGroups"
        :key="g.id"
        class="session-tab group-tab"
        :class="{ active: g.atPort === store.activePort }"
        @click="store.activePort = g.atPort"
      >
        <span class="dot" :class="{ on: store.sessions.find((s) => s.port === g.atPort)?.connected }" />
        <AppIcon :icon="Cable" :size="14" />
        {{ g.name }}
        <small>{{ g.atPort }}{{ g.logPort ? " + " + g.logPort : "" }}</small>
        <span class="tab-close" @click.stop="store.closeDeviceGroup(g.id)">
          <AppIcon :icon="X" :size="12" />
        </span>
      </div>
      <div
        v-for="s in store.standaloneSessions"
        :key="s.port"
        class="session-tab"
        :class="{ active: s.port === store.activePort }"
        @click="store.activePort = s.port"
      >
        <span class="dot" :class="{ on: s.connected }" />
        {{ s.port }} <small>@{{ s.baud }}</small>
        <span class="tab-close" @click.stop="closeTab(s.port)">
          <AppIcon :icon="X" :size="12" />
        </span>
      </div>
    </div>

    <div ref="bodyEl" class="body">
      <!-- 输出 + 输入 -->
      <section class="io">
        <EmptyState v-if="!connected && !store.activeFrames.length" class="connection-empty" :icon="Cable"
          :title="store.sessions.length ? '当前设备已断开' : '准备好开始调试了吗'" description="选择 AT 串口与波特率，连接后可发送指令，或运行基础诊断定位问题。"
          action-label="连接设备" @action="connectDevice" />
        <TerminalLog v-else
          :frames="displayedFrames"
          :source-key="`${store.activePort}:${store.activeSession?.generation ?? 0}`"
          :hidden-count="hiddenCount"
          :font-size="fontSize"
          :show-time="showTime"
          :show-hex="showHex"
          :auto-scroll="autoScroll"
          :error-codes="store.activeErrorCodes"
          :urc-patterns="store.activeProfile?.urc_patterns ?? []"
          :empty-message="filterText ? '没有匹配过滤条件的行' : store.sessions.length ? '等待数据…' : '点击「连接设备」开始调试'"
        />
        <div class="statusbar">
          <span title="本次软件运行中所有端口的累计接收量">总 RX <b>{{ store.rxBytes.toLocaleString() }}</b> B</span>
          <span title="本次软件运行中所有端口的累计发送量">总 TX <b>{{ store.txBytes.toLocaleString() }}</b> B</span>
          <span>{{ store.activeFrames.length.toLocaleString() }} 行</span>
          <span v-if="store.activeProfile" class="kb-hint">
            {{ store.activeProfile.id === 'generic' ? '通用指令集' : '型号' }}: {{ store.activeProfile.name }}
          </span>
          <span v-if="store.lastError" class="err">
            <AppIcon :icon="AlertTriangle" :size="13" />
            {{ store.lastError }}
          </span>
        </div>
        <div v-if="docHint" class="doc-hint">
          <span class="doc-cmd">{{ docHint.syntax || docHint.cmd }}</span>
          <n-tag v-if="docHint.risk !== 'none'" size="tiny" type="error" :bordered="false">
            <AppIcon :icon="AlertTriangle" :size="11" />
            高危: {{ docHint.risk }}
          </n-tag>
          <span class="doc-text">{{ docHint.doc }}</span>
        </div>
        <div class="input-bar">
          <n-input
            v-model:value="input"
            type="textarea"
            :rows="2"
            :placeholder="sendHex ? '输入 Hex，如 1A 2B 3C' : '输入 AT 指令，Enter 发送，↑↓ 历史'"
            @keydown="onKey"
          />
          <div class="input-side">
            <n-checkbox v-model:checked="sendHex" size="small">Hex</n-checkbox>
            <n-checkbox v-model:checked="appendNewline" size="small">回车换行</n-checkbox>
            <n-button type="primary" :disabled="!connected" @click="send">发送</n-button>
          </div>
        </div>
      </section>

      <!-- 日志口分屏（设备组激活且绑定了日志口时显示，宽度可拖） -->
      <div
        v-if="store.activeDeviceGroup?.logPort"
        class="splitter"
        @mousedown="startDrag('log', $event)"
      />
      <section
        v-if="store.activeDeviceGroup?.logPort"
        class="logpane"
        :style="{ width: logWidth + 'px' }"
      >
        <div class="logpane-head">
          <b>
            <AppIcon :icon="FileText" :size="14" />
            日志口 {{ store.activeDeviceGroup.logPort }}
          </b>
          <div class="logpane-ops">
            <n-checkbox v-model:checked="logAutoScroll" size="small">滚动</n-checkbox>
            <n-button size="tiny" quaternary @click="saveLogPane">保存</n-button>
            <n-button size="tiny" quaternary @click="clearLogPane">清屏</n-button>
          </div>
        </div>
        <TerminalLog
          :frames="logFrames"
          :source-key="store.activeDeviceGroup.logPort"
          :auto-scroll="logAutoScroll"
          :font-size="12"
          plain
          empty-message="等待日志口数据…"
        />
      </section>

      <!-- 快捷指令面板（宽度可拖） -->
      <div v-show="showQuickCommands" class="splitter" @mousedown="startDrag('quick', $event)" />
      <QuickCommandPanel v-show="showQuickCommands" :style="{ width: quickWidth + 'px' }" />
    </div>




  </div>
</template>

<style scoped>
.connection-empty { flex: 1; background: var(--surface-raised); border: 1px dashed var(--border-default); border-radius: 10px; }
.console { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden; }
.toolbar {
  display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 10px 14px;
  border-bottom: 1px solid var(--border-subtle);
  background: var(--surface-raised);
}
.toolbar > :not(.spacer) { flex-shrink: 0; }

.session-tabs {
  display: flex; gap: 4px; padding: 6px 12px 0; overflow-x: auto; overflow-y: hidden;
  border-bottom: 1px solid var(--border-subtle);
  background: var(--surface-raised);
}
.session-tab {
  display: flex; align-items: center; gap: 7px; padding: 6px 12px;
  border-radius: 8px 8px 0 0; cursor: pointer; font-size: 13px;
  background: var(--surface-hover); color: var(--text-secondary);
  user-select: none; flex-shrink: 0;
}
.session-tab.active {
  background: var(--brand-primary-soft);
  color: var(--brand-primary);
  font-weight: 600;
}
.session-tab small { color: var(--text-secondary); font-weight: 400; }
.dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-tertiary); }
.dot.on { background: var(--status-success); box-shadow: 0 0 5px var(--status-success); }
.tab-close { color: var(--text-tertiary); padding: 0 2px; display: inline-flex; }
.tab-close:hover { opacity: 1; color: var(--status-danger); }

.body { flex: 1; display: flex; min-height: 0; }
.io { flex: 1; display: flex; flex-direction: column; min-width: 0; padding: 10px 12px; gap: 8px; }

.statusbar {
  display: flex; flex-wrap: wrap; gap: 4px 18px;
  font-size: 12px; color: var(--text-tertiary); padding: 0 4px;
}
.statusbar b { color: var(--brand-primary); }
.statusbar .err { color: var(--status-danger); display: inline-flex; align-items: center; gap: 4px; max-width: 100%; overflow-wrap: anywhere; }
.kb-hint { color: var(--text-secondary); }

.doc-hint {
  display: flex; flex-wrap: wrap; align-items: flex-start; gap: 6px 8px; padding: 8px 10px;
  border-radius: 6px; background: var(--brand-primary-soft);
  font-size: 12px; line-height: 1.5;
  color: var(--text-primary); max-height: 140px; overflow-y: auto; flex-shrink: 0;
}
.doc-cmd {
  flex: 1 1 480px; min-width: 0; font-family: Consolas, monospace; color: var(--brand-primary);
  white-space: normal; overflow-wrap: anywhere;
}
.doc-text {
  flex: 1 0 100%; min-width: 0; color: var(--text-secondary);
  white-space: normal; overflow-wrap: anywhere;
}

.input-bar { display: flex; gap: 10px; align-items: stretch; flex-shrink: 0; }
.input-bar > :first-child { flex: 1; }
.input-side { display: flex; flex-direction: column; justify-content: space-between; gap: 4px; }

.splitter {
  width: 5px; flex-shrink: 0; cursor: col-resize; border-radius: 2px;
  transition: background 0.15s;
}
.splitter:hover, .splitter:active { background: var(--brand-primary); }


.logpane {
  min-width: 200px; flex-shrink: 0; display: flex; flex-direction: column;
  padding: 10px 6px 10px 4px; gap: 8px;
}
.logpane-head {
  display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 6px;
  font-size: 13px; color: var(--text-primary);
}
.logpane-head b { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; white-space: nowrap; }
.logpane-ops { display: flex; align-items: center; gap: 6px; flex-shrink: 0; margin-left: auto; white-space: nowrap; }

/* === 工具类（替代 inline style） === */

.workspace-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 40px; padding: 6px 14px; flex-shrink: 0; }
.workspace-heading h2 { font-size: 14px; line-height: 24px; margin: 0; font-weight: 600; }
.heading-actions { display: flex; align-items: center; gap: 8px; }
.toolbar { padding: 6px 14px; background: transparent; gap: 6px 10px; flex-shrink: 0; }
.toolbar :deep(.n-input) { margin-right: auto; }
.body { padding: 4px 8px 8px; }
.io { padding: 10px 8px; }
</style>
