<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { useDialog, useMessage } from "naive-ui";
import { ChevronDown, ChevronRight } from "@lucide/vue";
import AppIcon from "./AppIcon.vue";
import { useExecutionStore } from "../stores/execution";
import { useSerialStore } from "../stores/serial";
import type {
  ExecutionKind,
  ExecutionLifecycleEvent,
  ExecutionRisk,
  ExecutionSource,
  ExecutionStatus,
} from "../types";

const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{
  "update:show": [value: boolean];
  navigate: [view: string];
}>();

const store = useExecutionStore();
const serial = useSerialStore();
const dialog = useDialog();
const message = useMessage();

const expandedId = ref<string | null>(null);

// 打开抽屉时清掉上次的展开项
watch(
  () => props.show,
  (visible) => {
    if (visible) expandedId.value = null;
  }
);

const sourceLabels: Record<ExecutionSource, string> = {
  console: "控制台",
  quick_command: "快捷指令",
  profile_detection: "型号识别",
  ai: "AI",
  diagnostic: "诊断",
  stress: "指令压测",
  scenario_stress: "场景压测",
};

const kindLabels: Record<ExecutionKind, string> = {
  at_sequence: "AT序列",
  diagnostic: "诊断",
  stress: "压测",
  scenario_stress: "场景压测",
};

const statusLabels: Record<ExecutionStatus, string> = {
  running: "运行中",
  succeeded: "成功",
  failed: "失败",
  cancelled: "已取消",
};

const riskLabels: Record<ExecutionRisk, string> = {
  read_only: "只读",
  state_change: "改变状态",
  destructive: "高影响",
  unknown: "未知",
};

const finishedCount = computed(() => store.records.length - store.runningCount);

function statusType(status: ExecutionStatus): "info" | "success" | "error" | "default" {
  if (status === "running") return "info";
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  return "default";
}

function riskType(risk: ExecutionRisk): "success" | "warning" | "error" | "default" {
  if (risk === "read_only") return "success";
  if (risk === "state_change") return "warning";
  if (risk === "destructive") return "error";
  return "default";
}

function toggle(id: string) {
  expandedId.value = expandedId.value === id ? null : id;
}

function formatSummary(record: ExecutionLifecycleEvent): string {
  const s = record.summary;
  if (!s) return "";
  switch (record.kind) {
    case "at_sequence":
      return `${s.completed}/${s.planned} 条指令`;
    case "diagnostic":
      return `${s.completed}/${s.planned} 项检查，${s.passed} 通过，${s.failed} 失败`;
    case "stress":
      return `${s.completed}/${s.planned} 次，失败 ${s.failed}`;
    case "scenario_stress":
      return `${s.completed}/${s.planned} 轮，失败 ${s.failed}`;
  }
}

function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString("zh-CN", { hour12: false });
}

function formatDuration(record: ExecutionLifecycleEvent): string {
  if (record.status === "running") return "进行中";
  if (record.elapsedMs < 1000) return `${record.elapsedMs} ms`;
  if (record.elapsedMs < 60_000) return `${(record.elapsedMs / 1000).toFixed(1)} s`;
  const minutes = Math.floor(record.elapsedMs / 60_000);
  const seconds = Math.floor((record.elapsedMs % 60_000) / 1000);
  return `${minutes} min ${seconds} s`;
}

async function onStop(record: ExecutionLifecycleEvent) {
  const ok = await store.stop(record.executionId);
  if (ok) message.info("已发送停止信号");
  else message.warning("任务可能已结束");
}

async function onRerun(record: ExecutionLifecycleEvent) {
  if (!record.results?.length) {
    message.warning("该记录无可重放的步骤");
    return;
  }
  const doExec = async () => {
    try {
      const target = serial.executionTarget(record.target.port);
      await store.rerun(record, target, async (t, steps, source) => {
        await serial.executeAtSequence(t, steps, source);
      });
      message.success("已重新执行");
      emit("navigate", "console");
      emit("update:show", false);
    } catch (e) {
      message.error(String(e));
    }
  };
  if (record.risk === "destructive") {
    dialog.warning({
      title: "高影响指令确认",
      content: "该记录含高影响指令（如恢复出厂），确认重新执行？",
      positiveText: "执行",
      negativeText: "取消",
      onPositiveClick: doExec,
    });
  } else {
    await doExec();
  }
}

