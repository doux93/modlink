<script setup lang="ts">
import {
  NButton,
  NConfigProvider,
  NDialogProvider,
  NMessageProvider,
  NSwitch,
  darkTheme,
  zhCN,
  dateZhCN,
  type GlobalTheme,
} from "naive-ui";
import { computed, defineAsyncComponent, ref, watch } from "vue";
import {
  Terminal,
  FlaskConical,
  Bot,
  BookOpen,
  Wrench,
  PanelLeftClose,
  PanelLeftOpen,
  History,
  Sun,
  Moon,
  CircleHelp,
} from "@lucide/vue";
import DeviceContextBar from "./components/DeviceContextBar.vue";
import AppIcon from "./components/AppIcon.vue";
import BrandMark from "./components/BrandMark.vue";
import { useExecutionStore } from "./stores/execution";
import { useSerialStore } from "./stores/serial";
import { useWorkspace } from "./composables/useWorkspace";
import { useOnboarding } from "./composables/useOnboarding";
import { useUiPrefs, type ViewKey } from "./composables/useUiPrefs";
import { lightThemeOverrides, darkThemeOverrides } from "./styles/theme-overrides";
import WelcomeOverlay from "./components/onboarding/WelcomeOverlay.vue";

const executionStore = useExecutionStore();
const serialStore = useSerialStore();
const { theme, activeView: active, applyInitialTheme, sidebarCollapsed } = useUiPrefs();
applyInitialTheme();

void executionStore.init();
void serialStore.init();

const ExecutionDrawer = defineAsyncComponent(() => import("./components/ExecutionDrawer.vue"));
const { historyOpen: showExecutionHistory, connectionOpen } = useWorkspace();
const DeviceConnectionDialog = defineAsyncComponent(() => import("./components/DeviceConnectionDialog.vue"));
const connectionLoaded = ref(false);
watch(connectionOpen, (open) => { if (open) connectionLoaded.value = true; }, { immediate: true });
const showTools = ref(false);
const historyLoaded = ref(false);
watch(showExecutionHistory, (show) => { if (show) historyLoaded.value = true; });
function closeDrawer() {
  showExecutionHistory.value = false;
}

function onNavigate(view: string) {
  if ((["console","diag","ai","kb","stress","logs"] as const).includes(view as ViewKey)) {
    active.value = view as ViewKey;
  }
  closeDrawer();
  showTools.value = false;
}

const views = {
  console: defineAsyncComponent(() => import("./views/Console.vue")),
  diag: defineAsyncComponent(() => import("./views/Diagnosis.vue")),
  logs: defineAsyncComponent(() => import("./views/LogAnalysis.vue")),
  stress: defineAsyncComponent(() => import("./views/Stress.vue")),
  ai: defineAsyncComponent(() => import("./views/Ai.vue")),
  kb: defineAsyncComponent(() => import("./views/Knowledge.vue")),
};

const naiveTheme = computed<GlobalTheme | null>(() => (theme.value === "dark" ? darkTheme : null));
const themeOverrides = computed(() => (theme.value === "dark" ? darkThemeOverrides : lightThemeOverrides));

const menu = [
  { key: "console", label: "串口控制台", icon: Terminal },
  { key: "diag", label: "设备诊断", icon: FlaskConical },
  { key: "ai", label: "AI 助手", icon: Bot },
  { key: "kb", label: "知识中心", icon: BookOpen },
] as const;

const ActiveView = computed(() => views[active.value]);

// 首启引导：未连接任何会话 + 未完成过引导 → 显示
const { completed: onboarded, dismiss } = useOnboarding();
const helpRequested = ref(false);
function showHelp() { helpRequested.value = true; }
const showWelcome = computed(
  () => helpRequested.value || (!onboarded.value && serialStore.sessions.length === 0)
);

// 用户打开了会话（即便没点引导 CTA）→ 也视为完成引导，避免反复出现
watch(
  () => serialStore.sessions.length,
  (len) => {
    if (len > 0 && !onboarded.value) dismiss();
  }
);
</script>

