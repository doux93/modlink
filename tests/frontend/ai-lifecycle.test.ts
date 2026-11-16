// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { defineComponent } from "vue";
import { NConfigProvider, NMessageProvider, NDialogProvider } from "naive-ui";
import Ai from "../../src/views/Ai.vue";
import { useSerialStore } from "../../src/stores/serial";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
let wrapper: VueWrapper | undefined;
afterEach(() => { wrapper?.unmount(); wrapper = undefined; mocks.invoke.mockReset(); });

describe("AI page request lifecycle", () => {
  it.each(["cancel", "clear", "device", "unmount"])("aborts backend on %s and discards stale content", async (action) => {
    let finish!: (value: unknown) => void;
    mocks.invoke.mockImplementation(async (command) => {
      switch (command) {
        case "get_settings": return { ai: { base_url: "http://localhost/v1", model: "test", api_key_configured: true } };
        case "get_knowledge_capabilities": return { manualMode: "none", coreAvailable: true, installedPacks: [], modelCoverage: [], fulltextModelCoverage: [], topicCount: 0 };
        case "get_capability_evidence_context": return "";
        case "prepare_ai_request": case "cancel_ai_request": return true;
        case "ai_chat": return new Promise((resolve) => { finish = resolve; });
        default: throw new Error(`Unexpected IPC: ${command}`);
      }
    });
    const pinia = createPinia();
    setActivePinia(pinia);
    const serial = useSerialStore();
    serial.listening = true;
    serial.sessions = [{ port: "COM_A", baud: 115200, generation: 1, connected: true, profileId: "generic", firmware: "" }];
    serial.activePort = "COM_A";
    const host = defineComponent({ components: { Ai, NConfigProvider, NMessageProvider, NDialogProvider }, template: "<NConfigProvider><NMessageProvider><NDialogProvider><Ai /></NDialogProvider></NMessageProvider></NConfigProvider>" });
    wrapper = mount(host, { global: { plugins: [pinia] } });
    await flushPromises();
    await wrapper.find("textarea").setValue("分析日志");
    await wrapper.find("textarea").trigger("keydown", { key: "Enter" });
    await vi.waitFor(async () => { await flushPromises(); expect(finish).toBeTypeOf("function"); });
    const requestId = mocks.invoke.mock.calls.find(([command]) => command === "ai_chat")![1].requestId;
    if (action === "cancel" || action === "clear") {
      await wrapper.findAll("button").find((button) => button.text() === (action === "cancel" ? "取消请求" : "清空对话"))!.trigger("click");
    } else if (action === "device") {
      serial.sessions[0].generation += 1;
    } else {
      wrapper.unmount(); wrapper = undefined;
    }
    await flushPromises();
    expect(mocks.invoke).toHaveBeenCalledWith("cancel_ai_request", { requestId });
    finish({ choices: [{ message: { role: "assistant", content: "STALE-ANSWER" } }] });
    await flushPromises();
    if (wrapper) {
      expect(wrapper.text()).not.toContain("STALE-ANSWER");
      expect(wrapper.text()).not.toContain("规则引擎模式（离线）");
      expect(wrapper.find("textarea").attributes("disabled")).toBeUndefined();
    }
  });
});
