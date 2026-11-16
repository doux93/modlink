<script setup lang="ts">
import { computed, onActivated, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useMessage } from "naive-ui";
import { useSerialStore } from "../stores/serial";
import type {
  KnowledgeCapabilities,
  KnowledgePackManifest,
  KnowledgePackValidation,
  ManualContentLevel,
} from "../types";

const message = useMessage();
const serialStore = useSerialStore();
const caps = ref<KnowledgeCapabilities | null>(null);
const packDir = ref("");
const configuredPackDir = ref("");
const validation = ref<KnowledgePackValidation | null>(null);
const activePackValidation = ref<KnowledgePackValidation | null>(null);
const validating = ref(false);
const refreshing = ref(false);
const choosingDir = ref(false);
let refreshQueued = false;
const KNOWLEDGE_CAPABILITIES_CHANGED = "modulink:knowledge-capabilities-changed";

async function refresh() {
  if (refreshing.value) {
    refreshQueued = true;
    return;
  }
  refreshing.value = true;
  try {
    const requestedProfileId = serialStore.activeProfileId || null;
    const [settingsResult, capsResult] = await Promise.allSettled([
      invoke<any>("get_settings"),
      invoke<KnowledgeCapabilities>("get_knowledge_capabilities", {
        profileId: requestedProfileId,
      }),
    ]);

    if (settingsResult.status === "fulfilled") {
      const previousConfiguredDir = configuredPackDir.value;
      const nextConfiguredDir = settingsResult.value?.kb?.manuals_dir ?? "";
      configuredPackDir.value = nextConfiguredDir;
      if (!packDir.value || packDir.value === previousConfiguredDir) {
        packDir.value = nextConfiguredDir;
      }

      if (nextConfiguredDir) {
        try {
          activePackValidation.value = await invoke<KnowledgePackValidation>(
            "validate_knowledge_pack",
            { packDir: nextConfiguredDir },
          );
        } catch {
          activePackValidation.value = null;
        }
      } else {
        activePackValidation.value = null;
      }
    }

    caps.value = capsResult.status === "fulfilled" ? capsResult.value : null;
    window.dispatchEvent(new Event(KNOWLEDGE_CAPABILITIES_CHANGED));
  } finally {
    refreshing.value = false;
    if (refreshQueued) {
      refreshQueued = false;
      void refresh();
    }
  }
}

async function chooseDir() {
  if (choosingDir.value) return;
  choosingDir.value = true;
  try {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected === "string") {
      packDir.value = selected;
      validation.value = null;
    }
  } catch (e) {
    message.error(`无法打开目录选择器：${String(e)}`);
  } finally {
    choosingDir.value = false;
  }
}

async function validatePack() {
  if (!packDir.value.trim()) {
    message.warning("请先选择知识包目录");
    return;
  }
  validating.value = true;
  try {
    validation.value = await invoke<KnowledgePackValidation>("validate_knowledge_pack", {
      packDir: packDir.value.trim(),
    });
    if (validation.value.valid) {
      message.success("知识包验证通过，等待激活");
    } else {
      message.warning("验证未通过，请查看详情");
    }
  } catch (e) {
    message.error(String(e));
  } finally {
    validating.value = false;
  }
}

async function activatePack() {
  if (!packDir.value.trim()) {
    message.warning("请先选择知识包目录");
    return;
  }
  validating.value = true;
  try {
    const result = await invoke<KnowledgePackValidation>(
      "validate_and_activate_knowledge_pack",
      { packDir: packDir.value.trim() },
    );
    validation.value = result;
    if (result.valid) {
      await refresh();
      message.success("型号手册扩展已验证并激活");
    } else {
      message.warning("验证未通过，未保存配置");
    }
  } catch (e) {
    message.error(String(e));
  } finally {
    validating.value = false;
  }
}

async function deactivatePack() {
  try {
    await invoke("save_settings", {
      settings: { kb: { manuals_dir: "" } },
    });
    validation.value = null;
    activePackValidation.value = null;
    configuredPackDir.value = "";
    packDir.value = "";
    await refresh();
    message.success("已停用型号手册扩展，当前设备使用内置摘要");
  } catch (e) {
    message.error(String(e));
  }
}

function modeLabel(mode: ManualContentLevel | null | undefined): string {
  if (mode === "fulltext") return "全文可用";
  if (mode === "preview") return "摘要可用";
  return "手册不可用";
}

