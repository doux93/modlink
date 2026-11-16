<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { Cable, Usb, Activity, ArrowRight, X } from "@lucide/vue";
import AppIcon from "../AppIcon.vue";
import { useOnboarding } from "../../composables/useOnboarding";

const { dismiss: complete } = useOnboarding();
const emit = defineEmits<{ navigate: [view: string]; close: [] }>();
function dismiss() { complete(); emit("close"); }
let focusTimer: ReturnType<typeof setTimeout> | undefined;
const cardRef = ref<HTMLDivElement | null>(null);

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") dismiss();
}

onMounted(() => {
  document.addEventListener("keydown", onKeydown);
  // 进入动画结束后聚焦主按钮，方便键盘用户直接 Enter
  focusTimer = setTimeout(() => cardRef.value?.querySelector<HTMLButtonElement>(".welcome-cta")?.focus(), 100);
});
onUnmounted(() => {
  document.removeEventListener("keydown", onKeydown);
  clearTimeout(focusTimer);
});

const steps = [
  {
    icon: Usb,
    title: "插上模组",
    desc: "用 USB 转串口线把模组（Cat1 / Cat4 / RedCap）连接到本机，并确认驱动程序已安装。",
  },
  {
    icon: Cable,
    title: "选择串口",
    desc: "点击「连接设备」，选择设备的 AT 串口和波特率；多个设备可在顶部切换。",
  },
  {
    icon: Activity,
    title: "一键体检",
    desc: "进入「设备诊断」运行基础检查，查看异常与建议，处理后复测并导出报告。",
  },
];

function gotoConsole() {
  emit("navigate", "console");
  dismiss();
}
</script>

<template>
  <div class="welcome-mask" @click.self="dismiss">
    <div class="welcome-card" role="dialog" aria-modal="true" aria-label="首次使用引导" ref="cardRef">
      <button class="welcome-close" @click="dismiss" aria-label="关闭引导">
        <AppIcon :icon="X" :size="16" />
      </button>

      <header class="welcome-head">
        <div class="welcome-eyebrow">首次使用</div>
        <h2>开始第一次设备调测</h2>
        <p>ModuLink Studio 是蜂窝模组的串口调试工作台。从连接设备开始，逐步定位问题。</p>
      </header>

      <ol class="welcome-steps">
        <li v-for="(s, i) in steps" :key="i" class="step">
          <div class="step-index">{{ i + 1 }}</div>
          <div class="step-icon">
            <AppIcon :icon="s.icon" :size="22" />
          </div>
          <div class="step-body">
            <b>{{ s.title }}</b>
            <span>{{ s.desc }}</span>
          </div>
        </li>
      </ol>

      <footer class="welcome-foot">
        <button class="welcome-cta" @click="gotoConsole">
          打开串口控制台
          <AppIcon :icon="ArrowRight" :size="16" />
        </button>
        <small>可在「执行记录」中查看本次诊断</small>
      </footer>
    </div>
  </div>
</template>

<style scoped>
.welcome-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(2px);
  display: grid;
  place-items: center;
  z-index: var(--z-modal);
  animation: fade-in 180ms ease-out;
}

.welcome-card {
  position: relative;
  width: min(540px, 92vw);
  background: var(--surface-raised);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  padding: var(--space-7) var(--space-6) var(--space-6);
  color: var(--text-primary);
}

.welcome-close {
  position: absolute;
  top: var(--space-3);
  right: var(--space-3);
  background: transparent;
  border: 0;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm);
  display: grid;
  place-items: center;
  cursor: pointer;
  color: var(--text-tertiary);
  transition: background 120ms ease;
}
.welcome-close:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}

.welcome-head {
  text-align: center;
  margin-bottom: var(--space-6);
}

.welcome-eyebrow {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--brand-primary);
  margin-bottom: var(--space-2);
}

.welcome-head h2 {
  font-size: 22px;
  margin: 0 0 var(--space-2);
  font-weight: 600;
  color: var(--text-primary);
}

.welcome-head p {
  font-size: 13px;
  color: var(--text-secondary);
  margin: 0;
}

.welcome-steps {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.step {
  display: grid;
  grid-template-columns: 28px 44px 1fr;
  gap: var(--space-3);
  align-items: center;
  padding: var(--space-3);
  background: var(--surface-base);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-subtle);
}

.step-index {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  font-size: 12px;
  font-weight: 600;
  color: var(--brand-primary);
  background: var(--brand-primary-soft);
}

.step-icon {
  width: 44px;
  height: 44px;
  border-radius: var(--radius-md);
  display: grid;
  place-items: center;
  background: var(--surface-raised);
  color: var(--brand-primary);
  border: 1px solid var(--border-subtle);
}

.step-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.step-body b {
  font-size: 14px;
  color: var(--text-primary);
}

.step-body span {
  font-size: 12.5px;
  color: var(--text-secondary);
  line-height: 1.5;
}

.welcome-foot {
  margin-top: var(--space-6);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
}

.welcome-cta {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: 10px 22px;
  border: 0;
  border-radius: var(--radius-md);
  background: var(--brand-primary);
  color: var(--text-inverted);
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: background-color 120ms ease;
}
.welcome-cta:hover {
  background: var(--brand-primary-hover);
}
.welcome-cta:active {
  background: var(--brand-primary-pressed);
}

.welcome-foot small {
  font-size: 11px;
  color: var(--text-tertiary);
}

@keyframes fade-in {
  from {
    opacity: 0;
    transform: translateY(4px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
</style>