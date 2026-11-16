import { ref } from "vue";

// Shared entry points: every page opens the same connection panel/history drawer.
const connectionOpen = ref(false);
const historyOpen = ref(false);
export function useWorkspace() {
  return {
    connectionOpen, historyOpen,
    connectDevice: () => { connectionOpen.value = true; },
    openHistory: () => { historyOpen.value = true; },
  };
}