function modeType(mode: ManualContentLevel | null | undefined): "success" | "warning" | "error" {
  if (mode === "fulltext") return "success";
  if (mode === "preview") return "warning";
  return "error";
}

const effectiveMode = computed(
  () => caps.value?.effectiveManualMode ?? caps.value?.manualMode ?? "none",
);
const packageMode = computed(() => caps.value?.manualMode ?? "none");
const effectiveReason = computed(
  () => caps.value?.effectiveDegradationReason ?? caps.value?.degradationReason ?? "",
);
const usesLegacyEffectiveMode = computed(
  () => Boolean(caps.value && caps.value.effectiveManualMode == null),
);
const manualExtension = computed(
  () => caps.value?.installedPacks.find((pack) => pack.id !== "cmiot.core.builtin") ?? null,
);
const activeProfileLabel = computed(() => {
  const profileId = caps.value?.effectiveProfileId || serialStore.activeProfileId;
  const profile = serialStore.knowledge?.profiles.find((item) => item.id === profileId);
  if (!profileId || profileId === "generic") return "未识别具体型号";
  if (!profile) return profileId;
  return profile.name === profileId ? profile.name : `${profile.name} (${profileId})`;
});
const packageValidationLabel = computed(() => {
  if (!configuredPackDir.value) return "未配置";
  if (activePackValidation.value?.valid || manualExtension.value) return "验证通过";
  if (activePackValidation.value) return "验证失败";
  return "状态未知";
});
const packageValidationType = computed<"success" | "warning" | "error" | "default">(() => {
  if (!configuredPackDir.value) return "default";
  if (activePackValidation.value?.valid || manualExtension.value) return "success";
  if (activePackValidation.value) return "error";
  return "warning";
});
const displayedValidation = computed(() => {
  if (packDir.value === configuredPackDir.value) {
    return validation.value ?? activePackValidation.value;
  }
  return validation.value;
});
const displayedValidationTitle = computed(() =>
  packDir.value === configuredPackDir.value ? "已激活包验证" : "候选包验证",
);

function shortModels(models: string[] | null | undefined, max: number = 8): string {
  if (!models?.length) return "-";
  if (models.length <= max) return models.join(", ");
  return `${models.slice(0, max).join(", ")} 等 ${models.length} 个`;
}

function isV3Manifest(manifest: KnowledgePackManifest): boolean {
  return manifest.schema === "modulink/knowledge-pack@3";
}

function scopeLabel(manifest: KnowledgePackManifest): string {
  const scopes = ["精确型号"];
  if (manifest.includeSeries) scopes.push("同系列参考");
  if (isV3Manifest(manifest) && manifest.includeUnknownReference) {
    scopes.push("unknown 参考");
  } else if (!isV3Manifest(manifest) && manifest.includeGeneric) {
    scopes.push("通用资料");
  }
  return scopes.join(" + ");
}

interface ProbeResult {
  command: string;
  finalCode: string;
  responseLines: string[];
  recorded: boolean;
}

interface CapabilityObservation {
  profileId: string;
  port: string;
  firmware: string | null;
  command: string;
  query: string;
  responseLines: string[];
  updatedAt: number;
}

interface CapabilityEvidenceStore {
  schema: string;
  portFirmware: Record<string, { profileId: string; firmware: string; updatedAt: number }>;
  observations: CapabilityObservation[];
}

const probing = ref(false);
const probeResults = ref<ProbeResult[]>([]);
const evidenceStore = ref<CapabilityEvidenceStore | null>(null);

async function runProbe() {
  const port = serialStore.activePort;
  const profileId = serialStore.activeProfileId;
  if (!port) {
    message.warning("请先在控制台打开串口");
    return;
  }
  if (!profileId || profileId === "generic") {
    message.warning("请先识别或选择当前设备的型号画像");
    return;
  }
  probing.value = true;
  probeResults.value = [];
  try {
    probeResults.value = await invoke<ProbeResult[]>("probe_capabilities", {
      port,
      profileId,
    });
    const recorded = probeResults.value.filter((result) => result.recorded).length;
    message.success(`探测完成：${probeResults.value.length} 条指令，${recorded} 条已记录`);
    await loadEvidence();
  } catch (e) {
    message.error(String(e));
  } finally {
    probing.value = false;
  }
}

