<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from "vue";
import {
  darkTheme,
  zhCN,
  dateZhCN,
  type GlobalTheme,
} from "naive-ui";
import {
  Terminal,
  FlaskConical,
  Bot,
  BookOpen,
  Settings,
  BarChart3,
  History,
  Sun,
  Moon,
  CircleHelp,
} from "@lucide/vue";
import ExecutionDrawer from "./components/ExecutionDrawer.vue";
import DeviceContextBar from "./components/DeviceContextBar.vue";
import AppIcon from "./components/AppIcon.vue";
import BrandMark from "./components/BrandMark.vue";
import { useExecutionStore } from "./stores/execution";
import { useSerialStore } from "./stores/serial";
import { useOnboarding } from "./composables/useOnboarding";
import { useUiPrefs, type ViewKey } from "./composables/useUiPrefs";
import { lightThemeOverrides, darkThemeOverrides } from "./styles/theme-overrides";
import WelcomeOverlay from "./components/onboarding/WelcomeOverlay.vue";

const executionStore = useExecutionStore();
const serialStore = useSerialStore();
const { theme, activeView: active, applyInitialTheme } = useUiPrefs();
applyInitialTheme();

void executionStore.init();
void serialStore.init();

const showExecutionHistory = ref(false);
function closeDrawer() {
  showExecutionHistory.value = false;
}

function onNavigate(view: string) {
  if ((["console","diag","ai","kb","stress","logs"] as const).includes(view as ViewKey)) {
    active.value = view as ViewKey;
  }
  closeDrawer();
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
  { key: "diag", label: "验证中心", icon: FlaskConical },
  { key: "ai", label: "AI 助手", icon: Bot },
  { key: "kb", label: "知识中心", icon: BookOpen },
  { key: "stress", label: "稳定性验证", icon: Settings },
  { key: "logs", label: "日志分析", icon: BarChart3 },
] as const;

const ActiveView = computed(() => views[active.value]);

// 首启引导：未连接任何会话 + 未完成过引导 → 显示
const { completed: onboarded, dismiss, reset: showHelp } = useOnboarding();
const showWelcome = computed(
  () => !onboarded.value && serialStore.sessions.length === 0
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
        <div class="shell">
          <aside class="sidebar">
            <div class="brand">
              <BrandMark :size="34" />
              <div class="brand-text">
                <b>ModuLink</b>
                <small>模组智能调测</small>
              </div>
            </div>
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
            <div class="sidebar-footer">
              <n-button
                size="tiny"
                quaternary
                circle
                title="查看新手引导"
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
          <ExecutionDrawer v-model:show="showExecutionHistory" @navigate="onNavigate" />
        </div>
        <WelcomeOverlay v-if="showWelcome" />
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
  width: 200px;
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

@media (max-width: 1120px) {
  .sidebar {
    width: 64px;
    padding: 14px 8px;
  }
  .brand {
    justify-content: center;
    padding: 4px 0 18px;
  }
  .brand-text,
  .nav-label,
  .version {
    display: none;
  }
  .nav-item {
    justify-content: center;
    padding: 10px 0;
  }
  .execution-entry .running-count {
    position: absolute;
    top: 4px;
    right: 4px;
    margin-left: 0;
  }
  .sidebar-footer {
    justify-content: center;
    padding: 8px 0;
  }
}
</style>