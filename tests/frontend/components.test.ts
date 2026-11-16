// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { NButton, NCheckbox } from "naive-ui";
import type { Frame } from "../../src/types";
import type { UiItem } from "../../src/types/ai";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

import AiToolCard from "../../src/components/ai/AiToolCard.vue";
import TerminalLog from "../../src/components/console/TerminalLog.vue";
import App from "../../src/App.vue";
import { useSerialStore } from "../../src/stores/serial";
import { useExecutionStore } from "../../src/stores/execution";
import { useUiPrefs } from "../../src/composables/useUiPrefs";

const mounted: VueWrapper[] = [];
function render(component: any, options: any) {
  const wrapper = mount(component, options);
  mounted.push(wrapper);
  return wrapper;
}
afterEach(() => {
  mounted.splice(0).forEach((wrapper) => wrapper.unmount());
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("AI confirmation card", () => {
  const item = (): UiItem => ({ kind: "card", card: {
    toolCallId: "call-1", purpose: "读取信号", status: "pending", source: "llm", results: [],
    target: { port: "COM_A", sessionGeneration: 7, profileId: "ml307r" },
    steps: [{ cmd: "AT+CSQ", checked: true }],
  } });

  it("blocks invalid targets and reports selection changes without mutating the card", async () => {
    const card = item();
    const wrapper = render(AiToolCard, { props: { item: card, busy: false, targetAvailable: false } });
    expect(wrapper.text()).toContain("COM_A");
    expect(wrapper.text()).toContain("目标会话已失效");
    expect(wrapper.findAllComponents(NButton)[0].props("disabled")).toBe(true);
    wrapper.findComponent(NCheckbox).vm.$emit("update:checked", false);
    expect(wrapper.emitted("toggleStep")).toEqual([[0, false]]);
    expect(card.card!.steps[0].checked).toBe(true);
    await wrapper.setProps({ targetAvailable: true });
    await wrapper.findAll("button")[0].trigger("click");
    expect(wrapper.emitted("execute")).toHaveLength(1);
  });

  it("locks selection while busy and shows a failed card without execution buttons", async () => {
    const card = item();
    const wrapper = render(AiToolCard, { props: { item: card, busy: true, targetAvailable: true } });
    expect(wrapper.findComponent(NCheckbox).props("disabled")).toBe(true);
    await wrapper.setProps({ item: { ...card, card: { ...card.card, status: "failed", results: ["TIMEOUT"] } }, busy: false });
    expect(wrapper.text()).toContain("执行失败");
    expect(wrapper.text()).toContain("TIMEOUT");
    expect(wrapper.findAll("button")).toHaveLength(0);
  });
});

describe("terminal buffer rendering", () => {
  const frame = (seq: number): Frame => ({ seq, ts: 1, port: "COM_A", dir: "rx", kind: "response", text: `line ${seq}` });
  it("scrolls when a capped buffer replaces a row, and respects scroll pause", async () => {
    const wrapper = render(TerminalLog, { props: { frames: [frame(1), frame(2)], sourceKey: "COM_A:7", emptyMessage: "等待数据" } });
    const scroll = vi.spyOn(wrapper.element as HTMLElement, "scrollTo");
    await wrapper.setProps({ frames: [frame(2), frame(3)] });
    expect(scroll).toHaveBeenCalledOnce();
    scroll.mockClear();
    await wrapper.setProps({ autoScroll: false, frames: [frame(3), frame(4)] });
    expect(scroll).not.toHaveBeenCalled();
    await wrapper.setProps({ autoScroll: true });
    expect(scroll).toHaveBeenCalledOnce();
    expect(wrapper.text()).toContain("line 4");
    expect(wrapper.text()).not.toContain("line 1");
  });

  it("preserves Unicode hex output and error annotations after extraction", () => {
    const wrapper = render(TerminalLog, { props: {
      frames: [{ ...frame(1), text: "中" }, { ...frame(2), text: "+CME ERROR: 10", kind: "final" }],
      sourceKey: "COM_A:7", showHex: true, emptyMessage: "等待数据", hiddenCount: 5,
      urcPatterns: [{ pattern: "[", label: "invalid", severity_map: {} }],
      errorCodes: { "cme:10": { meaning: "SIM 卡未插入", causes: [], suggestions: [], code: 10, severity: "error", related_at: [] } },
    } });
    expect(wrapper.text()).toContain("E4 B8 AD");
    expect(wrapper.text()).toContain("SIM 卡未插入");
    expect(wrapper.text()).toContain("还有 5 行未显示");
  });
});

it("renders all lazy pages and the history drawer without global UI registration", async () => {
  const warn = vi.spyOn(console, "warn");
  mocks.invoke.mockImplementation(async (command) => {
    switch (command) {
      case "get_knowledge": return { profiles: [], error_codes: {}, profile_error_codes: {}, flows: [], at_docs: [], warnings: [] };
      case "get_settings": return { ai: { base_url: "", api_key_configured: false }, kb: { manuals_dir: "" } };
      case "get_knowledge_capabilities": return { coreAvailable: true, manualMode: "none", installedPacks: [], modelCoverage: [], fulltextModelCoverage: [], topicCount: 0, degradationReason: null };
      case "list_capability_observations": return { observations: [], portFirmware: {} };
      case "get_capability_evidence_context": return "";
      case "load_quickcmds": return null;
      case "save_quickcmds": return null;
      case "open_serial_ports": case "list_serial_ports": case "list_executions": case "list_applicable_flows": return [];
      default: throw new Error(`Unexpected test IPC: ${command}`);
    }
  });
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = render(App, { attachTo: document.body, global: { plugins: [pinia] } });
  const { activeView } = useUiPrefs();
  const pages = { console: ".console", diag: ".diag", ai: ".ai", kb: ".kb-page", stress: ".stress", logs: ".log-analysis" } as const;
  for (const [view, selector] of Object.entries(pages)) {
    activeView.value = view as keyof typeof pages;
    await vi.waitFor(async () => { await flushPromises(); expect(wrapper.find(selector).exists()).toBe(true); }, { timeout: 5000 });
  }
  activeView.value = "console";
  await flushPromises();
  await wrapper.findAll("button").find((button) => button.text() === "展开快捷指令")!.trigger("click");
  await flushPromises();
  const createButton = wrapper.findAll("button").find((button) => button.text() === "或手动新建一条")!;
  await createButton.trigger("click");
  await flushPromises();
  const commandInput = document.querySelector('textarea[placeholder="AT 指令或数据"]') as HTMLTextAreaElement;
  expect(commandInput).not.toBeNull();
  commandInput.value = "AT+CSQ";
  commandInput.dispatchEvent(new Event("input", { bubbles: true }));
  await flushPromises();
  const saveButton = [...document.querySelectorAll(".n-modal button")].find((button) => button.textContent?.trim() === "保存") as HTMLButtonElement;
  saveButton.click();
  await flushPromises();
  expect(useSerialStore().quickCmds?.groups[0].items[0].text).toBe("AT+CSQ");
  useExecutionStore().persistenceError = "测试保存错误";
  await wrapper.find(".execution-entry").trigger("click");
  await vi.waitFor(async () => { await flushPromises(); expect(document.body.textContent).toContain("执行记录"); expect(document.querySelector(".n-drawer")).not.toBeNull(); }, { timeout: 5000 });
  expect(warn.mock.calls.filter((args) => /Failed to resolve component/.test(String(args[0])))).toEqual([]);
  expect(document.querySelector(".n-alert")?.textContent).toContain("测试保存错误");
  expect(useSerialStore().listening).toBe(true);
}, 20_000);
