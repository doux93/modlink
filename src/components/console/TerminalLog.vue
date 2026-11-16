<script setup lang="ts">
import { computed, onActivated, onMounted, ref, watch } from "vue";
import { NTooltip } from "naive-ui";
import type { ErrorCodeEntry, Frame, UrcPattern } from "../../types";

const props = withDefaults(defineProps<{
  frames: readonly Frame[];
  sourceKey: string;
  hiddenCount?: number;
  fontSize?: number;
  showTime?: boolean;
  showHex?: boolean;
  autoScroll?: boolean;
  plain?: boolean;
  emptyMessage: string;
  errorCodes?: Record<string, ErrorCodeEntry>;
  urcPatterns?: UrcPattern[];
}>(), {
  hiddenCount: 0,
  fontSize: 13,
  showTime: true,
  showHex: false,
  autoScroll: true,
  plain: false,
  errorCodes: () => ({}),
  urcPatterns: () => [],
});

const viewport = ref<HTMLElement>();
const encoder = new TextEncoder();
const patterns = computed(() => props.urcPatterns.flatMap((pattern) => {
  try { return [{ ...pattern, re: new RegExp(pattern.pattern) }]; }
  catch { return []; }
}));

function errorAnnotation(frame: Frame) {
  if (frame.dir !== "rx") return;
  const match = /^\+(CME|CMS) ERROR:\s*(\d+)/.exec(frame.text.trim());
  const entry = match && props.errorCodes[`${match[1].toLowerCase()}:${match[2]}`];
  if (!entry) return;
  return {
    note: entry.meaning,
    tip: [
      entry.causes.length ? `可能原因:\n· ${entry.causes.join("\n· ")}` : "",
      entry.suggestions.length ? `建议:\n· ${entry.suggestions.join("\n· ")}` : "",
    ].filter(Boolean).join("\n\n"),
  };
}

function urcAnnotation(frame: Frame) {
  if (frame.dir !== "rx" || !["urc", "response"].includes(frame.kind)) return;
  for (const pattern of patterns.value) {
    const match = pattern.re.exec(frame.text.trim());
    if (!match) continue;
    let severity = pattern.severity ?? "info";
    const field = pattern.severity_field && match.groups?.[pattern.severity_field];
    if (field !== undefined && field !== "") {
      severity = pattern.severity_map[field] ?? pattern.severity_default ?? severity;
    }
    return { label: pattern.label, severity };
  }
}

function lineClass(frame: Frame) {
  if (frame.dir === "tx") return "ln-tx";
  if (frame.kind === "final") return frame.text.includes("ERROR") ? "ln-error" : "ln-ok";
  return frame.kind === "urc" ? "ln-urc" : "ln-rx";
}

// Compute annotations once per displayed row, rather than repeatedly in the template.
const rows = computed(() => props.frames.map((frame) => {
  const date = new Date(frame.ts);
  return {
    frame,
    time: `${date.toLocaleTimeString("zh-CN", { hour12: false })}.${String(date.getMilliseconds()).padStart(3, "0")}`,
    text: props.plain ? frame.text : props.showHex && !frame.hex
      ? Array.from(encoder.encode(frame.text), (byte) => byte.toString(16).padStart(2, "0").toUpperCase()).join(" ")
      : frame.hex ?? frame.text,
    className: props.plain ? "" : lineClass(frame),
    error: props.plain ? undefined : errorAnnotation(frame),
    urc: props.plain ? undefined : urcAnnotation(frame),
  };
}));

function scrollToLatest() {
  if (props.autoScroll && viewport.value) {
    viewport.value.scrollTo({ top: viewport.value.scrollHeight });
  }
}
watch(
  [() => props.sourceKey, () => props.frames[props.frames.length - 1]?.seq, () => props.autoScroll],
  scrollToLatest,
  { flush: "post" },
);
onMounted(scrollToLatest);
onActivated(scrollToLatest);
</script>

<template>
  <div ref="viewport" class="terminal" :class="{ 'log-output': plain }" :style="{ fontSize: fontSize + 'px' }" role="log" aria-live="off">
    <div v-if="hiddenCount > 0" class="cap-hint">
      ↑ 还有 {{ hiddenCount.toLocaleString() }} 行未显示（数据已保留，可用过滤或保存日志查看全部）
    </div>
    <div v-for="row in rows" :key="row.frame.seq" class="line" :class="row.className">
      <span v-if="showTime" class="ts">{{ row.time }}</span>
      <span v-if="!plain" class="dir">{{ row.frame.dir === "tx" ? "»" : "«" }}</span>
      <span class="txt">{{ row.text }}</span>
      <span v-if="row.urc" class="chip" :class="`sev-${row.urc.severity}`">{{ row.urc.label }}</span>
      <n-tooltip v-if="row.error" trigger="hover" placement="top-start" style="max-width: 380px">
        <template #trigger><span class="anno">ⓘ {{ row.error.note }}</span></template>
        <div style="white-space: pre-wrap">{{ row.error.tip }}</div>
      </n-tooltip>
    </div>
    <div v-if="!rows.length" class="placeholder">{{ emptyMessage }}</div>
  </div>
</template>

<style scoped>
.terminal {
  flex: 1; min-height: 0; overflow-y: auto; border-radius: 8px; padding: 10px 12px;
  background: var(--surface-terminal); color: var(--term-rx);
  font-family: Consolas, "Cascadia Mono", monospace; line-height: 1.55;
}
.log-output { background: var(--surface-terminal-alt); color: var(--term-rx); line-height: 1.5; }
.line { display: flex; flex-wrap: wrap; gap: 0 8px; white-space: pre-wrap; word-break: break-all; align-items: baseline; }
.txt { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.ts { color: var(--term-muted); flex-shrink: 0; }
.dir { flex-shrink: 0; opacity: 0.7; }
.ln-tx .txt { color: var(--term-tx); }
.ln-rx .txt { color: var(--term-rx); }
.ln-ok .txt { color: var(--term-ok); font-weight: 600; }
.ln-error .txt { color: var(--term-error); font-weight: 600; }
.ln-urc .txt { color: var(--term-urc); }
.placeholder { color: var(--term-muted); padding: 20px; text-align: center; }
.chip { flex-shrink: 0; font-size: 11px; padding: 0 7px; border-radius: 8px; line-height: 17px; }
.sev-info { background: rgba(45, 189, 120, 0.18); color: var(--term-ok); }
.sev-warning { background: rgba(224, 167, 59, 0.18); color: var(--term-urc); }
.sev-error { background: rgba(217, 83, 79, 0.18); color: var(--term-error); }
.anno { color: var(--term-anno); font-size: 12px; cursor: help; flex-shrink: 1; max-width: 100%; }
.anno:hover { color: var(--term-echo); }
.cap-hint { color: var(--status-warning); font-size: 12px; padding-bottom: 6px; }
</style>
