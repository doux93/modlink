import { computed, shallowRef } from "vue";

/** Reactive tool-call barrier; prevents another user turn until every card resolves. */
export function usePendingTools<T>() {
  const batch = shallowRef<{ context: T; unresolved: string[] } | null>(null);
  const awaitingConfirmation = computed(() => (batch.value?.unresolved.length ?? 0) > 0);
  function begin(context: T, ids: string[]) {
    batch.value = { context, unresolved: [...ids] };
  }
  function complete(id: string): T | undefined {
    const current = batch.value;
    if (!current || !current.unresolved.includes(id)) return undefined;
    const unresolved = current.unresolved.filter((pending) => pending !== id);
    if (unresolved.length) { batch.value = { ...current, unresolved }; return undefined; }
    batch.value = null;
    return current.context;
  }
  function clear() { batch.value = null; }
  return { awaitingConfirmation, begin, complete, clear };
}