async function loadEvidence() {
  try {
    evidenceStore.value = await invoke<CapabilityEvidenceStore>(
      "list_capability_observations",
      {},
    );
  } catch {
    /* 实测证据不可用不影响知识包状态。 */
  }
}

const observationCount = computed(() => evidenceStore.value?.observations.length ?? 0);

onActivated(async () => {
  await Promise.all([refresh(), loadEvidence()]);
});

watch(
  () => serialStore.activeProfileId,
  () => void refresh(),
);
</script>

<template>
  <div class="kb-page">
    <header class="kb-header">
      <div>
        <h2>知识中心</h2>
        <p>内置知识 + 型号手册扩展</p>
      </div>
      <n-button size="small" :loading="refreshing" @click="refresh">刷新状态</n-button>
    </header>

    <n-card title="当前设备知识能力" size="small" class="kb-card">
      <div class="effective-summary">
        <div>
          <div class="summary-label">当前型号</div>
          <div class="summary-value">{{ activeProfileLabel }}</div>
        </div>
        <n-tag :type="modeType(effectiveMode)" size="small" :bordered="false">
          {{ modeLabel(effectiveMode) }}
        </n-tag>
      </div>

      <div v-if="effectiveReason" class="kb-notice" :class="{ warning: effectiveMode !== 'fulltext' }">
        {{ effectiveReason }}
      </div>
      <div v-if="usesLegacyEffectiveMode" class="kb-compat-note">
        当前后端未返回型号级判定，暂按包级状态显示。
      </div>

      <n-divider />
      <div class="kb-row">
        <span>知识来源</span>
        <span class="source-model">内置知识 <b>+</b> 型号手册扩展</span>
      </div>
      <div class="kb-row">
        <span>内置知识</span>
        <span>
          <n-tag :type="caps?.coreAvailable ? 'success' : 'error'" size="small" :bordered="false">
            {{ caps?.coreAvailable ? "已就绪" : "未就绪" }}
          </n-tag>
        </span>
      </div>
      <div class="kb-row">
        <span>型号手册扩展</span>
        <span>{{ manualExtension ? `${manualExtension.id} v${manualExtension.version}` : "未激活" }}</span>
      </div>
      <div class="kb-row">
        <span>扩展包验证</span>
        <span>
          <n-tag :type="packageValidationType" size="small" :bordered="false">
            {{ packageValidationLabel }}
          </n-tag>
        </span>
      </div>
      <div class="kb-row">
        <span>包级手册状态</span>
        <span>
          <n-tag :type="modeType(packageMode)" size="small" :bordered="false">
            {{ modeLabel(packageMode) }}
          </n-tag>
        </span>
      </div>
      <div class="kb-row">
        <span>内置索引主题</span>
        <span>{{ caps?.topicCount ?? 0 }} 条</span>
      </div>
      <div class="kb-row">
        <span>画像覆盖</span>
        <span>{{ shortModels(caps?.modelCoverage) }}</span>
      </div>
      <div v-if="caps?.fulltextModelCoverage?.length" class="kb-row">
        <span>扩展包精确型号</span>
        <span>{{ shortModels(caps.fulltextModelCoverage) }}</span>
      </div>
    </n-card>

    <n-card title="型号手册扩展" size="small" class="kb-card">
      <div class="kb-row kb-dir-row">
        <span>知识包目录</span>
        <div class="kb-dir-input">
          <n-input
            v-model:value="packDir"
            placeholder="选择包含 manifest.json 和 topic_paths.jsonl 的目录"
            size="small"
            @update:value="validation = null"
          />
          <n-button size="small" :loading="choosingDir" @click="chooseDir">浏览</n-button>
        </div>
      </div>
      <div class="kb-actions">
        <n-button size="small" :loading="validating" @click="validatePack">验证</n-button>
        <n-button
          size="small"
          type="primary"
          :disabled="!validation?.valid"
          :loading="validating"
          @click="activatePack"
        >激活扩展</n-button>
        <n-button size="small" :disabled="!configuredPackDir" @click="deactivatePack">停用</n-button>
      </div>

      <div v-if="displayedValidation" class="kb-validation">
        <n-divider />
        <div class="kb-row">
          <span>{{ displayedValidationTitle }}</span>
          <span class="validation-state">
            <n-tag :type="displayedValidation.valid ? 'success' : 'error'" size="small" :bordered="false">
              {{ displayedValidation.valid ? "通过" : "未通过" }}
            </n-tag>
            <n-tag
              v-if="displayedValidation.valid && packDir !== configuredPackDir"
              size="small"
              type="warning"
              :bordered="false"
            >待激活</n-tag>
          </span>
        </div>
        <div v-if="displayedValidation.manifest" class="kb-manifest">
          <div class="kb-row"><span>包 ID</span><span>{{ displayedValidation.manifest.id }}</span></div>
          <div class="kb-row"><span>版本</span><span>{{ displayedValidation.manifest.version }}</span></div>
          <div class="kb-row"><span>适用型号</span><span>{{ shortModels(displayedValidation.manifest.models) }}</span></div>
          <div v-if="displayedValidation.manifest.schema !== 'modulink/knowledge-pack@1'" class="kb-row">
            <span>包含范围</span>
            <span>{{ scopeLabel(displayedValidation.manifest) }}</span>
          </div>
          <div v-if="displayedValidation.manifest.topicCounts" class="scope-counts">
            <div>
              <b>{{ displayedValidation.manifest.topicCounts.exact }}</b>
              <span>exact 精确</span>
            </div>
            <div>
              <b>{{ displayedValidation.manifest.topicCounts.series }}</b>
              <span>series 参考</span>
            </div>
            <div v-if="isV3Manifest(displayedValidation.manifest)">
              <b>{{ displayedValidation.manifest.topicCounts.unknown ?? 0 }}</b>
              <span>unknown 参考</span>
            </div>
            <div v-else-if="displayedValidation.manifest.topicCounts.generic != null">
              <b>{{ displayedValidation.manifest.topicCounts.generic }}</b>
              <span>generic 通用</span>
            </div>
          </div>
          <div class="kb-row"><span>生成时间</span><span>{{ displayedValidation.manifest.generatedAt || "-" }}</span></div>
          <div class="kb-row"><span>主题数</span><span>{{ displayedValidation.manifest.topicCount }}</span></div>
        </div>
        <div class="kb-row"><span>路径映射</span><span>{{ displayedValidation.topicPathsCount }} 条</span></div>
        <div class="kb-row"><span>采样可读</span><span>{{ displayedValidation.sampleReadable }} / {{ displayedValidation.sampleTotal }}</span></div>
        <div v-if="displayedValidation.errors.length" class="kb-errors">
          <div v-for="(error, index) in displayedValidation.errors" :key="index" class="kb-error">
            {{ error }}
          </div>
        </div>
      </div>
    </n-card>

    <n-card title="证据适用范围" size="small" class="kb-card">
      <div class="scope-legend">
        <div class="scope-item exact">
          <n-tag size="small" type="success" :bordered="false">exact</n-tag>
          <div><b>精确型号</b><span>当前型号直接证据；AT 执行仍由后端独立审核。</span></div>
        </div>
        <div class="scope-item series">
          <n-tag size="small" type="warning" :bordered="false">series</n-tag>
          <div><b>同系列参考</b><span>可辅助回答，不为兄弟型号授权 AT 执行。</span></div>
        </div>
        <div class="scope-item unknown">
          <n-tag size="small" :bordered="false">unknown</n-tag>
          <div><b>范围未解析</b><span>仅供检索参考，不作为型号结论或 AT 授权依据。</span></div>
        </div>
        <div class="scope-item runtime">
          <n-tag size="small" type="info" :bordered="false">实测</n-tag>
          <div><b>设备观测</b><span>按型号、固件和串口隔离，冲突时优先于手册通用描述。</span></div>
        </div>
      </div>
    </n-card>

    <n-card title="设备实测证据" size="small" class="kb-card">
      <div class="evidence-toolbar">
        <span>已记录 {{ observationCount }} 条观测</span>
        <n-button type="primary" size="small" :loading="probing" @click="runProbe">
          {{ probing ? "探测中..." : "探测当前设备" }}
        </n-button>
      </div>

      <n-data-table
        v-if="probeResults.length"
        :data="probeResults"
        size="small"
        :max-height="300"
        :columns="[
          { title: '指令', key: 'command', width: 180 },
          { title: '结果', key: 'finalCode', width: 100 },
          { title: '已记录', key: 'recorded', width: 80, render: (row: ProbeResult) => row.recorded ? '是' : '—' },
          { title: '响应', key: 'responseLines', render: (row: ProbeResult) => row.responseLines.join(' | ') },
        ]"
      />

      <n-collapse v-if="evidenceStore?.observations.length" class="evidence-history">
        <n-collapse-item title="历史实测观测" name="observations">
          <n-data-table
            :data="evidenceStore.observations"
            size="small"
            :max-height="400"
            :columns="[
              { title: '指令', key: 'command', width: 160 },
              { title: '型号', key: 'profileId', width: 100 },
              { title: '串口', key: 'port', width: 100 },
              { title: '固件', key: 'firmware', width: 120 },
              { title: '探测查询', key: 'query', width: 180 },
              { title: '响应', key: 'responseLines', render: (row: CapabilityObservation) => row.responseLines.join(' | ') },
            ]"
          />
        </n-collapse-item>
      </n-collapse>
    </n-card>
  </div>
