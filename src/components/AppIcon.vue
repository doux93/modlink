<script setup lang="ts">
import { NIcon } from "naive-ui";
import type { Component } from "vue";

/**
 * AppIcon — 统一图标入口。
 * 所有图标都走这个组件，禁止视图直接用 <n-icon>，否则大小/粗细会失控。
 * 默认 size=18, strokeWidth=1.75，与 lucide 默认一致。
 */
defineProps<{
  icon: Component;
  size?: number;
  color?: string;
  strokeWidth?: number;
}>();
</script>

<template>
  <n-icon
    :size="size ?? 18"
    :color="color"
    :component="icon"
    :stroke-width="strokeWidth ?? 1.75"
    aria-hidden="true"
  />
</template>