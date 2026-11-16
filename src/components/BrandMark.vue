<script setup lang="ts">
/**
 * BrandMark — ModuLink Studio logo
 * M 字母 + 右上角青绿天线 motif
 */
defineProps<{
  size?: number;
}>();
</script>

<template>
  <span
    class="brand-mark"
    :style="{ width: (size ?? 34) + 'px', height: (size ?? 34) + 'px' }"
    aria-hidden="true"
  >
    <svg
      :width="(size ?? 34) - 4"
      :height="(size ?? 34) - 4"
      viewBox="0 0 30 30"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <!-- 青绿天线 motif：右上角信号弧线 -->
      <path
        d="M 21 5.5 Q 25 5.5 25 9.5"
        stroke="#18b3a2"
        stroke-width="1.5"
        stroke-linecap="round"
        fill="none"
        opacity="0.95"
      />
      <circle cx="25" cy="9.5" r="1.2" fill="#18b3a2" />

      <!-- M 字母主体：白色粗体，两脚稍宽 -->
      <path
        d="M 5 24 L 5 8 L 11 18 L 17 8 L 17 24"
        stroke="#ffffff"
        stroke-width="2.6"
        stroke-linecap="round"
        stroke-linejoin="round"
        fill="none"
      />
    </svg>
  </span>
</template>

<style scoped>
.brand-mark {
  display: inline-grid;
  place-items: center;
  border-radius: 9px;
  background: linear-gradient(135deg, #2f6fed 0%, #18b3a2 100%);
  flex-shrink: 0;
}
</style>