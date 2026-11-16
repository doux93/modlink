<script setup lang="ts">
import {
  NButton,
  NCheckbox,
  NDivider,
  NInput,
  NModal,
  NSelect,
  NTag,
  NTooltip,
  useMessage,
} from "naive-ui";
import { ref, computed, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { X, Cable, FileText, AlertTriangle } from "@lucide/vue";
import AppIcon from "../components/AppIcon.vue";
import QuickCommandPanel from "../components/console/QuickCommandPanel.vue";
import TerminalLog from "../components/console/TerminalLog.vue";
import { useSerialStore } from "../stores/serial";
import { applicableAtCommands } from "../knowledge";
import { useUiPrefs } from "../composables/useUiPrefs";

const store = useSerialStore();
const message = useMessage();

onMounted(() => store.init());

// ---------------- 连接 / 会话 ----------------
const baudPresets = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];
const baudOptions = [
  ...baudPresets.map((b) => ({ label: String(b), value: b })),
  { label: "自定义", value: -1 },
];
const portOptions = computed(() =>
  store.ports.map((p) => ({
    label: p.description ? `${p.name} (${p.description})` : p.name,
    value: p.name,
    disabled: store.sessions.some((s) => s.port === p.name),
  }))
);
const profileOptions = computed(() => {
  const rank: Record<string, number> = { verified: 0, reference: 1, generic: 2 };
  const supportLabel: Record<string, string> = { verified: "已验证", reference: "资料画像" };
  return [...(store.knowledge?.profiles ?? [])]
    .sort((a, b) =>
      (rank[a.support_level] ?? 3) - (rank[b.support_level] ?? 3) ||
      a.name.localeCompare(b.name, "zh-CN", { numeric: true })
    )
    .map((p) => ({
      label: supportLabel[p.support_level]
        ? `${p.name} · ${supportLabel[p.support_level]}`
        : p.name,
      value: p.id,
    }));
});

async function connect() {
  try {
    const port = store.selPort;
    await store.openSession();
    message.success(`已连接 ${store.activePort} @ ${store.baud}`);
    // 型号自动识别结果提示（异步，不阻塞）
    store.detectProfile(port).then((name) => {
      if (name) message.info(`已识别模组型号，切换画像：${name}`);
    });
  } catch (e) {
    message.error(String(e));
  }
}

async function closeTab(port: string) {
  try {
    await store.closeSession(port);
  } catch (e) {
    message.error(String(e));
  }
}

// ---------------- 设备组（AT 口 + 日志口分屏） ----------------
const showGroupModal = ref(false);
const gName = ref("");
const gAtPort = ref("");
const gAtBaud = ref(115200);
const gLogPort = ref("");
const gLogBaud = ref(921600);
const baudCustomMode = ref(false);
const baudCustomText = ref("");
const gAtBaudCustomMode = ref(false);
const gAtBaudCustomText = ref("");
const gLogBaudCustomMode = ref(false);
const gLogBaudCustomText = ref("");

const freePortOptions = computed(() =>
  store.ports
    .filter((p) => !store.sessions.some((s) => s.port === p.name))
    .map((p) => ({
      label: p.description ? `${p.name} (${p.description})` : p.name,
      value: p.name,
    }))
);

function openGroupModal() {
  gName.value = "";
  gAtPort.value = freePortOptions.value[0]?.value ?? "";
  gLogPort.value = "";
  showGroupModal.value = true;
}

