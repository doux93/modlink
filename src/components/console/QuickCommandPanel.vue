<script setup lang="ts">
import {
  NButton,
  NCheckbox,
  NDropdown,
  NInput,
  NInputNumber,
  NModal,
  NPopconfirm,
  NTab,
  NTabs,
  NTag,
  useMessage,
} from "naive-ui";
import { computed, onUnmounted, ref, watch } from "vue";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Pencil, Trash2 } from "@lucide/vue";
import AppIcon from "../AppIcon.vue";
import { useSerialStore } from "../../stores/serial";
import { useExecutionStore } from "../../stores/execution";
import { useQuickCommandRunner } from "../../composables/useQuickCommandRunner";
import { useAsyncAction } from "../../composables/useAsyncAction";
import type { QuickCommand, QuickCmdsFile } from "../../types";

defineProps<{ floating?: boolean }>();
const emit = defineEmits<{ close: [] }>();
const store = useSerialStore();
const message = useMessage();
const connected = computed(() => store.activeSession?.connected ?? false);

// ---------------- 快捷指令 ----------------
const activeGroup = ref(0);
const executions = useExecutionStore();
const quickRunner = useQuickCommandRunner({
  execute: (target, steps, id) => store.executeAtSequence(target, steps, "quick_command", "stop_on_error", undefined, id),
  write: (target, command) => store.writeTo(target, command.text, command.format === "hex", command.append_newline, "quick_command"),
  stop: async (id) => {
    try { return await executions.stop(id); }
    catch (error) { message.error(`停止任务失败：${error}`); }
  },
});
const { looping, runningTarget } = quickRunner;
watch(() => executions.records.filter((record) => record.status === "running").map((record) => record.executionId),
  (ids) => ids.forEach(quickRunner.onExecutionStarted));
onUnmounted(() => { void quickRunner.stop(); });

// ---- 编辑（增删改 + 分组管理，全部自动持久化） ----
const showEdit = ref(false);
const editIdx = ref(-1); // -1 = 新增
const editDraft = ref<QuickCommand>(blankCmd());

function blankCmd(): QuickCommand {
  return { text: "", format: "text", append_newline: true, selected: false, note: "" };
}

function ensureQuickCmds() {
  if (!store.quickCmds) {
    store.quickCmds = {
      schema: "modulink/quickcmds@1",
      name: "我的指令集",
      groups: [{ name: "默认分组", items: [] }],
      loop: { times: 1, interval_ms: 300 },
      warnings: [],
    };
  }
}

function openEdit(idx: number) {
  ensureQuickCmds();
  editIdx.value = idx;
  editDraft.value =
    idx >= 0
      ? { ...store.quickCmds!.groups[activeGroup.value].items[idx] }
      : blankCmd();
  showEdit.value = true;
}

function saveEdit() {
  if (!editDraft.value.text.trim()) {
    message.warning("指令内容不能为空");
    return;
  }
  const items = store.quickCmds!.groups[activeGroup.value].items;
  if (editIdx.value >= 0) {
    items[editIdx.value] = { ...editDraft.value };
  } else {
    items.push({ ...editDraft.value });
  }
  showEdit.value = false;
  store.persistQuickCmds();
}

function removeCmd(idx: number) {
  store.quickCmds!.groups[activeGroup.value].items.splice(idx, 1);
  store.persistQuickCmds();
}

function moveCmd(idx: number, delta: number) {
  const items = store.quickCmds!.groups[activeGroup.value].items;
  const to = idx + delta;
  if (to < 0 || to >= items.length) return;
  [items[idx], items[to]] = [items[to], items[idx]];
  store.persistQuickCmds();
}

// ---- 分组管理 ----
const showGroupEdit = ref(false);
const groupNameDraft = ref("");

