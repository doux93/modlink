import { invoke } from "@tauri-apps/api/core";
import { AiRequestTimeoutError } from "../composables/useAiRequestGate";

interface AiConfig { base_url: string; model: string }
interface RequestLifetime { signal: AbortSignal; deadline: number }

/** Register before sending, so cancellation cannot overtake backend registration. */
export async function requestAiChat<T>(cfg: AiConfig, body: unknown, lifetime: RequestLifetime): Promise<T> {
  const { signal, deadline } = lifetime;
  if (signal.aborted) throw signal.reason;
  if (deadline <= Date.now()) throw new AiRequestTimeoutError();
  const requestId = crypto.randomUUID();
  await invoke("prepare_ai_request", { requestId });
  let cleanup: Promise<unknown> | undefined;
  const release = () => cleanup ??= invoke("cancel_ai_request", { requestId }).catch((error) => {
    console.warn("AI 请求清理失败，将由后端超时回收", error);
  });
  let onAbort = () => {};
  try {
    // Cancellation while registration was in flight must prevent the HTTP call.
    if (signal.aborted) throw signal.reason;
    const timeoutMs = Math.min(30_000, deadline - Date.now());
    if (timeoutMs <= 0) throw new AiRequestTimeoutError();
    const aborted = new Promise<never>((_, reject) => {
      onAbort = () => {
        void release();
        reject(signal.reason);
      };
      signal.addEventListener("abort", onAbort, { once: true });
    });
    return await Promise.race([
      invoke<T>("ai_chat", { requestId, timeoutMs, cfg, body }),
      aborted,
    ]);
  } finally {
    signal.removeEventListener("abort", onAbort);
    // Also releases a prepared request if validation or IPC fails before claim.
    void release();
  }
}
