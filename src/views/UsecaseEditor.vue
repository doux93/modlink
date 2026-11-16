<script setup lang="ts">
import {
  NButton,
  NCheckbox,
  NDrawer,
  NDrawerContent,
  NInput,
  NInputNumber,
  NSelect,
  useMessage,
} from "naive-ui";
import { computed, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { X } from "@lucide/vue";
import AppIcon from "../components/AppIcon.vue";
import { useSerialStore } from "../stores/serial";
import type { DiagFlowFull, DiagStepFull, EvalRule, UrcExpect } from "../types";

const props = defineProps<{
  show: boolean;
  baseFlow: DiagFlowFull | null;
  mode: "create" | "edit" | "clone";
}>();
const emit = defineEmits<{ "update:show": [boolean]; saved: [string] }>();
const store = useSerialStore();
const message = useMessage();

interface EditStep {
  title: string;
  send: string;
  judge: "ok" | "contains" | "urc";
  judgeText: string;
  timeoutMs: number;
  optional: boolean;
}

const name = ref("");
const category = ref("custom");
const appliesTo = ref("");
const executionMode = ref<"readonly" | "mutating" | "unknown">("unknown");
const steps = ref<EditStep[]>([]);
const saving = ref(false);

const categoryOptions = [
  { label: "Socket 数据", value: "socket" },
  { label: "MQTT", value: "mqtt" },
  { label: "HTTP", value: "http" },
  { label: "FTP", value: "ftp" },
  { label: "网络", value: "network" },
  { label: "功能/睡眠", value: "power" },
  { label: "短信", value: "sms" },
  { label: "外设/硬件", value: "hardware" },
  { label: "其他", value: "misc" },
  { label: "自定义", value: "custom" },
];

const titleText = computed(() => {
  if (props.mode === "edit") return "编辑用例";
  if (props.mode === "clone") return "另存为我的用例";
  return "新建用例";
});

watch(() => props.show, (v) => { if (v) loadFromBase(); });

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** escapeRegex 的逆：编辑已有用例时把 pattern 还原为用户输入的字面文本，
 *  避免 保存→编辑→保存 造成双重转义（\+CSQ: 变 \\\+CSQ:）。 */
function unescapeRegex(s: string): string {
  return s.replace(/\\([.*+?^${}()|[\]\\])/g, "$1");
}

function newStep(): EditStep {
  return { title: "", send: "AT", judge: "ok", judgeText: "", timeoutMs: 8000, optional: false };
}

function stepToEdit(s: DiagStepFull): EditStep {
  if (s.expect_urc) {
    return { title: s.title, send: s.send.join("\n"), judge: "urc", judgeText: unescapeRegex(s.expect_urc.pattern.replace(/^\^/, "")), timeoutMs: s.timeout_ms, optional: s.optional };
  }
  if (s.parse && s.parse.includes("(?P<hit>")) {
    const m = s.parse.match(/\(\?P<hit>(.+)\)$/);
    return { title: s.title, send: s.send.join("\n"), judge: "contains", judgeText: m ? unescapeRegex(m[1]) : "", timeoutMs: s.timeout_ms, optional: s.optional };
  }
  return { title: s.title, send: s.send.join("\n"), judge: "ok", judgeText: "", timeoutMs: s.timeout_ms, optional: s.optional };
}

function loadFromBase() {
  const f = props.baseFlow;
  if (f) {
    name.value = props.mode === "edit" ? f.name : f.name + " (副本)";
    category.value = f.category || "custom";
    appliesTo.value = f.applies_to.join(",");
    executionMode.value = f.execution_mode ?? "unknown";
    steps.value = f.steps.length ? f.steps.map(stepToEdit) : [newStep()];
  } else {
    name.value = "";
    category.value = "custom";
    appliesTo.value = store.activeProfile?.model_codes.join(",") ?? "";
    executionMode.value = "unknown";
    steps.value = [newStep()];
  }
}

function buildStep(es: EditStep, idx: number): DiagStepFull {
  const send = es.send.split("\n").map((s) => s.trim()).filter(Boolean);
  let parse: string | null = null;
  let expectUrc: UrcExpect | null = null;
  const evaluate: EvalRule[] = [];
  const fail: EvalRule = { when: null, default: true, result: "fail", detail: null, reason: "", suggestion: null, collect: [], abort: false };
  if (es.judge === "ok") {
    evaluate.push({ when: "final == 'OK'", default: false, result: "pass", detail: null, reason: null, suggestion: null, collect: [], abort: false });
    evaluate.push({ ...fail, reason: "返回 {final}", abort: true });
  } else if (es.judge === "contains") {
    parse = `(?P<hit>${escapeRegex(es.judgeText)})`;
    evaluate.push({ when: "hit != ''", default: false, result: "pass", detail: null, reason: null, suggestion: null, collect: [], abort: false });
    evaluate.push({ ...fail, reason: `未包含 ${es.judgeText}` });
  } else {
    expectUrc = { pattern: `^${escapeRegex(es.judgeText)}`, timeout_ms: 15000 };
    evaluate.push({ when: "final == 'OK' && urc == 1", default: false, result: "pass", detail: null, reason: null, suggestion: null, collect: [], abort: false });
    evaluate.push({ ...fail, reason: `未收到上报 ${es.judgeText}`, abort: true });
  }
  return {
    id: `s${idx + 1}`,
    title: es.title || `步骤${idx + 1}`,
    send: send.length ? send : ["AT"],
    timeout_ms: es.timeoutMs,
    parse,
    retry: null,
    optional: es.optional,
    expect_urc: expectUrc,
    evaluate,
  };
}

function buildFlow(): DiagFlowFull {
  const id = props.mode === "edit" && props.baseFlow ? props.baseFlow.id : `my_${Math.random().toString(16).slice(2, 10)}`;
  return {
    schema: "modulink/diagflow@1",
    id,
    name: name.value.trim() || "未命名用例",
    applies_to: appliesTo.value.split(",").map((s) => s.trim().toUpperCase()).filter(Boolean),
    kind: "usecase",
    category: category.value,
    source: "手工创建",
    verified: false,
    execution_mode: executionMode.value,
    steps: steps.value.map(buildStep),
  };
}

function addStep() { steps.value.push(newStep()); }
function removeStep(i: number) { steps.value.splice(i, 1); }
function moveStep(i: number, dir: number) {
  const j = i + dir;
  if (j < 0 || j >= steps.value.length) return;
  const t = steps.value[i];
  steps.value[i] = steps.value[j];
  steps.value[j] = t;
}

function importFromLog() {
  const port = store.activePort;
  const frames = store.framesByPort[port] ?? [];
  const txCmds = frames.filter((f) => f.dir === "tx").map((f) => f.text.trim()).filter((t) => t);
  if (!txCmds.length) {
    message.warning("当前端口暂无发送记录，先在控制台调通一遍流程");
    return;
  }
  const dedup: string[] = [];
  for (const c of txCmds) { if (dedup[dedup.length - 1] !== c) dedup.push(c); }
  steps.value = dedup.map((cmd) => ({ title: cmd, send: cmd, judge: "ok" as const, judgeText: "", timeoutMs: 8000, optional: false }));
  message.success(`已导入 ${dedup.length} 条发送记录为步骤草稿`);
}

async function save() {
  if (!steps.value.length) { message.warning("至少需要一个步骤"); return; }
  if (steps.value.some((s) => s.judge !== "ok" && !s.judgeText.trim())) {
    message.warning("包含/上报判断模式需填写判断文本");
    return;
  }
  const flow = buildFlow();
  saving.value = true;
  try {
    await invoke("save_usecase", { flow });
    await store.reloadKnowledge();
    message.success("用例已保存");
    emit("saved", flow.id);
    emit("update:show", false);
  } catch (e: any) {
    message.error(`保存失败：${e}`);
  } finally {
    saving.value = false;
  }
}

async function removeFlow() {
  if (!props.baseFlow || !props.baseFlow.id.startsWith("my_")) {
    message.warning("只能删除手工创建的用例");
    return;
  }
  saving.value = true;
  try {
    await invoke("delete_usecase", { flowId: props.baseFlow.id });
    await store.reloadKnowledge();
    message.success("已删除");
    emit("saved", "");
    emit("update:show", false);
  } catch (e: any) {
    message.error(`删除失败：${e}`);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <n-drawer :show="show" :width="560" placement="right" @update:show="emit('update:show', $event)">
    <n-drawer-content :title="titleText" closable>
      <div class="editor">
        <div class="form-row">
          <label>用例名称<n-input v-model:value="name" placeholder="如：连自家 MQTT 服务器" /></label>
          <label>分类<n-select v-model:value="category" :options="categoryOptions" /></label>
        </div>
        <label class="applies">
          适用型号（逗号分隔；用于筛选和长稳测试安全校验）
          <n-input v-model:value="appliesTo" placeholder="如 ML307C, ML307R" />
        </label>
        <label class="applies">
          执行影响范围
          <n-select v-model:value="executionMode" :options="[
            { label: '只读检查', value: 'readonly' },
            { label: '可能修改设备状态', value: 'mutating' },
            { label: '尚未确认', value: 'unknown' },
          ]" />
        </label>

        <div class="steps-head">
          <b>步骤列表（{{ steps.length }}）</b>
          <n-button size="small" secondary @click="importFromLog">从串口日志导入</n-button>
        </div>

        <div v-for="(s, i) in steps" :key="i" class="step-box">
          <div class="step-top">
            <span class="step-idx">#{{ i + 1 }}</span>
            <n-input v-model:value="s.title" placeholder="步骤备注（显示为标题）" size="small" />
            <n-button size="tiny" quaternary :disabled="i === 0" @click="moveStep(i, -1)">↑</n-button>
            <n-button size="tiny" quaternary :disabled="i === steps.length - 1" @click="moveStep(i, 1)">↓</n-button>
            <n-button size="tiny" quaternary type="error" @click="removeStep(i)">
              <AppIcon :icon="X" :size="13" />
            </n-button>
          </div>
          <n-input v-model:value="s.send" type="textarea" :autosize="{ minRows: 1, maxRows: 4 }" placeholder="发送指令，如 AT+CSQ（多行=多条）" size="small" />
          <div class="step-judge">
            <n-select v-model:value="s.judge" :options="[
              { label: '终结码 OK', value: 'ok' },
              { label: '响应包含文本', value: 'contains' },
              { label: '等待上报 URC', value: 'urc' },
            ]" size="small" style="width: 140px" />
            <n-input v-if="s.judge !== 'ok'" v-model:value="s.judgeText" size="small" :placeholder="s.judge === 'contains' ? '如 +CSQ: 20' : '如 +MIPOPEN: 0,0'" />
            <n-input-number v-model:value="s.timeoutMs" :min="500" :max="60000" :step="1000" size="small" style="width: 120px"><template #suffix>ms</template></n-input-number>
            <n-checkbox v-model:checked="s.optional">可选</n-checkbox>
          </div>
        </div>

        <n-button dashed block @click="addStep">＋ 添加步骤</n-button>
      </div>

      <template #footer>
        <div class="drawer-footer">
          <n-button v-if="mode === 'edit'" type="error" tertiary :loading="saving" @click="removeFlow">删除</n-button>
          <span style="flex: 1" />
          <n-button @click="emit('update:show', false)">取消</n-button>
          <n-button type="primary" :loading="saving" @click="save">保存</n-button>
        </div>
      </template>
    </n-drawer-content>
  </n-drawer>
</template>

<style scoped>
.editor { display: flex; flex-direction: column; gap: 14px; padding-bottom: 8px; }
.form-row { display: grid; grid-template-columns: 1fr 160px; gap: 12px; }
.form-row label, .applies { display: flex; flex-direction: column; gap: 6px; font-size: 12px; opacity: 0.7; }
.steps-head { display: flex; align-items: center; justify-content: space-between; margin-top: 6px; }
.step-box { border: 1px solid var(--border-default); border-radius: 8px; padding: 10px; display: flex; flex-direction: column; gap: 8px; background: var(--surface-raised); }
.step-top { display: flex; align-items: center; gap: 6px; }
.step-idx { font-weight: 700; opacity: 0.5; font-size: 13px; width: 28px; flex-shrink: 0; }
.step-top .n-input { flex: 1; }
.step-judge { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
.step-judge .n-input { flex: 1; min-width: 120px; }
.drawer-footer { display: flex; align-items: center; gap: 8px; }
</style>