function addGroup() {
  ensureQuickCmds();
  store.quickCmds!.groups.push({ name: `分组 ${store.quickCmds!.groups.length + 1}`, items: [] });
  activeGroup.value = store.quickCmds!.groups.length - 1;
  store.persistQuickCmds();
}

function openGroupRename() {
  groupNameDraft.value = store.quickCmds!.groups[activeGroup.value].name;
  showGroupEdit.value = true;
}

function saveGroupRename() {
  if (groupNameDraft.value.trim()) {
    store.quickCmds!.groups[activeGroup.value].name = groupNameDraft.value.trim();
    store.persistQuickCmds();
  }
  showGroupEdit.value = false;
}

function removeGroup() {
  store.quickCmds!.groups.splice(activeGroup.value, 1);
  if (!store.quickCmds!.groups.length) {
    store.quickCmds!.groups.push({ name: "默认分组", items: [] });
  }
  activeGroup.value = Math.max(0, activeGroup.value - 1);
  store.persistQuickCmds();
}

const { busy: transferring, error: transferError, run: transfer } = useAsyncAction("配置文件操作失败", (error) => message.error(error));
function importIni() {
  return transfer(async () => {
    const path = await openDialog({ filters: [{ name: "老工具配置", extensions: ["ini"] }] });
    if (!path) return;
    await store.importLegacy(path as string);
    activeGroup.value = 0;
    const file = store.quickCmds!;
    message.success(`导入成功：${file.groups.length} 组 / ${file.groups.reduce((n, g) => n + g.items.length, 0)} 条指令`);
    for (const warning of file.warnings) message.warning(warning);
  });
}
function exportIni() {
  return transfer(async () => {
    if (!store.quickCmds) return;
    const data: QuickCmdsFile = JSON.parse(JSON.stringify(store.quickCmds));
    const path = await saveDialog({ defaultPath: `${data.name}_导出.ini`, filters: [{ name: "老工具配置", extensions: ["ini"] }] });
    if (!path) return;
    await store.exportLegacy(path, data);
    message.success("已导出为老工具 ini 格式");
  });
}

async function sendQuick(c: QuickCommand) {
  if (!connected.value) {
    message.warning("请先连接串口");
    return;
  }
  try {
    await store.write(c.text, c.format === "hex", c.append_newline, "quick_command");
  } catch (error) {
    message.error(`发送失败：${error}`);
  }
}

async function sendSelected(loop: boolean) {
  const f = store.quickCmds;
  if (!f || !connected.value) return;
  const items = f.groups[activeGroup.value]?.items.filter((i) => i.selected).map((item) => ({ ...item })) ?? [];
  if (!items.length) {
    message.warning("当前分组没有勾选的指令");
    return;
  }
  try {
    const target = store.executionTarget(store.activePort);
    const times = loop ? f.loop.times : 1;
    const interval = f.loop.interval_ms;
    await executions.init();
    await quickRunner.run(target, items, times, interval);
  } catch (error) {
    message.error(`批量发送已停止：${error}`);
  }
}
</script>

