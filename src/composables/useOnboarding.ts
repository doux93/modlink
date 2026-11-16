import { ref, computed } from "vue";

const STORAGE_KEY = "modulink-onboarding-completed";

const completed = ref(
  typeof localStorage !== "undefined" && localStorage.getItem(STORAGE_KEY) === "1"
);

function dismiss() {
  completed.value = true;
  try {
    localStorage.setItem(STORAGE_KEY, "1");
  } catch (_) {
    /* ignore */
  }
}

function reset() {
  completed.value = false;
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch (_) {
    /* ignore */
  }
}

export function useOnboarding() {
  return {
    completed: computed(() => completed.value),
    dismiss,
    reset,
  };
}