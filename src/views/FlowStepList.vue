<script setup lang="ts">
import { ref, watch, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useMessage } from "naive-ui";
import { Timer, RotateCw } from "@lucide/vue";
import AppIcon from "../components/AppIcon.vue";
import { useSerialStore } from "../stores/serial";
import type { DiagFlowFull, EvalRule } from "../types";

const props = defineProps<{
  flow: DiagFlowFull;
  /** 运行中时禁用编辑 */
  disabled?: boolean;
}>();
const emit = defineEmits<{ saved: [id: string] }>();

const store = useSerialStore();
const message = useMessage();

const editing = ref(false);
const saving = ref(false);
const expanded = ref(false);

interface EditRule extends EvalRule {
  mode: "when" | "default";
  collectText: string;
  message: string;
}

interface EditRow {
  send: string;
  timeoutMs: number | null;
  parse: string;
  urcEnabled: boolean;
  urcPattern: string;
  urcTimeoutMs: number | null;
  rules: EditRule[];
}
const editRows = ref<EditRow[]>([]);

const isMine = computed(() => props.flow.id.startsWith("my_"));

watch(
  () => props.flow.id,
  () => {
    editing.value = false;
    expanded.value = false;
  }
);
watch(() => props.disabled, (disabled) => { if (disabled) editing.value = false; });

function toEditRule(rule: EvalRule): EditRule {
  const collect = Array.isArray(rule.collect) ? rule.collect : [];
  return {
    ...JSON.parse(JSON.stringify(rule)),
    mode: rule.default ? "default" : "when",
    collectText: collect.join("\n"),
    message: rule.result === "pass" ? (rule.detail ?? "") : (rule.reason ?? rule.detail ?? ""),
  };
}

function startEdit() {
  editRows.value = (props.flow.steps ?? []).map((s) => ({
    send: (s.send ?? []).join("\n"),
    timeoutMs: s.timeout_ms,
    parse: s.parse ?? "",
    urcEnabled: !!s.expect_urc,
    urcPattern: s.expect_urc?.pattern ?? "",
    urcTimeoutMs: s.expect_urc?.timeout_ms ?? 15000,
    rules: (s.evaluate ?? []).map(toEditRule),
  }));
  editing.value = true;
  expanded.value = true;
}

function cancelEdit() {
  editing.value = false;
}

function addRule(stepIndex: number) {
  editRows.value[stepIndex].rules.push({
    mode: "when",
    when: "final == 'OK'",
    default: false,
    result: "pass",
    detail: null,
    reason: null,
    suggestion: null,
    collect: [],
    collectText: "",
    message: "",
    abort: false,
  });
}

function removeRule(stepIndex: number, ruleIndex: number) {
  editRows.value[stepIndex].rules.splice(ruleIndex, 1);
}

function validRegex(value: string): boolean {
  try {
    new RegExp(value);
    return true;
  } catch {
    return false;
  }
}

function validateEdits(): string | null {
  for (let i = 0; i < editRows.value.length; i++) {
    const row = editRows.value[i];
    if (!row.send.split("\n").some((line) => line.trim())) return `步骤 ${i + 1} 至少需要一条指令`;
    if (row.timeoutMs == null || !Number.isFinite(row.timeoutMs) || row.timeoutMs < 500 || row.timeoutMs > 120000) {
      return `步骤 ${i + 1} 的超时必须在 500-120000 ms 之间`;
    }
    if (row.parse.trim() && !validRegex(row.parse.trim())) return `步骤 ${i + 1} 的解析正则无效`;
    if (row.urcEnabled) {
      if (!row.urcPattern.trim() || !validRegex(row.urcPattern.trim())) return `步骤 ${i + 1} 的 URC 正则无效`;
      if (row.urcTimeoutMs == null || !Number.isFinite(row.urcTimeoutMs) || row.urcTimeoutMs < 100) {
        return `步骤 ${i + 1} 的 URC 超时无效`;
      }
    }
    if (!row.rules.length) return `步骤 ${i + 1} 至少需要一条判定规则`;
    if (row.rules.filter((rule) => rule.mode === "default").length > 1) return `步骤 ${i + 1} 只能有一条默认规则`;
    for (const rule of row.rules) {
      if (rule.mode === "when" && !rule.when?.trim()) return `步骤 ${i + 1} 存在空判定条件`;
    }
  }
  return null;
}

/** 高亮占位符 <XXX> 的指令行（只读模式用） */
function renderCmd(text: string): string {
  const esc = text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return esc.replace(/&lt;[A-Z_]+&gt;/g, (m) => `<span class="ph">${m}</span>`);
}