<template>
  <aside class="quick" :class="{ 'quick-floating': floating }" aria-label="快捷指令" @keydown.esc.stop="emit('close')">
    <div class="quick-head">
      <b>快捷指令</b>
      <div>
        <n-button size="tiny" quaternary :disabled="transferring" @click="importIni">导入老 ini</n-button>
        <n-button size="tiny" quaternary :disabled="!store.quickCmds || transferring" @click="exportIni">
          导出
        </n-button>
        <n-button v-if="floating" size="tiny" quaternary aria-label="关闭快捷指令面板" @click="emit('close')">关闭</n-button>
      </div>
    </div>
    <p v-if="floating" class="floating-hint">临时展开，关闭后继续查看完整终端。</p>
    <p v-if="transferError" class="transfer-error" role="alert">{{ transferError }}；可重新操作。</p>
    <template v-if="store.quickCmds">
      <div class="group-bar">
        <n-tabs v-model:value="activeGroup" type="line" size="small" class="tabs-flex">
          <n-tab v-for="(g, i) in store.quickCmds.groups" :key="i" :name="i">
            {{ g.name }}
          </n-tab>
        </n-tabs>
        <n-dropdown
          trigger="click"
          :options="[
            { label: '新建分组', key: 'add' },
            { label: '重命名当前分组', key: 'rename' },
            { label: '删除当前分组', key: 'remove' },
          ]"
          @select="(k: string) => (k === 'add' ? addGroup() : k === 'rename' ? openGroupRename() : removeGroup())"
        >
          <n-button size="tiny" quaternary>⋯</n-button>
        </n-dropdown>
      </div>
      <div class="quick-list">
        <div
          v-for="(c, i) in store.quickCmds.groups[activeGroup]?.items"
          :key="i"
          class="quick-item"
        >
          <n-checkbox v-model:checked="c.selected" size="small" @update:checked="store.persistQuickCmds()" />
          <span class="quick-text" :title="c.note ? `${c.note}\n${c.text}` : c.text" @dblclick="openEdit(i)">
            <n-tag v-if="c.format === 'hex'" size="tiny" type="warning" :bordered="false">
              HEX
            </n-tag>
            {{ c.note || c.text }}
          </span>
          <span class="item-ops">
            <n-button size="tiny" quaternary aria-label="上移指令" @click="moveCmd(i, -1)">↑</n-button>
            <n-button size="tiny" quaternary aria-label="下移指令" @click="moveCmd(i, 1)">↓</n-button>
            <n-button size="tiny" quaternary aria-label="编辑指令" @click="openEdit(i)">
              <AppIcon :icon="Pencil" :size="13" />
            </n-button>
            <n-popconfirm @positive-click="removeCmd(i)">
              <template #trigger>
                <n-button size="tiny" quaternary aria-label="删除指令">
                  <AppIcon :icon="Trash2" :size="13" />
                </n-button>
              </template>
              删除这条指令？
            </n-popconfirm>
          </span>
          <n-button size="tiny" secondary type="primary" :disabled="!connected" @click="sendQuick(c)">发送</n-button>
        </div>
        <n-button size="tiny" dashed block class="mt-6" @click="openEdit(-1)">
          ＋ 新增指令
        </n-button>
      </div>
      <div class="quick-foot">
        <span class="loop-cfg">
          循环
          <n-input-number
            v-model:value="store.quickCmds.loop.times"
            size="tiny"
            :min="1"
            style="width: 80px"
            @update:value="store.persistQuickCmds()"
          />
          次 / 间隔
          <n-input-number
            v-model:value="store.quickCmds.loop.interval_ms"
            size="tiny"
            :min="0"
            :step="100"
            style="width: 90px"
            @update:value="store.persistQuickCmds()"
          />
          ms
        </span>
        <span v-if="runningTarget" class="kb-hint">执行目标：{{ runningTarget.port }}</span>
        <n-button v-if="!looping" size="tiny" type="primary" @click="sendSelected(true)">
          发送选中
        </n-button>
        <n-button v-else size="tiny" type="error" @click="quickRunner.stop()">停止</n-button>
      </div>
    </template>
    <div v-else class="quick-empty">
      <p>还没有快捷指令</p>
      <n-button size="small" type="primary" ghost :loading="transferring" @click="importIni">
        导入 ini 配置
      </n-button>
      <n-button size="small" quaternary @click="openEdit(-1)">或手动新建一条</n-button>
      <small>支持旧版 UART 配置，也可手动添加常用指令。</small>
    </div>

