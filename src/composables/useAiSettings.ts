import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useAsyncAction } from "./useAsyncAction";

export interface AiSettings {
  base_url: string;
  model: string;
  recent_log_lines: number;
  api_key_configured: boolean;
}

export function useAiSettings(notifySuccess: (message: string) => void) {
  const settings = ref<AiSettings>({ base_url: "", model: "auto", recent_log_lines: 80, api_key_configured: false });
  const draft = ref({ ...settings.value, api_key: "" });
  const show = ref(false);
  const loaded = ref(false);
  const revision = ref(0);
  const revealApiKey = ref(false);
  const loading = useAsyncAction("读取配置失败");
  const saving = useAsyncAction("保存配置失败");
  const error = computed(() => saving.error.value || loading.error.value);
  const busy = computed(() => loading.busy.value || saving.busy.value);

  function resetDraft() { draft.value = { ...settings.value, api_key: "" }; revealApiKey.value = false; }
  function load() {
    return loading.run(async () => {
      loaded.value = false;
      const result = await invoke<{ ai?: Partial<AiSettings> }>("get_settings");
      const ai = result?.ai;
      settings.value = {
        base_url: ai?.base_url ?? "", model: ai?.model ?? "auto",
        recent_log_lines: ai?.recent_log_lines ?? 80, api_key_configured: ai?.api_key_configured ?? false,
      };
      loaded.value = true;
      revision.value += 1;
      resetDraft();
    });
  }
  function open() {
    resetDraft(); saving.error.value = ""; show.value = true;
    if (!loaded.value) void load();
  }
  function setOpen(value: boolean) {
    if (saving.busy.value) return;
    if (value) open();
    else { show.value = false; resetDraft(); saving.error.value = ""; }
  }
  async function save() {
    if (!loaded.value || loading.busy.value) return false;
    return saving.run(async () => {
      const candidate = { ...draft.value };
      if (!Number.isInteger(candidate.recent_log_lines) || candidate.recent_log_lines < 20 || candidate.recent_log_lines > 300)
        throw new Error("日志行数应为 20～300 的整数");
      const ai: Record<string, unknown> = { base_url: candidate.base_url.trim(), model: candidate.model.trim(), recent_log_lines: candidate.recent_log_lines };
      if (candidate.api_key.trim()) ai.api_key = candidate.api_key.trim();
      await invoke("save_settings", { settings: { ai } });
      settings.value = { base_url: String(ai.base_url), model: String(ai.model), recent_log_lines: candidate.recent_log_lines,
        api_key_configured: !!candidate.api_key.trim() || settings.value.api_key_configured };
      revision.value += 1;
      show.value = false; resetDraft();
      notifySuccess("AI 配置已保存");
    });
  }
  function clearApiKey() {
    if (!loaded.value || loading.busy.value) return Promise.resolve(false);
    return saving.run(async () => {
      await invoke("save_settings", { settings: { ai: { api_key: null } } });
      settings.value.api_key_configured = false;
      revision.value += 1;
      draft.value.api_key_configured = false; draft.value.api_key = ""; revealApiKey.value = false;
      notifySuccess("API key 已从系统凭据中删除");
    });
  }
  return { settings, draft, show, loaded, revision, revealApiKey, busy, error, mutating: saving.busy, load, open, setOpen, save, clearApiKey };
}