function onViewDetail(record: ExecutionLifecycleEvent) {
  if (record.kind === "diagnostic") emit("navigate", "diag");
  else emit("navigate", "stress");
  emit("update:show", false);
}

function onAiAnalysis(record: ExecutionLifecycleEvent) {
  store.requestAiAnalysis(record);
  emit("navigate", "ai");
  emit("update:show", false);
}

function onLogView(record: ExecutionLifecycleEvent) {
  store.requestLogView(record.target.port);
  emit("navigate", "logs");
  emit("update:show", false);
}

async function onExport(record: ExecutionLifecycleEvent) {
  try {
    const saved = await store.exportReport(record);
    if (saved) message.success("报告已导出");
  } catch (e) {
    message.error(String(e));
  }
}
</script>

<template>
  <n-drawer
    :show="show"
    :width="520"
    placement="right"
    @update:show="emit('update:show', $event)"
  >
    <n-drawer-content title="执行记录" closable>
      <n-alert v-if="store.persistenceError" type="warning" closable @close="store.persistenceError = ''">
        {{ store.persistenceError }}
      </n-alert>
      <div class="history-toolbar">
        <span>最近 {{ store.records.length }} 条（运行中 {{ store.runningCount }}）</span>
        <n-button
          size="tiny"
          quaternary
          :disabled="finishedCount === 0"
          @click="store.clearFinished()"
        >
          清除已完成
        </n-button>
      </div>

      <div v-if="!store.records.length" class="history-empty">
        暂无执行记录
      </div>

      <div v-else class="history-list">
        <div
          v-for="record in store.records"
          :key="record.executionId"
          class="history-card"
        >
          <!-- summary row -->
          <div class="history-row" @click="toggle(record.executionId)">
            <div class="history-head">
              <span class="expand-icon" :class="{ expanded: expandedId === record.executionId }">
                <AppIcon :icon="expandedId === record.executionId ? ChevronDown : ChevronRight" :size="11" />
              </span>
              <strong>{{ kindLabels[record.kind] }}</strong>
              <span class="source-tag">{{ sourceLabels[record.source] }}</span>
              <n-tag size="tiny" :bordered="false" :type="statusType(record.status)">
                {{ statusLabels[record.status] }}
              </n-tag>
              <time>{{ formatTime(record.startedAt) }}</time>
            </div>
            <div class="history-meta">
              <span>{{ record.target.port }}</span>
              <span>会话 #{{ record.target.sessionGeneration }}</span>
              <span v-if="record.summary">{{ formatSummary(record) }}</span>
              <span v-else>规模 {{ record.stepCount }}</span>
              <span>{{ formatDuration(record) }}</span>
            </div>
            <div class="history-foot">
              <n-tag size="tiny" :bordered="false" :type="riskType(record.risk)">
                {{ riskLabels[record.risk] }}
              </n-tag>
              <span v-if="record.error" class="history-error" :title="record.error">
                {{ record.error }}
              </span>
            </div>
          </div>

          <!-- action buttons -->
          <div class="history-actions">
            <n-button
              v-if="record.status === 'running'"
              size="tiny"
              type="error"
              ghost
              @click.stop="onStop(record)"
            >
              停止
            </n-button>
            <template v-if="record.status === 'failed'">
              <n-button size="tiny" type="info" ghost @click.stop="onAiAnalysis(record)">
                AI 分析
              </n-button>
              <n-button size="tiny" ghost @click.stop="onLogView(record)">
                查看日志
              </n-button>
            </template>
            <n-tooltip
              v-if="record.status !== 'running' && record.results?.length && record.kind === 'at_sequence'"
              :disabled="!record.resultsTruncated"
            >
              <template #trigger>
                <n-button
                  size="tiny"
                  ghost
                  :disabled="record.resultsTruncated"
                  @click.stop="onRerun(record)"
                >
                  重新执行
                </n-button>
              </template>
              结果已截断，重放会丢失指令，已禁用
            </n-tooltip>
            <n-button
              v-if="record.status !== 'running' && record.kind !== 'at_sequence'"
              size="tiny"
              ghost
              @click.stop="onViewDetail(record)"
            >
              {{ record.kind === "diagnostic" ? "打开诊断页" : "打开压测页" }}
            </n-button>
            <n-button
              v-if="record.status !== 'running'"
              size="tiny"
              quaternary
              @click.stop="onExport(record)"
            >
              导出
            </n-button>
          </div>

          <!-- detail expansion -->
          <div v-if="expandedId === record.executionId" class="history-detail">
            <div v-if="record.results?.length" class="detail-list">
              <div
                v-for="(r, i) in record.results"
                :key="i"
                class="detail-item"
              >
                <div class="detail-cmd">
                  <span class="detail-idx">{{ i + 1 }}</span>
                  <code>{{ r.request }}</code>
                  <n-tag
                    size="tiny"
                    :bordered="false"
                    :type="r.final_code === 'OK' ? 'success' : 'error'"
                  >
                    {{ r.final_code }}
                  </n-tag>
                  <span class="detail-elapsed">{{ r.elapsed_ms }}ms</span>
                </div>
                <div v-if="r.lines.length" class="detail-resp">
                  <div v-for="(line, j) in r.lines" :key="j" class="detail-line">{{ line }}</div>
                </div>
              </div>
            </div>
            <div v-else class="detail-empty">
              诊断/压测的详细报告请查看对应页面
            </div>
            <div v-if="record.resultsTruncated" class="detail-truncated">
              <span class="truncated-prefix">提示</span> 结果体积超限，已截断显示与导出。
            </div>
          </div>
        </div>
      </div>
    </n-drawer-content>
  </n-drawer>