</template>

<style scoped>
.kb-page {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  width: min(880px, 100%);
  padding: 16px 20px 28px;
}
.kb-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 16px;
}
.kb-header h2 { margin: 0; }
.kb-header p { margin: 4px 0 0; color: var(--text-secondary); font-size: 13px; }
.kb-card { margin-bottom: 16px; }
.effective-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  min-height: 48px;
}
.summary-label { color: var(--text-secondary); font-size: 12px; }
.summary-value { margin-top: 3px; font-size: 17px; font-weight: 600; }
.kb-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  min-height: 32px;
  padding: 4px 0;
}
.kb-row > span:first-child { flex-shrink: 0; color: var(--text-secondary); }
.kb-row > span:last-child { min-width: 0; text-align: right; overflow-wrap: anywhere; }
.source-model { font-weight: 600; }
.source-model b { padding: 0 5px; color: var(--brand-primary); }
.kb-notice {
  margin-top: 10px;
  padding: 8px 10px;
  border-left: 3px solid var(--status-success);
  background: var(--status-success-soft);
  font-size: 13px;
}
.kb-notice.warning {
  border-left-color: var(--status-warning);
  background: var(--status-warning-soft);
}
.kb-compat-note { margin-top: 8px; color: var(--text-secondary); font-size: 12px; }
.kb-dir-row { align-items: flex-start; }
.kb-dir-input { display: flex; flex: 1; gap: 8px; min-width: 0; }
.kb-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 12px; }
.kb-validation { margin-top: 8px; }
.kb-manifest { padding: 4px 0; }
.validation-state { display: flex; justify-content: flex-end; gap: 6px; flex-wrap: wrap; }
.scope-counts {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 1px;
  margin: 8px 0;
  background: var(--border-subtle);
  border: 1px solid var(--border-subtle);
}
.scope-counts > div { padding: 9px 10px; background: var(--surface-raised); }
.scope-counts b, .scope-counts span { display: block; }
.scope-counts b { font-size: 16px; color: var(--text-primary); }
.scope-counts span { margin-top: 2px; color: var(--text-tertiary); font-size: 12px; }
.kb-errors { margin-top: 8px; }
.kb-error { padding: 3px 0; color: var(--status-danger); font-size: 13px; }
.scope-legend { display: grid; gap: 0; }
.scope-item {
  display: grid;
  grid-template-columns: 72px 1fr;
  align-items: start;
  gap: 10px;
  padding: 10px 0;
  border-bottom: 1px solid var(--border-subtle);
}
.scope-item:last-child { border-bottom: 0; }
.scope-item b, .scope-item span { display: block; }
.scope-item b { color: var(--text-primary); }
.scope-item span { margin-top: 2px; color: var(--text-tertiary); font-size: 12px; }
.evidence-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}
.evidence-toolbar > span { color: var(--text-secondary); font-size: 13px; }
.evidence-history { margin-top: 12px; }

@media (max-width: 640px) {
  .kb-page { padding: 12px; }
  .kb-row, .kb-dir-row { align-items: stretch; flex-direction: column; gap: 4px; }
  .kb-row > span:last-child { text-align: left; }
  .kb-dir-input { width: 100%; }
  .scope-counts { grid-template-columns: 1fr; }
  .scope-item { grid-template-columns: 64px 1fr; }
}
</style>
