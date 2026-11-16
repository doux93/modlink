import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { requestAiChat } from "../../src/utils/aiTransport";
import { AiRequestCancelledError, AiRequestTimeoutError, useAiRequestGate } from "../../src/composables/useAiRequestGate";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
const cfg = { base_url: "http://localhost/v1", model: "test" };
const deferred = <T>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const gate = () => useAiRequestGate(() => undefined, () => "no-device", () => 0, () => 0, vi.fn());
beforeEach(() => mocks.invoke.mockReset());
afterEach(() => vi.useRealTimers());

describe("cancellable AI transport", () => {
  it("does not register an already cancelled or expired request", async () => {
    const controller = new AbortController();
    controller.abort(new AiRequestCancelledError());
    await expect(requestAiChat(cfg, {}, { signal: controller.signal, deadline: Date.now() + 1000 })).rejects.toBeInstanceOf(AiRequestCancelledError);
    await expect(requestAiChat(cfg, {}, { signal: new AbortController().signal, deadline: 0 })).rejects.toBeInstanceOf(AiRequestTimeoutError);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("cancels a late registration without sending HTTP after the UI is cancelled", async () => {
    const registration = deferred<void>();
    mocks.invoke.mockImplementation((command) => command === "prepare_ai_request" ? registration.promise : Promise.resolve(true));
    const requestGate = gate();
    const request = requestGate.beginRequest("waiting", 0);
    const waiting = requestGate.waitForRequest(requestAiChat(cfg, {}, { signal: request.abortController.signal, deadline: request.deadline }), request);
    requestGate.cancelActiveRequest();
    await expect(waiting).rejects.toBeInstanceOf(AiRequestCancelledError);
    registration.resolve();
    await vi.waitFor(() => expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual(["prepare_ai_request", "cancel_ai_request"]));
    expect(mocks.invoke.mock.calls[1][1].requestId).toBe(mocks.invoke.mock.calls[0][1].requestId);
  });

  it("aborts the backend at the shared deadline and ignores a late answer", async () => {
    vi.useFakeTimers();
    const response = deferred<unknown>();
    mocks.invoke.mockImplementation((command) => command === "ai_chat" ? response.promise : Promise.resolve());
    const requestGate = gate();
    const request = requestGate.beginRequest("waiting", 0);
    const waiting = requestGate.waitForRequest(requestAiChat(cfg, {}, { signal: request.abortController.signal, deadline: request.deadline }), request);
    const rejected = expect(waiting).rejects.toBeInstanceOf(AiRequestTimeoutError);
    await vi.advanceTimersByTimeAsync(30_000);
    await rejected;
    expect(request.abortController.signal.aborted).toBe(true);
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "cancel_ai_request")).toHaveLength(1);
    response.resolve({ content: "too late" });
    await Promise.resolve();
    expect(() => requestGate.assertRequestCurrent(request)).toThrow(AiRequestTimeoutError);
  });

  it("does not let an old cancel target the next HTTP request", async () => {
    const responses = [deferred<string>(), deferred<string>()];
    let index = 0;
    mocks.invoke.mockImplementation((command) => command === "ai_chat" ? responses[index++].promise : Promise.resolve());
    const first = new AbortController();
    const second = new AbortController();
    const one = requestAiChat<string>(cfg, {}, { signal: first.signal, deadline: Date.now() + 10_000 });
    const two = requestAiChat<string>(cfg, {}, { signal: second.signal, deadline: Date.now() + 10_000 });
    await vi.waitFor(() => expect(index).toBe(2));
    const chats = mocks.invoke.mock.calls.filter(([command]) => command === "ai_chat");
    expect(chats[0][1].requestId).not.toBe(chats[1][1].requestId);
    first.abort(new AiRequestCancelledError());
    await expect(one).rejects.toBeInstanceOf(AiRequestCancelledError);
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "cancel_ai_request").map(([, args]) => args.requestId)).toEqual([chats[0][1].requestId]);
    responses[1].resolve("new answer");
    await expect(two).resolves.toBe("new answer");
    responses[0].reject(new Error("late network failure"));
  });

  it("cleans up failed IPC and limits backend time to the remaining request budget", async () => {
    mocks.invoke.mockImplementation((command) => command === "ai_chat" ? Promise.reject(new Error("network failure")) : Promise.resolve());
    const request = requestAiChat(cfg, {}, { signal: new AbortController().signal, deadline: Date.now() + 1234 });
    await expect(request).rejects.toThrow("network failure");
    const args = mocks.invoke.mock.calls.find(([command]) => command === "ai_chat")![1];
    expect(args.timeoutMs).toBeGreaterThan(0);
    expect(args.timeoutMs).toBeLessThanOrEqual(1234);
    expect(mocks.invoke).toHaveBeenLastCalledWith("cancel_ai_request", { requestId: args.requestId });
  });

  it("expires while registering without starting HTTP", async () => {
    vi.useFakeTimers();
    const registration = deferred<void>();
    mocks.invoke.mockImplementation((command) => command === "prepare_ai_request" ? registration.promise : Promise.resolve());
    const request = requestAiChat(cfg, {}, { signal: new AbortController().signal, deadline: Date.now() + 20 });
    const rejected = expect(request).rejects.toBeInstanceOf(AiRequestTimeoutError);
    await vi.advanceTimersByTimeAsync(21);
    registration.resolve();
    await rejected;
    expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual(["prepare_ai_request", "cancel_ai_request"]);
  });
});
