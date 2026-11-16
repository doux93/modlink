import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPersistence, type QuickWorkspace } from "../../src/services/quickCommandPersistence";
import { createCloseGuard } from "../../src/services/closeGuard";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => { invoke.mockReset(); });
afterEach(() => { vi.useRealTimers(); });
function setup() {
  const state: QuickWorkspace = { quickCmds: { schema: "modulink/quickcmds@1", name: "original", groups: [], loop: { times: 1, interval_ms: 300 }, warnings: [] },
    quickLoaded: true, quickLoading: false, quickSaving: false, quickRevision: 1, quickSavedRevision: 0, quickError: "" };
  return { state, persistence: createPersistence(state) };
}
describe("quick command persistence", () => {
  it("flushes before the debounce expires and does not write again afterwards", async () => {
    vi.useFakeTimers(); invoke.mockResolvedValue(null);
    const { state, persistence } = setup(); persistence.schedule();
    await persistence.flush(); expect(invoke).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(1000); expect(invoke).toHaveBeenCalledOnce();
    expect(state.quickSavedRevision).toBe(1);
  });
  it("serializes writes and drains edits made while an earlier snapshot is saving", async () => {
    const { state, persistence } = setup(); let finish!: () => void;
    invoke.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; })).mockResolvedValue(null);
    const first = persistence.flush(); await Promise.resolve();
    state.quickCmds!.name = "latest"; state.quickRevision++;
    expect(persistence.flush()).toBe(first); expect(invoke).toHaveBeenCalledOnce();
    expect(invoke.mock.calls[0][1].data.name).toBe("original");
    finish(); await first;
    expect(invoke).toHaveBeenCalledTimes(2); expect(invoke.mock.calls[1][1].data.name).toBe("latest");
    expect(state.quickSavedRevision).toBe(2);
  });
  it("keeps failed edits dirty and clears the error only after a successful retry", async () => {
    const { state, persistence } = setup(); invoke.mockRejectedValueOnce(new Error("disk denied"));
    await expect(persistence.flush()).rejects.toThrow("disk denied");
    expect(state.quickSavedRevision).toBe(0); expect(state.quickError).toContain("尚未保存");
    invoke.mockResolvedValue(null); await persistence.flush();
    expect(state.quickSavedRevision).toBe(1); expect(state.quickError).toBe("");
  });
  it("blocks edits after an unreadable file until a load or explicit recovery succeeds", async () => {
    const { state, persistence } = setup(); state.quickRevision = 0; state.quickCmds = null;
    invoke.mockRejectedValueOnce(new Error("malformed JSON"));
    expect(await persistence.read()).toBe(false); expect(state.quickLoaded).toBe(false);
    state.quickRevision++;
    await expect(persistence.flush()).rejects.toThrow("未覆盖原文件");
    expect(invoke).toHaveBeenCalledOnce(); state.quickRevision = 0;
    invoke.mockResolvedValue({ name: "backup" });
    expect(await persistence.read(true)).toBe(true); expect(state.quickCmds!.name).toBe("backup");
    expect(invoke).toHaveBeenLastCalledWith("restore_quickcmds");
  });
});
describe("exit before autosave", () => {
  it("prevents closing synchronously, deduplicates close clicks and exits only after saving", async () => {
    let finish!: () => void;
    const flush = vi.fn(() => new Promise<void>(resolve => { finish = resolve; }));
    const destroy = vi.fn(async () => {}); const preventDefault = vi.fn();
    const close = createCloseGuard({ dirty: () => true, flush, destroy, setClosing: vi.fn(), failed: vi.fn() });
    const pending = close({ preventDefault }); expect(preventDefault).toHaveBeenCalledOnce();
    await close({ preventDefault }); expect(flush).toHaveBeenCalledOnce(); expect(destroy).not.toHaveBeenCalled();
    finish(); await pending; expect(destroy).toHaveBeenCalledOnce();
  });
  it("keeps the window open after a failed save and permits a later retry", async () => {
    const flush = vi.fn().mockRejectedValueOnce(new Error("disk full")).mockResolvedValue(undefined);
    const destroy = vi.fn(async () => {}); const failed = vi.fn(); const setClosing = vi.fn();
    const close = createCloseGuard({ dirty: () => true, flush, destroy, failed, setClosing });
    await close({ preventDefault: vi.fn() }); expect(destroy).not.toHaveBeenCalled(); expect(failed).toHaveBeenCalledOnce();
    expect(setClosing).toHaveBeenLastCalledWith(false);
    await close({ preventDefault: vi.fn() }); expect(destroy).toHaveBeenCalledOnce();
  });
});
