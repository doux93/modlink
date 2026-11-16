<script setup lang="ts">
import {
  NButton,
  NSelect,
  NTag,
  useMessage,
} from "naive-ui";
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useWorkspace } from "../composables/useWorkspace";
import { useSerialStore } from "../stores/serial";
import { deviceStatus } from "../utils/deviceStatus";
import type { KnowledgeCapabilities, ManualContentLevel } from "../types";

const store = useSerialStore();
const message = useMessage();
const { connectDevice } = useWorkspace();
const busy = ref(false);
const capabilities = ref<KnowledgeCapabilities | null>(null);
const capabilitiesLoading = ref(false);
const KNOWLEDGE_CAPABILITIES_CHANGED = "modulink:knowledge-capabilities-changed";
let capabilityRequestVersion = 0;

const activeSession = computed(() => store.activeSession);
const status = computed(() => deviceStatus(activeSession.value, store.activeProfile));
const portOptions = computed(() => store.sessions
  .filter((s) => !store.deviceGroups.some((g) => g.logPort === s.port))
  .map((s) => {
    const group = store.deviceGroups.find((g) => g.atPort === s.port);
    return { label: `${group?.name && group.name !== s.port ? group.name + ' · ' : ''}${s.port}${s.connected ? '' : ' · 已断开'}`, value: s.port };
  }));
const profileOptions = computed(() => (store.knowledge?.profiles ?? []).map((p) => ({ label: p.name, value: p.id })));
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
  capabilitiesLoading.value = true;
  capabilities.value = null;
  try {
    const result = await invoke<KnowledgeCapabilities>("get_knowledge_capabilities", {
      profileId: store.activeProfileId || null,
    });
    if (requestVersion === capabilityRequestVersion) capabilities.value = result;
  } catch {
    if (requestVersion === capabilityRequestVersion) capabilities.value = null;
  } finally {
    if (requestVersion === capabilityRequestVersion) capabilitiesLoading.value = false;
  }
}

function handleKnowledgeCapabilitiesChanged() {
  void refreshCapabilities();
}

function selectPort(port: string) {
  store.selPort = port;
  store.activePort = port;
}

async function disconnectActive() {
  if (!activeSession.value || busy.value) return;
  const port = activeSession.value.port;
  busy.value = true;
  try {
    if (store.activeDeviceGroup) await store.closeDeviceGroup(store.activeDeviceGroup.id);
    else await store.closeSession(port);
    message.info(`已断开 ${port}`);
  } catch (error) {
    message.error(String(error));
  } finally {
    busy.value = false;
  }
}

async function identify() {
  const port = store.activePort;
  const session = store.activeSession;
  if (!session?.connected || status.value.identifying) return;
  const name = await store.detectProfile(port);
  if (!store.sessions.includes(session) || !session.connected) return;
  if (name) message.success(`${port} · 已识别 ${name}`);
  else if (session.identification === "unmatched" || session.identification === "failed")
    message.warning(`${port} · ${session.identificationError || "未匹配到型号，可手动选择；通用调试仍可使用"}`);
}

onMounted(async () => {
  window.addEventListener(KNOWLEDGE_CAPABILITIES_CHANGED, handleKnowledgeCapabilitiesChanged);
  await store.init();
  if (!store.activePort && portOptions.value.length) selectPort(portOptions.value[0].value);
  await refreshCapabilities();
});
onUnmounted(() => {
  capabilityRequestVersion++;
  window.removeEventListener(KNOWLEDGE_CAPABILITIES_CHANGED, handleKnowledgeCapabilitiesChanged);
});

watch(
  () => store.activeProfileId,
  () => void refreshCapabilities(),
);
</script>

<template>
  <header class="device-context">
    <span class="connection-state" :class="{ online: status.connected }"><i />{{ status.connectionLabel }}</span>
    <span class="context-label">当前设备</span>
    <n-select v-if="portOptions.length" :value="store.activePort || null" class="port-select" size="small" aria-label="当前设备" :options="portOptions" :consistent-menu-width="false" @update:value="selectPort" />
    <span v-else class="no-device">连接后开始调试与诊断</span>
    <n-button size="small" :type="activeSession?.connected ? 'default' : 'primary'" @click="connectDevice">{{ activeSession?.connected ? '添加设备' : '连接设备' }}</n-button>
    <span v-if="activeSession" class="model-label" :title="[status.modelLabel, status.detail, status.reason].filter(Boolean).join('；')" role="status">{{ status.modelLabel }} · {{ activeSession.baud }}</span>
    <details v-if="activeSession" class="device-details">
      <summary>设备详情</summary>
      <div class="detail-panel">
        <b>设备 {{ activeSession.port }}</b>
        <p class="device-availability">{{ status.detail }}</p>
        <p v-if="status.reason" class="identification-reason">{{ status.reason }}</p>
        <p>设备型号（可手动选择）</p>
        <n-select :value="store.activeProfileId" @update:value="store.setActiveProfile" :options="profileOptions" filterable aria-label="设备型号" placeholder="选择设备型号" />
        <p>固件：{{ activeSession.firmware || '尚未读取' }}</p>
        <p>可用资料：<n-tag size="small" :type="capabilities ? modeType : 'default'" :bordered="false">{{ capabilitiesLoading ? '正在读取' : capabilities ? modeText : '读取失败' }}</n-tag> <n-button v-if="!capabilitiesLoading && !capabilities" size="tiny" quaternary @click="refreshCapabilities">重试</n-button></p>
        <div class="detail-actions"><n-button size="small" :disabled="!activeSession.connected" :loading="status.identifying" @click="identify">重新识别</n-button><n-button size="small" :loading="busy" @click="disconnectActive">断开设备</n-button></div>
      </div>
    </details>
  </header>
</template>

<style scoped>
.device-context { position: relative; min-height: 56px; padding: 10px 16px; display: flex; align-items: center; flex-wrap: wrap; gap: 10px; border-bottom: 1px solid var(--border-subtle); background: var(--surface-raised); flex-shrink: 0; }
.connection-state { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-secondary); white-space: nowrap; }
.connection-state i { width: 7px; height: 7px; border-radius: 50%; background: var(--text-tertiary); }
.connection-state.online { color: var(--status-success); }
.connection-state.online i { background: var(--status-success); }
.context-label, .no-device, .model-label { font-size: 12px; color: var(--text-secondary); }
.port-select { width: 190px; }
.model-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 240px; }
.device-details { margin-left: auto; font-size: 12px; }
summary { cursor: pointer; color: var(--text-secondary); }
.detail-panel { position: absolute; top: calc(100% + 4px); right: 12px; z-index: 25; width: min(400px, 90vw); padding: 18px; background: var(--surface-raised); border: 1px solid var(--border-default); border-radius: 10px; box-shadow: var(--shadow-lg); font-size: 13px; }
.detail-panel p { margin: 12px 0; color: var(--text-secondary); }
.detail-actions { display: flex; gap: 8px; margin-top: 16px; }
</style>