</template>

<style scoped>
.history-toolbar {
  height: 34px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--border-subtle);
  color: var(--text-tertiary);
  font-size: 12px;
}

.history-empty {
  min-height: 180px;
  display: grid;
  place-items: center;
  color: var(--text-tertiary);
}

.history-card {
  padding: 10px 0;
  border-bottom: 1px solid var(--border-subtle);
}

.history-row {
  cursor: pointer;
}

.history-head,
.history-meta,
.history-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.expand-icon {
  font-size: 9px;
  opacity: 0.5;
  transition: transform 0.15s;
  display: inline-flex;
  align-items: center;
}

.history-head strong {
  font-size: 13px;
  color: var(--text-primary);
}
.source-tag {
  font-size: 11px;
  color: var(--text-secondary);
  padding: 1px 4px;
  background: var(--surface-hover);
  border-radius: 3px;
}

.history-head time {
  margin-left: auto;
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
  font-size: 12px;
}

.history-meta {
  margin-top: 6px;
  color: var(--text-secondary);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  padding-left: 17px;
}

.history-meta span + span::before {
  content: "\00B7";
  margin-right: 8px;
  opacity: 0.55;
}

.history-foot {
  margin-top: 6px;
  padding-left: 17px;
}

.history-error {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--status-danger);
  font-size: 12px;
}

.history-actions {
  display: flex;
  gap: 6px;
  margin-top: 8px;
  padding-left: 17px;
  flex-wrap: wrap;
}

.history-detail {
  margin-top: 8px;
  padding: 8px 10px 4px 17px;
  background: var(--surface-hover);
  border-radius: 6px;
}

.detail-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.detail-item {
  border-left: 2px solid var(--border-default);
  padding-left: 8px;
}

.detail-cmd {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.detail-idx {
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
  min-width: 16px;
}

.detail-cmd code {
  font-family: "Cascadia Code", "Consolas", monospace;
  font-size: 12px;
  background: var(--surface-hover);
  padding: 1px 5px;
  border-radius: 3px;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-primary);
}

.detail-elapsed {
  color: var(--text-tertiary);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.detail-resp {
  margin-top: 3px;
  padding-left: 22px;
}

.detail-line {
  font-family: "Cascadia Code", "Consolas", monospace;
  font-size: 11px;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-all;
}

.detail-empty {
  color: var(--text-tertiary);
  font-size: 12px;
  padding: 4px 0;
}
.detail-truncated {
  margin-top: 6px;
  color: var(--status-warning);
  font-size: 11px;
}
.truncated-prefix {
  font-weight: 600;
}
</style>