async function save() {
  if (props.flow.steps.length !== editRows.value.length) return;
  const issue = validateEdits();
  if (issue) {
    message.warning(issue);
    return;
  }
  saving.value = true;
  try {
    const cloned: DiagFlowFull = JSON.parse(JSON.stringify(props.flow));
    cloned.steps = cloned.steps.map((step, i) => ({
      ...step,
      send: editRows.value[i].send
        .split("\n")
        .map((s) => s.trim())
        .filter(Boolean),
      timeout_ms: editRows.value[i].timeoutMs!,
      parse: editRows.value[i].parse.trim() || null,
      expect_urc: editRows.value[i].urcEnabled
        ? { pattern: editRows.value[i].urcPattern.trim(), timeout_ms: editRows.value[i].urcTimeoutMs! }
        : null,
      evaluate: editRows.value[i].rules.map((rule) => ({
        when: rule.mode === "when" ? rule.when?.trim() || null : null,
        default: rule.mode === "default",
        result: rule.result,
        detail: rule.result === "pass" ? rule.message.trim() || null : null,
        reason: rule.result === "pass" ? null : rule.message.trim() || null,
        suggestion: rule.suggestion?.trim() || null,
        collect: rule.collectText.split("\n").map((item) => item.trim()).filter(Boolean),
        abort: rule.abort,
      })),
    }));
    cloned.verified = false;
    cloned.execution_mode = "unknown";
    // 非 my_ 用例：自动克隆为 my_ 新副本
    if (!isMine.value) {
      cloned.id = `my_${Math.random().toString(16).slice(2, 10)}`;
      cloned.kind = "usecase";
      cloned.source = `${props.flow.source}（克隆）`;
    }
    await invoke("save_usecase", { flow: cloned });
    await store.reloadKnowledge();
    message.success(isMine.value ? "已保存，需重新实机复核" : `已克隆为「${cloned.name}」并保存`);
    editing.value = false;
    emit("saved", cloned.id);
  } catch (e: any) {
    message.error(`保存失败：${e}`);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="flow-steps" :class="{ disabled }">
    <div class="steps-toolbar">
      <span class="steps-count">{{ flow.steps.length }} 步</span>
      <span v-if="editing" class="edit-hint">修改指令或判定后需重新实机复核{{ isMine ? "" : "（将创建你的副本）" }}</span>
      <span style="flex: 1" />
      <template v-if="!editing">
        <n-button size="tiny" secondary :disabled="disabled" @click="startEdit">编辑步骤</n-button>
        <n-button v-if="flow.steps.length > 3" size="tiny" quaternary @click="expanded = !expanded">
          {{ expanded ? "收起步骤" : `展开其余 ${flow.steps.length - 3} 步` }}
        </n-button>
      </template>
      <template v-else>
        <n-button size="tiny" :loading="saving" type="primary" @click="save">保存</n-button>
        <n-button size="tiny" quaternary :disabled="saving" @click="cancelEdit">取消</n-button>
      </template>
    </div>

    <div v-for="(s, i) in flow.steps" v-show="editing || expanded || i < 3" :key="s.id" class="step-row">
      <div class="step-meta">
        <span class="step-no">{{ i + 1 }}</span>
        <span class="step-title">{{ s.title }}</span>
        <span v-if="s.optional" class="tag opt">可选</span>
        <span class="tag mode" :class="`m-${flow.execution_mode}`">
          {{ flow.execution_mode === "readonly" ? "只读" : flow.execution_mode === "mutating" ? "可能改设备" : "未标注" }}
        </span>
        <span v-if="!editing" class="tag t-out">
          <AppIcon :icon="Timer" :size="11" />
          {{ s.timeout_ms }}ms
        </span>
        <span v-if="s.retry" class="tag t-retry">
          <AppIcon :icon="RotateCw" :size="11" />
          {{ s.retry.times }}次
        </span>
        <span v-if="s.expect_urc" class="tag t-urc">URC {{ s.expect_urc.timeout_ms }}ms</span>
      </div>

      <!-- 只读：指令预览 -->
      <template v-if="!editing">
        <div class="cmds">
          <code v-for="(cmd, ci) in s.send" :key="ci" class="cmd" v-html="renderCmd(cmd)" />
        </div>
        <div v-if="s.expect_urc" class="urc-line">等待上报: <code>{{ s.expect_urc.pattern }}</code></div>
      </template>

      <!-- 编辑：指令、解析、URC 和判定条件均可修改 -->
      <template v-else>
        <n-input
          v-model:value="editRows[i].send"
          type="textarea"
          :autosize="{ minRows: 1, maxRows: 4 }"
          size="small"
          placeholder="AT 指令（多行=多条）"
        />
        <n-input-number
          v-model:value="editRows[i].timeoutMs"
          :min="500"
          :max="60000"
          :step="1000"
          size="small"
          style="width: 130px; margin-top: 4px"
        >
          <template #suffix>ms</template>
        </n-input-number>
        <div class="edit-section">
          <label class="edit-label">解析正则 <span>用于从响应中提取变量</span></label>
          <n-input v-model:value="editRows[i].parse" size="small" placeholder="可选，例如 \\+CSQ: (?<rssi>\\d+)" />
        </div>
        <div class="edit-section urc-edit">
          <n-checkbox v-model:checked="editRows[i].urcEnabled">等待 URC</n-checkbox>
          <n-input v-if="editRows[i].urcEnabled" v-model:value="editRows[i].urcPattern" size="small" placeholder="URC 正则，例如 ^\\+MIPOPEN:" />
          <n-input-number v-if="editRows[i].urcEnabled" v-model:value="editRows[i].urcTimeoutMs" :min="100" :max="120000" :step="500" size="small" style="width: 130px">
            <template #suffix>ms</template>
          </n-input-number>
        </div>
        <div class="rules-edit">
          <div class="rules-head">
            <span class="edit-label">判定规则</span>
            <n-button size="tiny" quaternary @click="addRule(i)">＋ 添加规则</n-button>
          </div>
          <div v-for="(rule, ri) in editRows[i].rules" :key="ri" class="rule-edit-row">
            <n-select v-model:value="rule.mode" size="small" :options="[
              { label: '条件', value: 'when' },
              { label: '默认', value: 'default' },
            ]" style="width: 86px" />
            <n-input v-if="rule.mode === 'when'" v-model:value="rule.when" size="small" class="rule-condition" placeholder="例如 final == 'OK' && urc == 1" />
            <n-select v-model:value="rule.result" size="small" :options="[
              { label: '通过', value: 'pass' },
              { label: '警告', value: 'warn' },
              { label: '失败', value: 'fail' },
            ]" style="width: 86px" />
            <n-input v-model:value="rule.message" size="small" class="rule-message" placeholder="结果说明 / 失败原因" />
            <n-checkbox v-model:checked="rule.abort">中止</n-checkbox>
            <n-button size="tiny" quaternary type="error" @click="removeRule(i, ri)">删除</n-button>
            <n-input v-model:value="rule.suggestion" size="small" class="rule-suggestion" placeholder="可选建议" />
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.flow-steps { border: 1px solid var(--border-subtle); border-radius: 8px; padding: 10px; margin-top: 8px; }
.flow-steps.disabled { opacity: .6; }
.steps-toolbar { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
.steps-count { font-weight: 600; font-size: 13px; color: var(--text-primary); }
.edit-hint { font-size: 12px; color: var(--text-tertiary); }

.step-row { padding: 8px 0; border-bottom: 1px solid var(--border-subtle); }
.step-row:last-child { border-bottom: none; }
.step-meta { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin-bottom: 4px; }
.step-no { width: 18px; height: 18px; border-radius: 50%; background: var(--brand-primary-soft); color: var(--brand-primary); font-size: 11px; display: grid; place-items: center; font-weight: 600; }
.step-title { font-size: 13px; font-weight: 500; color: var(--text-primary); }
.tag { font-size: 10px; padding: 1px 5px; border-radius: 4px; white-space: nowrap; display: inline-flex; align-items: center; gap: 3px; }
.tag.opt { background: var(--surface-hover); color: var(--text-tertiary); }
.tag.m-readonly { background: var(--status-success-soft); color: var(--status-success); }
.tag.m-mutating { background: var(--status-danger-soft); color: var(--status-danger); }
.tag.m-unknown { background: var(--surface-hover); color: var(--text-tertiary); }
.tag.t-out, .tag.t-retry, .tag.t-urc { background: var(--surface-hover); color: var(--text-tertiary); }

.cmds { display: flex; flex-direction: column; gap: 2px; }
.cmd { font-family: "Cascadia Code", "Consolas", monospace; font-size: 12px; padding: 2px 6px; border-radius: 4px; background: var(--surface-hover); color: var(--text-primary); }
:deep(.ph) { color: var(--status-warning); font-weight: 600; }
.urc-line { font-size: 11px; color: var(--text-tertiary); margin-top: 3px; }
.urc-line code { font-family: monospace; }
.edit-section, .rules-edit { margin-top: 8px; padding-top: 8px; border-top: 1px solid var(--border-subtle); }
.edit-label { display: flex; align-items: baseline; gap: 6px; margin-bottom: 5px; font-size: 11px; color: var(--text-secondary); }
.edit-label span { font-size: 10px; color: var(--text-tertiary); }
.urc-edit { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.urc-edit .n-input { flex: 1; min-width: 180px; }
.rules-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 5px; }
.rules-head .edit-label { margin-bottom: 0; }
.rule-edit-row { display: grid; grid-template-columns: 86px minmax(160px, 1.5fr) 86px minmax(140px, 1fr) auto auto; gap: 6px; align-items: center; margin-top: 6px; }
.rule-condition, .rule-message, .rule-suggestion { min-width: 0; }
.rule-suggestion { grid-column: 2 / -1; }
@media (max-width: 760px) {
  .rule-edit-row { grid-template-columns: 86px minmax(0, 1fr) 86px auto auto; }
  .rule-message { grid-column: 2 / -1; }
  .rule-suggestion { grid-column: 2 / -1; }
}
</style>
