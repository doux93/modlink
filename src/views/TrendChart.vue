<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ data: { i: number; ms: number; ok: boolean }[] }>();

const W = 760;
const H = 180;
const PAD = 28;

const trend = computed(() => {
  const d = props.data;
  if (d.length < 2) return null;
  const maxMs = Math.max(...d.map((p) => p.ms), 1);
  const xStep = (W - PAD * 2) / Math.max(d.length - 1, 1);
  const yScale = (v: number) => H - PAD - (v / maxMs) * (H - PAD * 2);
  const pts = d.map((p, idx) => `${(PAD + idx * xStep).toFixed(1)},${yScale(p.ms).toFixed(1)}`);
  const fails = d
    .map((p, idx) => ({ idx, ok: p.ok }))
    .filter((p) => !p.ok)
    .map((p) => ({ x: PAD + p.idx * xStep, y: yScale(d[p.idx].ms) }));
  return { line: pts.join(" "), fails, maxMs };
});

const histogram = computed(() => {
  const d = props.data;
  if (!d.length) return null;
  const vals = d.map((p) => p.ms);
  const max = Math.max(...vals, 1);
  const buckets = 12;
  const counts = new Array(buckets).fill(0);
  for (const v of vals) {
    const idx = Math.min(buckets - 1, Math.floor((v / max) * buckets));
    counts[idx]++;
  }
  const maxCount = Math.max(...counts, 1);
  const bw = (W - PAD * 2) / buckets;
  const bars = counts.map((c, idx) => ({
    x: PAD + idx * bw,
    h: (c / maxCount) * (H - PAD * 2),
    c,
    lo: Math.round((idx / buckets) * max),
  }));
  return { bars, bw, max };
});
</script>

<template>
  <div class="charts">
    <div v-if="trend" class="chart-block">
      <div class="chart-title">延迟趋势（每轮耗时，红点=失败轮）</div>
      <svg :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" class="svg-chart">
        <line :x1="PAD" :y1="H - PAD" :x2="W - PAD" :y2="H - PAD" stroke="rgba(128,128,128,.3)" />
        <line :x1="PAD" :y1="PAD" :x2="PAD" :y2="H - PAD" stroke="rgba(128,128,128,.3)" />
        <text :x="4" :y="PAD + 4" fill="rgba(128,128,128,.6)" font-size="10">{{ trend.maxMs }}ms</text>
        <polyline :points="trend.line" fill="none" stroke="#4c8dff" stroke-width="1.2" />
        <circle v-for="(f, idx) in trend.fails" :key="idx" :cx="f.x" :cy="f.y" r="2.2" fill="#ff7b72" />
      </svg>
    </div>
    <div v-if="histogram" class="chart-block">
      <div class="chart-title">延迟分布直方图</div>
      <svg :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" class="svg-chart">
        <rect v-for="(b, idx) in histogram.bars" :key="idx"
          :x="b.x + 1" :y="H - PAD - b.h" :width="Math.max(histogram.bw - 2, 1)" :height="b.h"
          fill="rgba(76,141,255,.5)" />
        <line :x1="PAD" :y1="H - PAD" :x2="W - PAD" :y2="H - PAD" stroke="rgba(128,128,128,.3)" />
        <text :x="PAD" :y="H - 8" fill="rgba(128,128,128,.5)" font-size="9">0</text>
        <text :x="W - PAD - 16" :y="H - 8" fill="rgba(128,128,128,.5)" font-size="9">{{ histogram.max }}ms</text>
      </svg>
    </div>
    <div v-if="!data.length" class="empty">运行后显示趋势图</div>
  </div>
</template>

<style scoped>
.charts { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; max-width: 1180px; margin-top: 18px; }
.chart-block { min-width: 0; border: 1px solid rgba(128,128,128,.16); border-radius: 8px; padding: 12px; background: rgba(128,128,128,.025); }
.chart-title { font-size: 12px; opacity: .6; margin-bottom: 6px; }
.svg-chart { width: 100%; height: 150px; display: block; }
.empty { padding: 20px 0; text-align: center; opacity: .4; font-size: 12px; }
@media (max-width: 860px) {
  .charts { grid-template-columns: minmax(0, 1fr); }
}
</style>