async function createGroup() {
  if (!gAtPort.value) {
    message.warning("请选择 AT 口");
    return;
  }
  try {
    await store.createDeviceGroup(
      gName.value.trim(),
      gAtPort.value,
      gAtBaud.value,
      gLogPort.value === gAtPort.value ? "" : gLogPort.value,
      gLogBaud.value
    );
    showGroupModal.value = false;
    message.success(`设备组已创建：${gAtPort.value}${gLogPort.value ? " + " + gLogPort.value : ""}`);
    store.detectProfile(gAtPort.value).then((name) => {
      if (name) message.info(`已识别模组型号，切换画像：${name}`);
    });
    if (store.lastError) message.warning(store.lastError);
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

async function reloadKb() {
  await store.reloadKnowledge();
  message.success(
    `知识库已重载：${store.knowledge?.profiles.length ?? 0} 个模组画像，` +
      `${Object.keys(store.knowledge?.error_codes ?? {}).length} 条错误码`
  );
}

// ---------------- 布局：可拖动分栏（宽度持久化） ----------------
const quickW = ref(Number(localStorage.getItem("ml.quickW")) || 320);
const logW = ref(Number(localStorage.getItem("ml.logW")) || 380);

function startDrag(which: "quick" | "log", e: MouseEvent) {
  e.preventDefault();
  const startX = e.clientX;
  const start = which === "quick" ? quickW.value : logW.value;
  const move = (ev: MouseEvent) => {
    const v = Math.min(720, Math.max(220, start + (startX - ev.clientX)));
    if (which === "quick") quickW.value = v;
    else logW.value = v;
  };
  const up = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    localStorage.setItem("ml.quickW", String(quickW.value));
    localStorage.setItem("ml.logW", String(logW.value));
  };
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
    <!-- 顶部连接栏 -->
    <header class="toolbar">
      <n-select
        v-model:value="store.selPort"
        :options="portOptions"
        size="small"
        style="width: 220px"
        placeholder="选择串口"
      />
      <n-button size="small" quaternary @click="store.refreshPorts()">刷新</n-button>
      <n-input v-if="baudCustomMode" :value="baudCustomText" @update:value="(v: string) => { const t = v.replace(/\D/g, ''); baudCustomText = t; const n = parseInt(t || '0', 10); if (n > 0) store.baud = n; }" @blur="() => { if (!baudCustomText) { baudCustomMode = false; } }" size="small" style="width: 105px" placeholder="输入波特率" /><n-select v-else v-model:value="store.baud" :options="baudOptions" size="small" style="width: 105px" @update:value="(v: number) => { if (v === -1) { baudCustomMode = true; const prev = store.baud; baudCustomText = String(prev > 0 ? prev : 115200); store.baud = prev > 0 ? prev : 115200; } }" />
      <n-button size="small" type="primary" @click="connect">打开会话</n-button>
      <n-tooltip trigger="hover">
        <template #trigger>
          <n-button size="small" secondary @click="openGroupModal">＋设备组</n-button>
        </template>
        AT 口 + 日志口绑定为一个设备，分屏显示（Cat1/RedCap 双口调试）
      </n-tooltip>
      <n-divider vertical />
      <span class="lbl">模组</span>
      <n-select
        class="profile-select"
        :value="store.activeProfileId"
        @update:value="store.setActiveProfile"
        :options="profileOptions"
        :consistent-menu-width="false"
        :title="store.activeProfile?.name"
        filterable
        size="small"
        placeholder="选择模组画像"
      />
      <n-tooltip trigger="hover">
        <template #trigger>
          <n-button size="small" quaternary @click="reloadKb">↻</n-button>
        </template>
        热重载知识库（data/ 目录的 YAML 改完即生效）
      </n-tooltip>
      <div class="spacer" />
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
    </header>

    <!-- 会话页签（多路串口：设备组在前，独立会话在后） -->
    <div v-if="store.sessions.length" class="session-tabs">
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

    <div class="body">
      <!-- 输出 + 输入 -->
      <section class="io">
        <TerminalLog
          :frames="displayedFrames"
          :source-key="`${store.activePort}:${store.activeSession?.generation ?? 0}`"
          :hidden-count="hiddenCount"
          :font-size="fontSize"
          :show-time="showTime"
          :show-hex="showHex"
          :auto-scroll="autoScroll"
          :error-codes="store.activeErrorCodes"
          :urc-patterns="store.activeProfile?.urc_patterns ?? []"
          :empty-message="filterText ? '没有匹配过滤条件的行' : store.sessions.length ? '等待数据…' : '选择串口并点击「打开会话」开始（可同时打开多个）'"
        />
        <div class="statusbar">
          <span>RX <b>{{ store.rxBytes.toLocaleString() }}</b> B</span>
          <span>TX <b>{{ store.txBytes.toLocaleString() }}</b> B</span>
          <span>{{ store.activeFrames.length.toLocaleString() }} 行</span>
          <span v-if="store.activeProfile" class="kb-hint">
            画像: {{ store.activeProfile.name }}
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
        :style="{ width: logW + 'px' }"
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
      <div class="splitter" @mousedown="startDrag('quick', $event)" />
      <QuickCommandPanel :style="{ width: quickW + 'px' }" />
    </div>



    <!-- 新建设备组弹窗 -->
    <n-modal v-model:show="showGroupModal" preset="card" style="width: 480px" title="新建设备组（AT 口 + 日志口）">
      <div class="edit-form">
        <n-input v-model:value="gName" placeholder="设备名称（可选，如：1号板）" />
        <div class="edit-row">
          <span class="lbl" style="width: 46px">AT 口</span>
          <n-select v-model:value="gAtPort" :options="freePortOptions" size="small" style="flex: 1" />
          <n-input v-if="gAtBaudCustomMode" :value="gAtBaudCustomText" @update:value="(v: string) => { const t = v.replace(/\D/g, ''); gAtBaudCustomText = t; const n = parseInt(t || '0', 10); if (n > 0) gAtBaud = n; }" @blur="() => { if (!gAtBaudCustomText) { gAtBaudCustomMode = false; } }" size="small" style="width: 105px" placeholder="输入波特率" /><n-select v-else v-model:value="gAtBaud" :options="baudOptions" size="small" style="width: 105px" @update:value="(v: number) => { if (v === -1) { gAtBaudCustomMode = true; const prev = gAtBaud; gAtBaudCustomText = String(prev > 0 ? prev : 115200); gAtBaud = prev > 0 ? prev : 115200; } }" />
        </div>
        <div class="edit-row">
          <span class="lbl" style="width: 46px">日志口</span>
          <n-select
            v-model:value="gLogPort"
            :options="freePortOptions.filter((o) => o.value !== gAtPort)"
            size="small"
            style="flex: 1"
            placeholder="可选，不选则只开 AT 口"
            clearable
          />
          <n-input v-if="gLogBaudCustomMode" :value="gLogBaudCustomText" @update:value="(v: string) => { const t = v.replace(/\D/g, ''); gLogBaudCustomText = t; const n = parseInt(t || '0', 10); if (n > 0) gLogBaud = n; }" @blur="() => { if (!gLogBaudCustomText) { gLogBaudCustomMode = false; } }" size="small" style="width: 105px" placeholder="输入波特率" /><n-select v-else v-model:value="gLogBaud" :options="baudOptions" size="small" style="width: 105px" @update:value="(v: number) => { if (v === -1) { gLogBaudCustomMode = true; const prev = gLogBaud; gLogBaudCustomText = String(prev > 0 ? prev : 921600); gLogBaud = prev > 0 ? prev : 921600; } }" />
        </div>
      </div>
      <template #footer>
        <div class="modal-actions">
          <n-button @click="showGroupModal = false">取消</n-button>
          <n-button type="primary" @click="createGroup">创建并连接</n-button>
        </div>
      </template>
    </n-modal>


  </div>
</template>

<style scoped>
.console { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden; }
.toolbar {
  display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 10px 14px;
  border-bottom: 1px solid var(--border-subtle);
  background: var(--surface-raised);
}
.toolbar > :not(.spacer) { flex-shrink: 0; }
.profile-select { width: 320px; flex: 0 0 320px; }
.lbl { font-size: 12px; color: var(--text-tertiary); }
.spacer { flex: 1 1 24px; min-width: 24px; }

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
.session-tab small { opacity: 0.6; font-weight: 400; }
.dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-tertiary); }
.dot.on { background: var(--status-success); box-shadow: 0 0 5px var(--status-success); }
.tab-close { opacity: 0.45; padding: 0 2px; display: inline-flex; }
.tab-close:hover { opacity: 1; color: var(--status-danger); }

