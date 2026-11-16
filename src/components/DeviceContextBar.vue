<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useMessage } from "naive-ui";
import { useSerialStore } from "../stores/serial";
import type { KnowledgeCapabilities, ManualContentLevel } from "../types";

const store = useSerialStore();
const message = useMessage();
const busy = ref(false);
const identifying = ref(false);
const capabilities = ref<KnowledgeCapabilities | null>(null);
const KNOWLEDGE_CAPABILITIES_CHANGED = "modulink:knowledge-capabilities-changed";
let capabilityRequestVersion = 0;

const selectedPort = computed(() =>
  store.activePort || store.selPort || store.ports[0]?.name || "",
);
const activeSession = computed(() => store.activeSession);
const activeProfile = computed(() => store.activeProfile);
const profileLabel = computed(() =>
  activeSession.value && activeProfile.value?.id !== "generic"
    ? activeProfile.value?.name || "未识别"
    : "未识别",
);
const portOptions = computed(() =>
  store.ports.map((port) => {
    const session = store.sessions.find((item) => item.port === port.name && item.connected);
    return {
      label: `${port.name}${port.description ? ` · ${port.description}` : ""}${session ? " · 已连接" : ""}`,
      value: port.name,
    };
  }),
);
const effectiveMode = computed<ManualContentLevel>(() =>
  capabilities.value?.effectiveManualMode ?? capabilities.value?.manualMode ?? "none",
);
const modeText = computed(() => ({
  fulltext: "全文手册",
  preview: "摘要参考",
  none: "手册不可用",
})[effectiveMode.value]);
const modeType = computed(() => ({
  fulltext: "success" as const,
  preview: "warning" as const,
  none: "error" as const,
})[effectiveMode.value]);

async function refreshCapabilities() {
  const requestVersion = ++capabilityRequestVersion;
  try {
    const result = await invoke<KnowledgeCapabilities>("get_knowledge_capabilities", {
      profileId: store.activeProfileId || null,
    });
    if (requestVersion === capabilityRequestVersion) capabilities.value = result;
  } catch {
    if (requestVersion === capabilityRequestVersion) capabilities.value = null;
  }
}

function handleKnowledgeCapabilitiesChanged() {
  void refreshCapabilities();
}

function selectPort(port: string) {
  store.selPort = port;
  store.activePort = port;
}

async function connectSelected() {
  if (!selectedPort.value || busy.value) return;
  busy.value = true;
  try {
    store.selPort = selectedPort.value;
    await store.openSession(selectedPort.value);
    message.success(`已连接 ${selectedPort.value}`);
    await identify();
  } catch (error) {
    message.error(String(error));
  } finally {
    busy.value = false;
  }
}

async function disconnectActive() {
  if (!activeSession.value || busy.value) return;
  const port = activeSession.value.port;
  busy.value = true;
  try {
    await store.closeSession(port);
    message.info(`已断开 ${port}`);
  } catch (error) {
    message.error(String(error));
  } finally {
    busy.value = false;
  }
}

async function identify() {
  const port = store.activePort;
  if (!port || identifying.value) return;
  identifying.value = true;
  try {
    const name = await store.detectProfile(port);
    if (name) message.success(`已识别 ${name}`);
    else message.warning("未能精确识别型号，保留当前画像");
    await refreshCapabilities();
  } finally {
    identifying.value = false;
  }
}

onMounted(async () => {
  window.addEventListener(KNOWLEDGE_CAPABILITIES_CHANGED, handleKnowledgeCapabilitiesChanged);
  await store.init();
  const initialPort =
    store.activePort ||
    store.sessions.find((session) => session.connected)?.port ||
    store.selPort ||
    store.ports[0]?.name ||
    "";
  if (initialPort) selectPort(initialPort);
  await refreshCapabilities();
});
onUnmounted(() => {
  window.removeEventListener(KNOWLEDGE_CAPABILITIES_CHANGED, handleKnowledgeCapabilitiesChanged);
});

watch(
  () => store.activeProfileId,
  () => void refreshCapabilities(),
);
</script>

<template>
  <header class="device-context">
    <span class="connection-dot" :class="{ online: activeSession?.connected }" />
    <n-select
      :value="selectedPort"
      class="port-select"
      size="small"
      filterable
      :options="portOptions"
      placeholder="选择串口"
      :consistent-menu-width="false"
      @update:value="selectPort"
    />
    <n-button
      v-if="!activeSession?.connected"
      size="small"
      type="primary"
      :disabled="!selectedPort"
      :loading="busy"
      @click="connectSelected"
    >
      连接
    </n-button>
    <n-button v-else size="small" quaternary :loading="busy" @click="disconnectActive">
      断开
    </n-button>

    <span class="divider" />
    <div class="context-field">
      <small>型号</small>
      <b>{{ profileLabel }}</b>
    </div>
    <div class="context-field firmware" :title="activeSession?.firmware || '尚未读取固件版本'">
      <small>固件</small>
      <b>{{ activeSession?.firmware || "--" }}</b>
    </div>
    <n-tag size="small" :type="modeType" :bordered="false" :title="capabilities?.effectiveDegradationReason || ''">
      {{ modeText }}
    </n-tag>
    <n-button
      size="small"
      quaternary
      :disabled="!activeSession?.connected"
      :loading="identifying"
      title="重新识别型号和固件"
      @click="identify"
    >
      重新识别
    </n-button>
    <span class="session-id" v-if="activeSession">会话 #{{ activeSession.generation }}</span>
  </header>
</template>

<style scoped>
.device-context {
  min-height: 48px;
  padding: 7px 14px;
  display: flex;
  align-items: center;
  gap: 9px;
  border-bottom: 1px solid var(--border-subtle);
  background: var(--surface-raised);
  flex-shrink: 0;
}
.connection-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-tertiary);
  box-shadow: 0 0 0 3px var(--surface-hover);
}
.connection-dot.online {
  background: var(--status-success);
  box-shadow: 0 0 0 3px var(--status-success-soft);
}
.port-select { width: min(300px, 24vw); min-width: 180px; }
.divider { width: 1px; height: 24px; background: var(--border-default); margin: 0 3px; }
.context-field { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
.context-field small { color: var(--text-tertiary); font-size: 11px; }
.context-field b { font-size: 12px; font-weight: 600; white-space: nowrap; color: var(--text-primary); }
.firmware { max-width: 280px; }
.firmware b { overflow: hidden; text-overflow: ellipsis; }
.session-id { margin-left: auto; color: var(--text-tertiary); font-size: 11px; white-space: nowrap; }

@media (max-width: 900px) {
  .firmware, .session-id, .divider { display: none; }
  .port-select { width: min(240px, 38vw); }
  .device-context { overflow-x: auto; }
}
</style>