<template>
  <n-config-provider
    :theme="naiveTheme"
    :theme-overrides="themeOverrides"
    :locale="zhCN"
    :date-locale="dateZhCN"
  >
    <n-message-provider>
      <n-dialog-provider>
        <div class="shell" :class="{ 'sidebar-collapsed': sidebarCollapsed }">
          <aside class="sidebar">
            <div class="brand">
              <BrandMark :size="34" />
              <div class="brand-text">
                <b>ModuLink</b>
                <small>模组智能调测</small>
              </div>
            </div>
            <div class="nav-section-label">工作区</div>
            <nav aria-label="主导航">
              <div
                v-for="m in menu"
                :key="m.key"
                class="nav-item"
                :class="{ active: active === m.key }"
                :title="m.label"
                :aria-current="active === m.key ? 'page' : undefined"
                role="link"
                tabindex="0"
                @click="onNavigate(m.key)"
                @keydown.enter="onNavigate(m.key)"
                @keydown.space.prevent="onNavigate(m.key)"
              >
                <AppIcon :icon="m.icon" class="nav-icon" />
                <span class="nav-label">{{ m.label }}</span>
              </div>
            </nav>
            <div class="advanced-tools" @keydown.esc="showTools = false">
              <button class="nav-item tools-trigger" :class="{ active: active === 'stress' || active === 'logs' }" title="高级工具" :aria-expanded="showTools" @click="showTools = !showTools">
                <AppIcon :icon="Wrench" class="nav-icon" /><span class="nav-label">高级工具</span>
              </button>
              <div v-if="showTools" class="tools-menu">
                <p>按需使用的辅助工具</p>
                <button @click="onNavigate('logs')">日志分析</button>
                <button @click="onNavigate('stress')">稳定性验证</button>
              </div>
            </div>
            <div
              class="nav-item execution-entry"
              title="执行记录"
              role="link"
              tabindex="0"
              @click="showExecutionHistory = true"
              @keydown.enter="showExecutionHistory = true"
              @keydown.space.prevent="showExecutionHistory = true"
            >
              <AppIcon :icon="History" class="nav-icon" />
              <span class="nav-label">执行记录</span>
              <span v-if="executionStore.runningCount" class="running-count">
                {{ executionStore.runningCount }}
              </span>
            </div>
            <button class="sidebar-toggle" :aria-label="sidebarCollapsed ? '展开导航' : '收起导航'" :title="sidebarCollapsed ? '展开导航' : '收起导航'" @click="sidebarCollapsed = !sidebarCollapsed">
              <AppIcon :icon="sidebarCollapsed ? PanelLeftOpen : PanelLeftClose" :size="17" /><span class="nav-label">收起导航</span>
            </button>
            <div class="sidebar-footer">
              <n-button
                size="tiny"
                quaternary
                circle
                title="查看新手引导"
                aria-label="查看新手引导"
                @click="showHelp"
              >
                <template #icon>
                  <AppIcon :icon="CircleHelp" :size="16" />
                </template>
              </n-button>
              <n-switch
                :value="theme === 'dark'"
                size="small"
                @update:value="(v: boolean) => (theme = v ? 'dark' : 'light')"
              >
                <template #checked>
                  <AppIcon :icon="Moon" :size="12" />
                </template>
                <template #unchecked>
                  <AppIcon :icon="Sun" :size="12" />
                </template>
              </n-switch>
              <small class="version">v0.1.0</small>
            </div>
          </aside>
          <main class="content">
            <DeviceContextBar />
            <div class="view-host">
              <KeepAlive>
                <component :is="ActiveView" />
              </KeepAlive>
            </div>
          </main>
          <ExecutionDrawer v-if="historyLoaded" v-model:show="showExecutionHistory" @navigate="onNavigate" />
        </div>
        <WelcomeOverlay v-if="showWelcome" @navigate="onNavigate" @close="helpRequested = false" />
        <DeviceConnectionDialog v-if="connectionLoaded" />
      </n-dialog-provider>
    </n-message-provider>
  </n-config-provider>
