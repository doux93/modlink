<script setup lang="ts">
import { NButton, NCheckbox, NTag } from "naive-ui";
import { AlertTriangle, ListChecks, FlaskConical } from "@lucide/vue";
import AppIcon from "../AppIcon.vue";
import type { UiItem } from "../../types/ai";

defineProps<{ item: UiItem; busy: boolean; targetAvailable: boolean }>();
const emit = defineEmits<{
  execute: [];
  cancel: [];
  toggleStep: [index: number, checked: boolean];
}>();
</script>

<template>
  <!-- AT 序列确认卡 -->
  <div v-if="item.kind === 'card'" class="card" :class="`card-${item.card!.status}`">
    <div class="card-head">
      <b>
        <AppIcon :icon="ListChecks" :size="14" />
        <span class="gap-4">AT 指令序列</span>
      </b>
      <span class="card-purpose">{{ item.card!.purpose }}</span>
      <n-tag v-if="item.card!.target" size="tiny" :bordered="false">{{ item.card!.target!.port }}</n-tag>
      <n-tag v-if="item.card!.status === 'cancelled'" size="tiny" :bordered="false">已取消</n-tag>
      <n-tag v-else-if="item.card!.status === 'failed'" size="tiny" type="error" :bordered="false">执行失败</n-tag>
      <n-tag v-else-if="item.card!.status === 'done'" size="tiny" type="success" :bordered="false">已执行</n-tag>
    </div>
    <div v-for="(s, si) in item.card!.steps" :key="si" class="card-step">
      <n-checkbox :checked="s.checked" @update:checked="(checked: boolean) => emit('toggleStep', si, checked)" size="small" :disabled="busy || item.card!.status !== 'pending'" />
      <code>{{ s.cmd }}</code>
      <n-tag v-if="s.risk && s.risk !== 'none'" size="tiny" type="error" :bordered="false">
        <span class="btn-label-row">
          <AppIcon :icon="AlertTriangle" :size="11" />
          <span class="gap-3">高危: {{ s.risk }}</span>
        </span>
      </n-tag>
    </div>
    <div v-if="item.card!.results.length" class="card-results">
      <div v-for="(r, ri) in item.card!.results" :key="ri" class="card-result">{{ r }}</div>
    </div>
    <div v-if="item.card!.status === 'pending'" class="card-actions">
      <n-button size="small" type="primary" :disabled="busy || !targetAvailable" @click="emit('execute')">
        执行选中指令
      </n-button>
      <n-button size="small" :disabled="busy" @click="emit('cancel')">取消</n-button>
      <small v-if="!targetAvailable" class="warn">目标会话已失效，请重新生成</small>
    </div>
  </div>

  <!-- 测试用例确认卡 -->
  <div
    v-else-if="item.kind === 'diagnostic'"
    class="card"
    :class="`card-${item.diagnostic!.status}`"
  >
    <div class="card-head">
      <b>
        <AppIcon :icon="FlaskConical" :size="14" />
        <span class="gap-4">测试用例</span>
      </b>
      <span class="card-purpose">{{ item.diagnostic!.purpose }}</span>
      <n-tag v-if="item.diagnostic!.target" size="tiny" :bordered="false">{{ item.diagnostic!.target!.port }}</n-tag>
      <n-tag v-if="item.diagnostic!.status === 'cancelled'" size="tiny" :bordered="false">已取消</n-tag>
      <n-tag v-else-if="item.diagnostic!.status === 'failed'" size="tiny" type="error" :bordered="false">未通过</n-tag>
      <n-tag v-else-if="item.diagnostic!.status === 'done'" size="tiny" type="success" :bordered="false">
        已完成
      </n-tag>
    </div>
    <div class="card-step">
      <code>{{ item.diagnostic!.flowId }}</code>
    </div>
    <div v-if="item.diagnostic!.details.length" class="card-results">
      <div class="card-result">overall: {{ item.diagnostic!.result }}</div>
      <div v-for="(r, ri) in item.diagnostic!.details" :key="ri" class="card-result">{{ r }}</div>
    </div>
    <div v-if="item.diagnostic!.status === 'pending'" class="card-actions">
      <n-button size="small" type="primary" :disabled="busy || !targetAvailable" @click="emit('execute')">
        运行体检
      </n-button>
      <n-button size="small" :disabled="busy" @click="emit('cancel')">取消</n-button>
      <small v-if="!targetAvailable" class="warn">目标会话已失效，请重新生成</small>
    </div>
  </div>
</template>

<style scoped>
.card {
  align-self: flex-start; border: 1px solid var(--brand-primary); border-radius: 12px;
  padding: 12px 16px; min-width: 0; max-width: 85%;
  background: var(--surface-raised);
}
.card-cancelled { opacity: 0.55; }
.card-head { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 10px; margin-bottom: 8px; }
.card-head b { display: inline-flex; align-items: center; color: var(--text-primary); }
.card-purpose { color: var(--text-secondary); font-size: 13px; }
.card-step { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 3px 0; }
.card-step code {
  font-family: Consolas, monospace; font-size: 13px; background: var(--surface-base);
  padding: 2px 8px; border-radius: 5px; max-width: 100%; overflow-wrap: anywhere;
  color: var(--text-primary);
}
.card-results { margin-top: 8px; border-top: 1px dashed var(--border-default); padding-top: 8px; }
.card-result { font-family: Consolas, monospace; font-size: 12px; color: var(--text-secondary); padding: 2px 0; }
.card-actions { display: flex; gap: 8px; align-items: center; margin-top: 10px; }

.warn { color: var(--status-warning); }
.gap-4 { margin-left: var(--space-1); }
.gap-3 { margin-left: 3px; }
.btn-label-row { display: inline-flex; align-items: center; }
.card-failed { border-color: var(--status-danger); }
.card-running { border-color: var(--status-warning); }
.card-actions { flex-wrap: wrap; }
.card-result { overflow-wrap: anywhere; }
@media (max-width: 720px) { .card { max-width: 100%; } }
</style>
