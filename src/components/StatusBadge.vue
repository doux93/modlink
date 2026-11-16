<script setup lang="ts">
import AppIcon from "./AppIcon.vue";
import type { Component } from "vue";

/**
 * StatusBadge — 统一状态徽章
 *
 * status: success | warning | danger | info | neutral | brand
 * variant: soft（柔和背景） | solid（实色块）
 * icon: 可选 lucide 图标
 */
withDefaults(
  defineProps<{
    status?: "success" | "warning" | "danger" | "info" | "neutral" | "brand";
    variant?: "soft" | "solid";
    icon?: Component;
    label: string;
  }>(),
  { status: "neutral", variant: "soft" }
);
</script>

<template>
  <span
    class="status-badge"
    :class="[`badge-${variant}`, `badge-${status}`]"
  >
    <AppIcon v-if="icon" :icon="icon" :size="11" />
    <span>{{ label }}</span>
  </span>
</template>

<style scoped>
.status-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  font-weight: 600;
  line-height: 1.6;
  padding: 2px 9px;
  border-radius: 999px;
  white-space: nowrap;
}

/* soft 变体 */
.badge-soft.badge-success { background: var(--status-success-soft); color: var(--status-success); }
.badge-soft.badge-warning { background: var(--status-warning-soft); color: var(--status-warning); }
.badge-soft.badge-danger  { background: var(--status-danger-soft);  color: var(--status-danger); }
.badge-soft.badge-info    { background: var(--status-info-soft);    color: var(--status-info); }
.badge-soft.badge-neutral { background: var(--surface-hover);       color: var(--text-secondary); }
.badge-soft.badge-brand   { background: var(--brand-primary-soft);  color: var(--brand-primary); }

/* solid 变体 */
.badge-solid.badge-success { background: var(--status-success); color: var(--text-inverted); }
.badge-solid.badge-warning { background: var(--status-warning); color: var(--text-inverted); }
.badge-solid.badge-danger  { background: var(--status-danger);  color: var(--text-inverted); }
.badge-solid.badge-info    { background: var(--status-info);    color: var(--text-inverted); }
.badge-solid.badge-neutral { background: var(--text-tertiary); color: var(--text-inverted); }
.badge-solid.badge-brand   { background: var(--brand-primary); color: var(--text-inverted); }
</style>