<!-- 指令编辑弹窗 -->
<n-modal v-model:show="showEdit" preset="card" style="width: 520px"
  :title="editIdx >= 0 ? '编辑指令' : '新增指令'">
  <div class="edit-form">
    <n-input
      v-model:value="editDraft.text"
      type="textarea"
      :rows="3"
      :placeholder="editDraft.format === 'hex' ? 'Hex 数据，如 1A 2B 3C' : 'AT 指令或数据'"
    />
    <n-input v-model:value="editDraft.note" placeholder="备注（可选，显示为按钮名称）" />
    <div class="edit-row">
      <n-checkbox :checked="editDraft.format === 'hex'"
        @update:checked="(v: boolean) => (editDraft.format = v ? 'hex' : 'text')">
        Hex 发送
      </n-checkbox>
      <n-checkbox v-model:checked="editDraft.append_newline">追加回车换行</n-checkbox>
      <span class="edit-delay">
        延时
        <n-input-number v-model:value="editDraft.delay_ms" size="small" :min="0" :step="100"
          style="width: 110px" placeholder="默认" clearable />
        ms
      </span>
    </div>
  </div>
  <template #footer>
    <div class="modal-actions">
      <n-button @click="showEdit = false">取消</n-button>
      <n-button type="primary" @click="saveEdit">保存</n-button>
    </div>
  </template>
</n-modal>

<!-- 分组重命名弹窗 -->
<n-modal v-model:show="showGroupEdit" preset="card" style="width: 380px" title="重命名分组">
  <n-input v-model:value="groupNameDraft" @keydown.enter="saveGroupRename" />
  <template #footer>
    <div class="modal-actions">
      <n-button @click="showGroupEdit = false">取消</n-button>
      <n-button type="primary" @click="saveGroupRename">保存</n-button>
    </div>
  </template>
</n-modal>
  </aside>
</template>

<style scoped>
.quick {
  min-width: 200px; flex-shrink: 0;
  display: flex; flex-direction: column; padding: 10px 10px 10px 4px;
}
.quick-floating { position: absolute; right: 8px; top: 4px; bottom: 8px; z-index: 10; padding: 12px; background: var(--surface-raised); border: 1px solid var(--border-default); border-radius: 10px; box-shadow: var(--shadow-lg); }
.floating-hint { margin: 0 0 8px; font-size: 12px; line-height: 1.5; color: var(--text-secondary); }
.quick-head {
  display: flex; flex-wrap: wrap; gap: 4px; justify-content: space-between; align-items: center;
  padding: 2px 2px 8px; color: var(--text-primary);
}
.quick-list { flex: 1; overflow-y: auto; margin-top: 6px; }
.group-bar { display: flex; align-items: center; gap: 4px; }
.quick-item {
  display: flex; align-items: center; gap: 7px;
  padding: 5px 4px; border-radius: 6px;
  color: var(--text-primary);
}
.quick-item:hover { background: var(--surface-hover); }
.item-ops { display: none; flex-shrink: 0; }
.quick-item:hover .item-ops, .quick-item:focus-within .item-ops { display: inline-flex; }
.edit-form { display: flex; flex-direction: column; gap: 12px; }
.edit-row { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
.edit-delay { display: flex; align-items: center; gap: 6px; font-size: 13px; }
.quick-text {
  flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-family: Consolas, monospace; font-size: 12.5px;
  color: var(--text-primary);
}
.quick-foot {
  display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding-top: 8px;
  border-top: 1px solid var(--border-subtle);
}
.loop-cfg {
  display: flex; flex: 1 1 250px; flex-wrap: wrap; align-items: center; gap: 4px;
  font-size: 12px; color: var(--text-secondary);
}
.quick-empty {
  flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 12px; color: var(--text-secondary); text-align: center; padding: 0 16px;
}
.transfer-error { color: var(--status-danger); font-size: 12px; overflow-wrap: anywhere; }
.quick-empty small { color: var(--text-tertiary); line-height: 1.5; }

.tabs-flex { flex: 1; min-width: 0; }
.mt-6 { margin-top: 6px; }
.modal-actions { display: flex; justify-content: flex-end; gap: 8px; }
.kb-hint { color: var(--text-secondary); }
</style>
