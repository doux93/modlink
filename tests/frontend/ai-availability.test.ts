import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAiAvailability } from "../../src/composables/useAiAvailability";
import { useAiSettings } from "../../src/composables/useAiSettings";
const { invoke, request } = vi.hoisted(() => ({ invoke: vi.fn(), request: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("../../src/utils/aiTransport", () => ({ requestAiChat: request }));
beforeEach(() => { invoke.mockReset(); request.mockReset(); });
async function setup(key = true) {
  invoke.mockResolvedValue({ ai: { base_url: "https://example.test/v1", model: "test", api_key_configured: key } });
  const settings = useAiSettings(vi.fn()); const availability = useAiAvailability(settings);
  await settings.load(); return { settings, availability };
}
describe("truthful AI status", () => {
  it("does not treat a default URL without a key as available or send a test request", async () => {
    const { availability } = await setup(false);
    expect(availability.label.value).toBe("AI 待配置");
    expect(await availability.verify()).toBe(false); expect(request).not.toHaveBeenCalled();
  });
  it("marks availability only after an actual successful response and preserves failure for retry", async () => {
    const { availability } = await setup(); expect(availability.label.value).toBe("AI 尚未验证");
    request.mockRejectedValueOnce(new Error("401 unauthorized"));
    expect(await availability.verify()).toBe(false); expect(availability.label.value).toBe("AI 连接失败");
    request.mockResolvedValue({ choices: [{}] }); expect(await availability.verify()).toBe(true);
    expect(availability.label.value).toBe("AI 可用"); expect(availability.error.value).toBe("");
    expect(JSON.stringify(request.mock.calls[1][1])).not.toContain("设备");
  });
  it("deduplicates tests and ignores a successful response from a previous configuration", async () => {
    const { settings, availability } = await setup(); let finish!: (v: unknown) => void;
    request.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const pending = availability.verify(); expect(await availability.verify()).toBe(false);
    const signal = request.mock.calls[0][2].signal;
    settings.open(); settings.draft.value.model = "replacement"; await settings.save();
    expect(signal.aborted).toBe(true); finish({ choices: [{}] });
    expect(await pending).toBe(false); expect(availability.label.value).toBe("AI 尚未验证");
  });
  it("does not disguise a settings read failure as an unconfigured service", async () => {
    invoke.mockRejectedValue(new Error("credential store unavailable"));
    const settings = useAiSettings(vi.fn()); const availability = useAiAvailability(settings);
    await settings.load(); expect(availability.label.value).toBe("配置读取失败");
    expect(await availability.verify()).toBe(false);
  });
});
