import { ref } from "vue";

/** Prevent duplicate submissions; retain a recoverable error near the action. */
export function useAsyncAction(label: string, notifyError?: (message: string) => void) {
  const busy = ref(false);
  const error = ref("");
  async function run(action: () => Promise<void>): Promise<boolean> {
    if (busy.value) return false;
    busy.value = true;
    error.value = "";
    try { await action(); return true; }
    catch (cause) {
      error.value = `${label}：${cause instanceof Error ? cause.message : String(cause)}`;
      notifyError?.(error.value);
      return false;
    } finally { busy.value = false; }
  }
  return { busy, error, run };
}
