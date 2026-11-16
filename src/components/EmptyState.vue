<script setup lang="ts">
import type { Component } from "vue";
import AppIcon from "./AppIcon.vue";

/**
 * EmptyState — 空状态/首启引导卡片
 *
 * 用法：
 *   <EmptyState :icon="BookOpen" title="知识包未安装"
 *               description="导入 .mlkb 包后可启用手册全文。"
 *               action-label="选择知识包"
 *               @action="onChoose" />
 */
defineProps<{
  icon: Component;
  title: string;
  description?: string;
  actionLabel?: string;
  actionIcon?: Component;
}>();

defineEmits<{
  action: [];
}>();
</script>

<template>
  <div class="empty-state">
    <div class="empty-icon">
      <AppIcon :icon="icon" :size="32" />
    </div>
    <h4 class="empty-title">{{ title }}</h4>
    <p v-if="description" class="empty-desc">{{ description }}</p>
    <button
      v-if="actionLabel"
      class="empty-action"
      type="button"
      @click="$emit('action')"
    >
      <AppIcon v-if="actionIcon" :icon="actionIcon" :size="14" />
      <span>{{ actionLabel }}</span>
    </button>
    <slot />
  </div>
</template>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: var(--space-7) var(--space-5);
  gap: var(--space-2);
  color: var(--text-secondary);
}

.empty-icon {
  width: 56px;
  height: 56px;
  border-radius: var(--radius-lg);
  background: var(--brand-primary-soft);
  color: var(--brand-primary);
  display: grid;
  place-items: center;
  margin-bottom: var(--space-2);
}

.empty-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0;
}

.empty-desc {
  font-size: 13px;
  color: var(--text-secondary);
  max-width: 360px;
  line-height: 1.55;
  margin: 0;
}

.empty-action {
  margin-top: var(--space-3);
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: 8px 16px;
  border: 0;
  border-radius: var(--radius-md);
  background: var(--brand-primary);
  color: var(--text-inverted);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: background-color 120ms ease;
}

.empty-action:hover {
  background: var(--brand-primary-hover);
}
.empty-action:active {
  background: var(--brand-primary-pressed);
}
</style>