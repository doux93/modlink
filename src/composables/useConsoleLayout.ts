import { computed, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, type Ref } from "vue";
import { resolveConsoleLayout } from "../utils/consoleLayout";
import { readUiPreference, writeUiPreference } from "../utils/uiStorage";
import { useUiPrefs } from "./useUiPrefs";

function readWidth(key: string, fallback: number) {
  const value = Number(readUiPreference(key));
  return Number.isFinite(value) && value >= 260 && value <= 720 ? value : fallback;
}

/** Owns measurement/drag cleanup; the view only renders the resulting layout. */
export function useConsoleLayout(hasLog: Ref<boolean>, hasCommands: Ref<boolean>) {
  const { quickPanelPreference } = useUiPrefs();
  const bodyEl = ref<HTMLElement>();
  const width = ref(0);
  const quickW = ref(readWidth("ml.quickW", 280));
  const logW = ref(readWidth("ml.logW", 340));
  const layout = computed(() => resolveConsoleLayout({
    width: width.value, hasLog: hasLog.value, hasCommands: hasCommands.value,
    preference: quickPanelPreference.value, quickWidth: quickW.value, logWidth: logW.value,
  }));
  let observer: ResizeObserver | undefined;
  let finishDrag: (() => void) | undefined;
  function measure() {
    const measured = bodyEl.value?.getBoundingClientRect().width;
    if (measured) width.value = measured;
  }
  onMounted(() => {
    measure();
    observer = new ResizeObserver(measure);
    if (bodyEl.value) observer.observe(bodyEl.value);
  });
  onActivated(measure);
  onDeactivated(() => finishDrag?.());
  onBeforeUnmount(() => { observer?.disconnect(); finishDrag?.(); });

  function startDrag(which: "quick" | "log", event: MouseEvent) {
    event.preventDefault();
    finishDrag?.();
    const startX = event.clientX;
    const initial = which === "quick" ? layout.value.quickWidth : layout.value.logWidth;
    const move = (next: MouseEvent) => {
      const value = Math.min(720, Math.max(260, initial + startX - next.clientX));
      if (which === "quick") quickW.value = value;
      else logW.value = value;
    };
    const finish = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", finish);
      finishDrag = undefined;
      writeUiPreference("ml.quickW", String(quickW.value));
      writeUiPreference("ml.logW", String(logW.value));
    };
    finishDrag = finish;
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", finish);
  }
  return {
    bodyEl,
    quickVisible: computed(() => layout.value.quickVisible),
    quickFloating: computed(() => layout.value.quickFloating),
    quickWidth: computed(() => layout.value.quickWidth),
    logWidth: computed(() => layout.value.logWidth),
    toggleQuick: () => { quickPanelPreference.value = layout.value.quickVisible ? "closed" : "open"; },
    closeQuick: () => {
      quickPanelPreference.value = "closed";
      bodyEl.value?.parentElement?.querySelector<HTMLButtonElement>('[aria-controls="console-quick-panel"]')?.focus();
    },
    startDrag,
  };
}
