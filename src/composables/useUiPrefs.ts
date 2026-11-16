import { ref, watch } from "vue";
import { readUiPreference as safeGet, writeUiPreference as safeSet } from "../utils/uiStorage";
import type { QuickPanelPreference } from "../utils/consoleLayout";

/**
 * useUiPrefs — 模块级共享 UI 偏好，保留已有 localStorage key。
 *
 * 持久化范围：
 *  - ml.theme   ("dark" | "light")
 *  - ml.lastView ("console" | "diag" | "ai" | "kb" | "stress" | "logs")
 *  - ml.fontSize (number)
 *  - ml.sidebarCollapsed (0 | 1)
 *  - ml.showQuickCommands (0 | 1；未设置时按内容和窗口自动布局)
 *
 * 不在持久化范围：
 *  - onboarding 完成状态（useOnboarding.ts 单 key，更独立）
 *
 * 所有失败（隐私模式 / 禁用 storage / SSR 等）一律静默回退到默认值。
 */
export type ViewKey = "console" | "diag" | "ai" | "kb" | "stress" | "logs";
const VALID_VIEWS: readonly ViewKey[] = ["console", "diag", "ai", "kb", "stress", "logs"];

const isValidView = (v: string | null): v is ViewKey =>
  !!v && (VALID_VIEWS as readonly string[]).includes(v);

// --- 模块级 singleton refs，避免每个调用方一份独立状态 ---
const theme = ref<"dark" | "light">(safeGet("ml.theme") === "light" ? "light" : "dark");
const activeView = ref<ViewKey>(
  isValidView(safeGet("ml.lastView")) ? (safeGet("ml.lastView") as ViewKey) : "console"
);

function readFontSize(): number {
  const raw = safeGet("ml.fontSize");
  const n = raw == null ? NaN : Number(raw);
  return Number.isFinite(n) && n >= 10 && n <= 24 ? n : 13;
}
const fontSize = ref<number>(readFontSize());
// Preserve explicit choices made by earlier versions; no stored choice means auto.
const storedQuickPanel = safeGet("ml.showQuickCommands");
const quickPanelPreference = ref<QuickPanelPreference>(storedQuickPanel === "1" ? "open" : storedQuickPanel === "0" ? "closed" : "auto");
watch(quickPanelPreference, (value) => safeSet("ml.showQuickCommands", value === "auto" ? null : value === "open" ? "1" : "0"));
const sidebarCollapsed = ref(safeGet("ml.sidebarCollapsed") === "1");
watch(sidebarCollapsed, (value) => safeSet("ml.sidebarCollapsed", value ? "1" : "0"));

watch(theme, (v) => {
  safeSet("ml.theme", v);
  try {
    document.documentElement.setAttribute("data-theme", v);
  } catch {
    /* SSR / 极端环境忽略 */
  }
});
watch(activeView, (v) => safeSet("ml.lastView", v));
watch(fontSize, (v) => safeSet("ml.fontSize", String(v)));

/** 启动时把主题应用到 <html>（避免与 index.html 内联脚本双写） */
function applyInitialTheme(): void {
  try {
    document.documentElement.setAttribute("data-theme", theme.value);
  } catch {
    /* ignore */
  }
}

export function useUiPrefs() {
  return {
    theme,
    activeView,
    fontSize,
    sidebarCollapsed,
    quickPanelPreference,
    applyInitialTheme,
  };
}
