import { computed, ref } from "vue";
import type { DeviceContext } from "../utils/deviceContext";

export type AiRequestStage = "idle" | "preparing" | "searching" | "waiting" | "resuming";
export interface AiRequestContext {
  target?: DeviceContext;
  contextKey: string;
  id: number;
  deadline: number;
  historyCheckpoint: number;
  uiCheckpoint: number;
  conversationVersion: number;
  cancelled: boolean;
  cancelledPromise: Promise<void>;
  resolveCancelled: () => void;
  abortController: AbortController;
}

export class AiRequestCancelledError extends Error {
  constructor() {
    super("AI 请求已取消");
    this.name = "AiRequestCancelledError";
  }
}

export class AiRequestTimeoutError extends Error {
  constructor() {
    super("AI 请求等待超过 30 秒");
    this.name = "AiRequestTimeoutError";
  }
}

export function isRequestCancelled(error: unknown): boolean {
  return error instanceof AiRequestCancelledError;
}

export function useAiRequestGate(
  getContext: () => DeviceContext | undefined,
  getContextKey: () => string,
  getVersion: () => number,
  getUiLength: () => number,
  onCancel: (request: AiRequestContext) => void,
) {
  const AI_REQUEST_TIMEOUT_MS = 30_000;
  const requestStage = ref<AiRequestStage>("idle");
  const activeRequestId = ref<number | null>(null);
  let activeRequest: AiRequestContext | null = null;
  let nextRequestId = 0;

  const requestInFlight = computed(() => activeRequestId.value !== null);
  const requestStageLabel = computed(() => {
    switch (requestStage.value) {
      case "preparing": return "正在准备设备与日志上下文…";
      case "searching": return "正在检索当前型号手册…";
      case "waiting": return "正在等待 AI 响应…";
      case "resuming": return "正在根据执行结果继续分析…";
      default: return "";
    }
  });

  function beginRequest(stage: Exclude<AiRequestStage, "idle">, historyCheckpoint: number): AiRequestContext {
    cancelActiveRequest();
    let resolveCancelled = () => {};
    const cancelledPromise = new Promise<void>((resolve) => {
      resolveCancelled = resolve;
    });
    const target = getContext();
    const request: AiRequestContext = {
      id: ++nextRequestId,
      deadline: Date.now() + AI_REQUEST_TIMEOUT_MS,
      historyCheckpoint,
      uiCheckpoint: getUiLength(),
      conversationVersion: getVersion(),
      target: target ? { ...target } : undefined,
      contextKey: getContextKey(),
      cancelled: false,
      cancelledPromise,
      resolveCancelled,
      abortController: new AbortController(),
    };
    activeRequest = request;
    activeRequestId.value = request.id;
    requestStage.value = stage;
    return request;
  }

  function requestIsCurrent(request: AiRequestContext): boolean {
    return activeRequest === request &&
      !request.cancelled &&
      request.conversationVersion === getVersion() && request.contextKey === getContextKey();
  }

  function assertRequestCurrent(request: AiRequestContext) {
    if (request.abortController.signal.aborted) throw request.abortController.signal.reason;
    if (!requestIsCurrent(request)) throw new AiRequestCancelledError();
  }

  function setRequestStage(request: AiRequestContext, stage: Exclude<AiRequestStage, "idle">) {
    assertRequestCurrent(request);
    requestStage.value = stage;
  }

  async function waitForRequest<T>(promise: Promise<T>, request: AiRequestContext): Promise<T> {
    // The caller may already have started IPC. Observe rejection even if this
    // request expired before entering the race.
    void promise.catch(() => {});
    assertRequestCurrent(request);
    const remaining = request.deadline - Date.now();
    if (remaining <= 0) {
      const error = new AiRequestTimeoutError();
      request.abortController.abort(error);
      throw error;
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const result = await Promise.race([
        promise,
        request.cancelledPromise.then(() => {
          throw new AiRequestCancelledError();
        }),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => {
            const error = new AiRequestTimeoutError();
            request.abortController.abort(error);
            reject(error);
          }, remaining);
        }),
      ]);
      assertRequestCurrent(request);
      return result;
    } finally {
      if (timer) clearTimeout(timer);
    }
  }

  function finishRequest(request: AiRequestContext) {
    if (activeRequest !== request) return;
    activeRequest = null;
    activeRequestId.value = null;
    requestStage.value = "idle";
  }

  function cancelActiveRequest() {
    const request = activeRequest;
    if (!request) return;
    request.cancelled = true;
    request.abortController.abort(new AiRequestCancelledError());
    request.resolveCancelled();
    onCancel(request);
    finishRequest(request);
  }

  return { requestStage, activeRequestId, requestInFlight, requestStageLabel,
    beginRequest, assertRequestCurrent, setRequestStage, waitForRequest, finishRequest, cancelActiveRequest };
}
