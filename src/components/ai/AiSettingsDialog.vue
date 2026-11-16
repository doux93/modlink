<script setup lang="ts">
import { NButton, NInput, NInputNumber, NModal, NPopconfirm } from "naive-ui";
import type { useAiSettings } from "../../composables/useAiSettings";
import type { ManualContentLevel } from "../../types";
const props = defineProps<{ controller: ReturnType<typeof useAiSettings>; manualMode: ManualContentLevel }>();
const { draft, show, loaded, revealApiKey, busy, mutating, error, load, setOpen, save, clearApiKey } = props.controller;
</script>
<template>
    <!-- 配置弹窗 -->
    <n-modal :show="show" @update:show="setOpen" :closable="!mutating" :mask-closable="!mutating" :close-on-esc="!mutating" preset="card" class="cfg-modal" title="AI 与知识库配置">
      <div class="cfg">
        <n-input :disabled="busy || !loaded" v-model:value="draft.base_url" placeholder="Base URL，如 https://onerouter.cmaiot.cn/v1" />
        <div class="cfg-key-row">
          <n-input
            :disabled="busy || !loaded" v-model:value="draft.api_key"
            :type="revealApiKey ? 'text' : 'password'"
            :placeholder="draft.api_key_configured ? 'API Key 已配置，留空表示不修改' : 'API Key'"
            class="cfg-key-input"
          />
          <n-button
            size="small"
            quaternary
            :title="revealApiKey ? '隐藏 API Key' : '明文显示 API Key（请确认环境安全）'"
            @click="revealApiKey = !revealApiKey"
          >
            {{ revealApiKey ? "隐藏" : "显示" }}
          </n-button>
        </div>
        <small v-if="draft.api_key_configured" class="cfg-note">API key 保存在 Windows 系统凭据中</small>
        <div class="cfg-row">
          <n-input :disabled="busy || !loaded" v-model:value="draft.model" placeholder="模型，如 auto" class="cfg-input-model" />
          <n-input-number :disabled="busy || !loaded" v-model:value="draft.recent_log_lines" :min="20" :max="300" class="cfg-input-num">
            <template #suffix>行日志</template>
          </n-input-number>
        </div>
        <div class="cfg-note">
          手册知识包管理已迁移至「知识中心」页面，当前型号模式：{{ manualMode === "fulltext" ? "全文" : manualMode === "preview" ? "摘要" : "不可用" }}
        </div>
        <small class="cfg-note">连接地址和模型保存在应用配置目录，API key 不写入配置文件</small>
      </div>
      <p v-if="error" class="cfg-error" role="alert">{{ error }}；输入已保留，可重试。</p>
      <n-button v-if="!loaded && !busy" size="small" @click="load">重新读取配置</n-button>
      <template #footer>
        <div class="cfg-actions">
          <n-popconfirm v-if="draft.api_key_configured" @positive-click="clearApiKey">
            <template #trigger><n-button type="error" tertiary :disabled="busy || !loaded">清除 API Key</n-button></template>
            确认删除系统凭据中的 API key？
          </n-popconfirm>
          <n-button :disabled="mutating" @click="setOpen(false)">取消</n-button>
          <n-button type="primary" :loading="mutating" :disabled="busy || !loaded" @click="save">保存</n-button>
        </div>
      </template>
    </n-modal>
</template>
<style scoped>
.cfg { display: flex; flex-direction: column; gap: 12px; }
.cfg-modal { width: 520px; }
.cfg-note { color: var(--text-tertiary); font-size: 13px; }
.cfg-row { display: flex; gap: 10px; }
.cfg-row > :first-child { flex: 1; }
.cfg-actions { display: flex; justify-content: flex-end; gap: 8px; }
.cfg-key-row { display: flex; align-items: center; gap: 8px; }
.cfg-key-row .cfg-key-input { flex: 1; min-width: 0; }
.cfg-input-model { flex: 1; }
.cfg-input-num { width: 160px; }
.cfg-error { color: var(--status-danger); font-size: 13px; overflow-wrap: anywhere; }
</style>