.body { flex: 1; display: flex; min-height: 0; }
.io { flex: 1; display: flex; flex-direction: column; min-width: 0; padding: 10px 12px; gap: 8px; }

.statusbar {
  display: flex; flex-wrap: wrap; gap: 4px 18px;
  font-size: 12px; color: var(--text-tertiary); padding: 0 4px;
}
.statusbar b { color: var(--brand-primary); }
.statusbar .err { color: var(--status-danger); display: inline-flex; align-items: center; gap: 4px; }
.kb-hint { color: var(--text-secondary); }

.doc-hint {
  display: flex; flex-wrap: wrap; align-items: flex-start; gap: 6px 8px; padding: 8px 10px;
  border-radius: 6px; background: var(--brand-primary-soft);
  font-size: 12px; line-height: 1.5;
  color: var(--text-primary);
}
.doc-cmd {
  flex: 1 1 480px; min-width: 0; font-family: Consolas, monospace; color: var(--brand-primary);
  white-space: normal; overflow-wrap: anywhere;
}
.doc-text {
  flex: 1 0 100%; min-width: 0; color: var(--text-secondary);
  white-space: normal; overflow-wrap: anywhere;
}

.input-bar { display: flex; gap: 10px; align-items: stretch; }
.input-bar > :first-child { flex: 1; }
.input-side { display: flex; flex-direction: column; justify-content: space-between; gap: 4px; }

.splitter {
  width: 5px; flex-shrink: 0; cursor: col-resize; border-radius: 2px;
  transition: background 0.15s;
}
.splitter:hover, .splitter:active { background: var(--brand-primary); }


.logpane {
  min-width: 220px; flex-shrink: 0; display: flex; flex-direction: column;
  padding: 10px 6px 10px 4px; gap: 8px;
}
.logpane-head {
  display: flex; justify-content: space-between; align-items: center;
  font-size: 13px; color: var(--text-primary);
}
.logpane-head b { display: inline-flex; align-items: center; gap: 6px; }
.logpane-ops { display: flex; align-items: center; gap: 6px; }
.edit-form { display: flex; flex-direction: column; gap: 12px; }

/* === 工具类（替代 inline style） === */
.modal-actions { display: flex; justify-content: flex-end; gap: 8px; }
.field-row { display: flex; align-items: center; gap: 8px; }
.field-row > .lbl { width: 46px; }
.field-row > .flex-1 { flex: 1; }
</style>
