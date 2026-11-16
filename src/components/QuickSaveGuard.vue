<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from "vue";
import { NButton } from "naive-ui";
import { isTauri } from "@tauri-apps/api/core";
import { useSerialStore } from "../stores/serial";
import { createCloseGuard } from "../services/closeGuard";
const store = useSerialStore();
const closeError = ref("");
let unlisten: (() => void) | undefined;
let disposed = false;
async function retry() {
  try { await store.flushQuickCmds(); closeError.value = ""; } catch { /* error remains visible */ }
}
onMounted(async () => {
  if (!isTauri()) { store.quickGuardReady = true; return; }
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const window = getCurrentWindow();
    const stop = await window.onCloseRequested(createCloseGuard({
      dirty: () => store.quickDirty || store.quickSaving,
      flush: () => store.flushQuickCmds(),
      destroy: () => window.destroy(),
      setClosing: value => { store.quickClosing = value; },
      failed: error => { closeError.value = `关闭已取消，快捷指令修改仍保留：${error}`; },
    }));
    if (disposed) stop(); else { unlisten = stop; store.quickGuardReady = true; }
  } catch (error) { closeError.value = `退出前保存保护未能启用：${error}`; }
});
onBeforeUnmount(() => { disposed = true; store.quickGuardReady = false; unlisten?.(); });
</script>
<template>
  <div v-if="closeError || store.quickError" class="quick-save-alert" role="alert">
    <span>{{ closeError || store.quickError }}</span>
    <n-button v-if="store.quickLoaded" size="tiny" :loading="store.quickSaving" @click="retry">重试保存</n-button>
    <n-button v-else size="tiny" :loading="store.quickLoading" @click="store.loadQuickCmds()">重新读取</n-button>
    <n-button v-if="!store.quickLoaded" size="tiny" :disabled="store.quickLoading" @click="store.loadQuickCmds(true)">恢复上次备份</n-button>
  </div>
</template>
<style scoped>
.quick-save-alert { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; padding: 8px 14px; background: var(--surface-raised); color: var(--status-danger); border-bottom: 1px solid var(--border-subtle); font-size: 12px; }
.quick-save-alert span { flex: 1; min-width: 200px; overflow-wrap: anywhere; }
</style>