</template>

<style>
/* 终端始终深色——行业惯例（VS Code / Fiddler / Postman） */
.terminal-surface {
  background: var(--surface-terminal);
  color: var(--text-primary);
  font-family: Consolas, "Cascadia Mono", monospace;
}

.shell {
  display: flex;
  height: 100vh;
  background: var(--surface-base);
  color: var(--text-primary);
}

.sidebar {
  width: 176px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-subtle);
  padding: 14px 10px;
}

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 8px 18px;
}

.brand-text b {
  font-size: 15px;
  display: block;
  line-height: 1.15;
}

.brand-text small {
  color: var(--text-tertiary);
  font-size: 11px;
}

nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 9px 10px;
  border-radius: var(--radius-md);
  cursor: pointer;
  margin-bottom: 2px;
  user-select: none;
  color: var(--text-secondary);
  transition: background-color 120ms ease, color 120ms ease;
}

.nav-item:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}

.nav-item.active {
  background: var(--brand-primary-soft);
  color: var(--brand-primary);
  font-weight: 600;
}

/* active 状态左侧 2px 品牌色边条 */
.nav-item.active::before {
  content: "";
  position: absolute;
  left: 0;
  top: 8px;
  bottom: 8px;
  width: 2px;
  border-radius: 1px;
  background: var(--brand-primary);
}

.nav-icon {
  display: inline-flex;
  flex-shrink: 0;
  color: currentColor;
}

.nav-label {
  flex: 1;
}

.execution-entry {
  margin-top: auto;
}

.running-count {
  margin-left: auto;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  border-radius: 9px;
  display: grid;
  place-items: center;
  background: var(--brand-primary);
  color: var(--text-inverted);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.sidebar-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px;
  color: var(--text-tertiary);
}

.version {
  color: var(--text-tertiary);
}

.content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.view-host {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
}

/* Collapse is an explicit, persisted preference; labels remain visible by default. */
.sidebar-collapsed .sidebar {
    width: 64px;
    padding: 14px 8px;
  }
.sidebar-collapsed .brand {
    justify-content: center;
    padding: 4px 0 18px;
  }
.sidebar-collapsed .brand-text,
.sidebar-collapsed .nav-label,
.sidebar-collapsed .version {
    display: none;
  }
.sidebar-collapsed .nav-item {
    justify-content: center;
    padding: 10px 0;
  }
.sidebar-collapsed .execution-entry .running-count {
    position: absolute;
    top: 4px;
    right: 4px;
    margin-left: 0;
  }
.sidebar-collapsed .sidebar-footer {
    justify-content: center;
    padding: 8px 0;
  }

.nav-item { text-align: left; font: inherit; border: 0; }
.advanced-tools { position: relative; margin-top: 12px; }
.tools-trigger { width: 100%; background: transparent; }
.tools-menu { position: absolute; left: calc(100% + 12px); top: 0; z-index: 30; width: 190px; padding: 8px; background: var(--surface-raised); border: 1px solid var(--border-default); border-radius: 10px; box-shadow: var(--shadow-lg); }
.tools-menu p { padding: 6px 8px; color: var(--text-tertiary); font-size: 12px; }
.tools-menu button { display: block; padding: 10px; width: 100%; text-align: left; background: transparent; color: var(--text-primary); border: 0; border-radius: 6px; cursor: pointer; }
.tools-menu button:hover { background: var(--surface-hover); }
.sidebar-toggle { display: flex; align-items: center; gap: 12px; padding: 12px; border: 0; background: transparent; color: var(--text-secondary); cursor: pointer; }

.nav-section-label { padding: 4px 10px 10px; font-size: 11px; color: var(--text-tertiary); letter-spacing: 1px; }
.sidebar-collapsed .nav-section-label { display: none; }
.sidebar { padding-top: 20px; }
.nav-item { min-height: 42px; font-size: 13px; }
.execution-entry { margin-bottom: 8px; }
.sidebar-toggle { border-top: 1px solid var(--border-subtle); }
</style>