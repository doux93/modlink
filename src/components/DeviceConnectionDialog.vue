<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { NButton, NInput, NModal, NSelect, useMessage } from "naive-ui";
import { useSerialStore } from "../stores/serial";
import { useWorkspace } from "../composables/useWorkspace";

const store = useSerialStore();
const message = useMessage();
const { connectionOpen } = useWorkspace();
const port = ref<string | null>(null);
const baud = ref(115200);
const name = ref("");
const logPort = ref<string | null>(null);
const logBaud = ref(921600);
const advanced = ref(false);
const busy = ref(false);
const refreshing = ref(false);
const error = ref("");
const availablePorts = computed(() => store.ports.map((p) => ({
  label: `${p.name}${p.description ? ` · ${p.description}` : ""}`,
  value: p.name,
  disabled: store.sessions.some((s) => s.port === p.name && s.connected) || store.deviceGroups.some((g) => g.logPort === p.name),
})));
const reconnectingGroup = computed(() => store.deviceGroups.find((g) => g.atPort === port.value));
const logOptions = computed(() => availablePorts.value.filter((p) => p.value !== port.value && !store.sessions.some((s) => s.port === p.value)));
const validBaud = (value: number) => Number.isInteger(value) && value > 0 && value <= 4_000_000;
const canConnect = computed(() => !!port.value && availablePorts.value.some((p) => p.value === port.value && !p.disabled)
  && validBaud(baud.value) && (!advanced.value || !logPort.value || (logPort.value !== port.value
    && availablePorts.value.some((p) => p.value === logPort.value && !p.disabled) && validBaud(logBaud.value))));
async function refresh() {
  refreshing.value = true;
  error.value = "";
  try {
    await store.refreshPorts();
    if (!availablePorts.value.some((p) => p.value === port.value && !p.disabled)) {
      port.value = availablePorts.value.find((p) => !p.disabled)?.value ?? null;
    }
  } catch (e) { error.value = `无法刷新串口：${e}`; }
  finally { refreshing.value = false; }
}
watch(connectionOpen, (open) => {
  if (!open) return;
  name.value = ""; logPort.value = null; advanced.value = false;
  baud.value = store.baud; logBaud.value = 921600;
  port.value = store.selPort || null;
  void refresh();
}, { immediate: true });
watch(port, (value) => { if (logPort.value === value) logPort.value = null; });

async function connect() {
  if (!canConnect.value || busy.value || !port.value) return;
  const targetPort = port.value;
  busy.value = true; error.value = "";
  try {
    store.baud = baud.value; store.selPort = targetPort; store.lastError = "";
    if (reconnectingGroup.value) {
      await store.openSession(targetPort);
      if (name.value.trim()) reconnectingGroup.value.name = name.value.trim();
    } else if (name.value.trim() || (advanced.value && logPort.value)) {
      if (store.sessions.some((s) => s.port === targetPort && !s.connected)) await store.closeSession(targetPort);
      await store.createDeviceGroup(name.value.trim(), targetPort, baud.value, advanced.value ? logPort.value || "" : "", logBaud.value);
    } else {
      await store.openSession(targetPort);
    }
    connectionOpen.value = false;
    message.success(`已连接 ${targetPort}，正在识别型号`);
    if (store.lastError) message.warning(store.lastError);
    // Identification stays bound to this device even if the user switches views.
    void store.detectProfile(targetPort).then((model) => {
      if (model) message.success(`${targetPort} · ${model}`);
    });
  } catch (e) { error.value = `连接失败：${e}。请检查设备连接、驱动及端口占用后重试。`; }
  finally { busy.value = false; }
}
function setOpen(open: boolean) { if (!busy.value) connectionOpen.value = open; }
</script>

<template>
  <n-modal :show="connectionOpen" @update:show="setOpen" preset="card" title="连接设备"
    class="connection-dialog" style="width: min(520px, 92vw)" :closable="!busy" :mask-closable="!busy" :close-on-esc="!busy">
    <p class="intro">选择设备的 AT 串口。连接成功后可直接开始调试或诊断。</p>
    <div class="field"><label id="connect-port-label">AT 串口</label><div class="port-row">
      <n-select v-model:value="port" aria-labelledby="connect-port-label" :options="availablePorts" placeholder="选择设备串口" filterable :disabled="busy || refreshing" />
      <n-button :loading="refreshing" :disabled="busy" @click="refresh">刷新串口</n-button>
    </div></div>
    <p v-if="!refreshing && !availablePorts.length" class="hint">未发现串口。请连接 USB 设备并安装对应驱动，然后刷新。</p>
    <p v-else-if="!refreshing && availablePorts.every((p) => p.disabled)" class="hint">所有串口均已打开，可关闭此面板并在顶部切换设备。</p>
    <div class="field"><label for="connect-baud">波特率</label>
      <input id="connect-baud" v-model.number="baud" type="number" min="1" max="4000000" step="1" list="common-baud" :disabled="busy" />
      <datalist id="common-baud"><option v-for="b in [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600]" :key="b" :value="b" /></datalist>
      <small>默认 115200，可按设备要求修改。</small>
    </div>
    <div class="field"><label id="connect-name-label">设备名称（可选）</label>
      <n-input v-model:value="name" aria-labelledby="connect-name-label" placeholder="例如：1 号测试板" :disabled="busy" />
    </div>
    <p v-if="reconnectingGroup" class="hint">重新连接现有设备，保留已关联的日志串口。</p>
    <details v-else :open="advanced" @toggle="advanced = ($event.target as HTMLDetailsElement).open">
      <summary>高级连接：同时采集日志串口</summary>
      <div class="field"><label id="connect-log-label">日志串口（可选）</label>
        <n-select v-model:value="logPort" aria-labelledby="connect-log-label" :options="logOptions" clearable placeholder="选择日志串口" :disabled="busy" />
      </div>
      <div class="field"><label for="connect-log-baud">日志串口波特率</label><input id="connect-log-baud" v-model.number="logBaud" type="number" min="1" max="4000000" step="1" :disabled="busy" /></div>
    </details>
    <p v-if="error" role="alert" class="error">{{ error }}</p>
    <template #footer><div class="actions"><n-button :disabled="busy" @click="setOpen(false)">取消</n-button><n-button type="primary" :disabled="!canConnect || refreshing" :loading="busy" @click="connect">连接并识别</n-button></div></template>
  </n-modal>
</template>

<style scoped>
.intro { margin-bottom: 18px; color: var(--text-secondary); line-height: 1.6; }
.field { display: flex; flex-direction: column; gap: 7px; margin: 14px 0; }
.field label { font-weight: 500; }
.port-row, .actions { display: flex; gap: 10px; }
.port-row > .n-select { flex: 1; min-width: 0; }
.actions { justify-content: flex-end; }
input { padding: 8px 10px; border: 1px solid var(--border-default); border-radius: 6px; background: var(--surface-base); color: var(--text-primary); font: inherit; }
small, .hint { color: var(--text-secondary); font-size: 12px; }
summary { cursor: pointer; color: var(--text-secondary); padding: 8px 0; }
.error { color: var(--status-danger); line-height: 1.6; }
</style>
