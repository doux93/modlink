import { computed, ref, watch } from "vue";
import { requestAiChat } from "../utils/aiTransport";
import type { useAiSettings } from "./useAiSettings";

export function useAiAvailability(config: ReturnType<typeof useAiSettings>) {
  const result = ref<"unverified" | "available" | "failed">("unverified");
  const error = ref("");
  const checking = ref(false);
  const configured = computed(() => config.loaded.value && !!config.settings.value.base_url.trim()
    && !!config.settings.value.model.trim() && config.settings.value.api_key_configured);
  let controller: AbortController | undefined;
  function cancel() { controller?.abort(new Error("测试已取消")); controller = undefined; checking.value = false; }
  watch(config.revision, () => { cancel(); result.value = "unverified"; error.value = ""; }, { flush: "sync" });
  const label = computed(() => !config.loaded.value ? (config.error.value ? "配置读取失败" : "正在读取配置")
    : !configured.value ? "AI 待配置" : checking.value ? "正在测试连接" :
    result.value === "available" ? "AI 可用" : result.value === "failed" ? "AI 连接失败" : "AI 尚未验证");
  function succeeded(revision: number) {
    if (revision === config.revision.value) { result.value = "available"; error.value = ""; }
  }
  function failed(revision: number, reason: unknown) {
    if (revision === config.revision.value) { result.value = "failed"; error.value = String(reason); }
  }
  async function verify() {
    if (!configured.value || checking.value || config.busy.value) return false;
    const revision = config.revision.value;
    const request = new AbortController(); controller = request; checking.value = true;
    const timer = setTimeout(() => request.abort(new Error("连接测试超时，请检查服务地址和网络")), 10_000);
    try {
      const response = await requestAiChat<{ choices?: unknown[] }>({ ...config.settings.value },
        { messages: [{ role: "user", content: "Reply OK." }], max_tokens: 1, temperature: 0 },
        { signal: request.signal, deadline: Date.now() + 10_000 });
      if (request.signal.aborted || controller !== request || revision !== config.revision.value) return false;
      if (!response.choices?.length) throw new Error("服务响应缺少有效结果，请检查模型配置");
      succeeded(revision); return true;
    } catch (reason) {
      if (controller === request && revision === config.revision.value) failed(revision, reason);
      return false;
    } finally {
      clearTimeout(timer);
      if (controller === request) { controller = undefined; checking.value = false; }
    }
  }
  return { configured, label, result, error, checking, succeeded, failed, verify, cancel };
}
