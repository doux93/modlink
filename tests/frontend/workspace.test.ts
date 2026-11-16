// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { defineComponent } from "vue";
import { NConfigProvider, NMessageProvider, NDialogProvider, NSelect } from "naive-ui";
import Connection from "../../src/components/DeviceConnectionDialog.vue";
import DeviceContextBar from "../../src/components/DeviceContextBar.vue";
import Console from "../../src/views/Console.vue";
import Diagnosis from "../../src/views/Diagnosis.vue";
import Welcome from "../../src/components/onboarding/WelcomeOverlay.vue";
import { useSerialStore } from "../../src/stores/serial";
import { useExecutionStore } from "../../src/stores/execution";
import { useWorkspace } from "../../src/composables/useWorkspace";
import { useUiPrefs } from "../../src/composables/useUiPrefs";
import type { DiagFlow, DiagReport, KnowledgeStore } from "../../src/types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn(), open: vi.fn(), confirm: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save, open: mocks.open }));
let wrapper: VueWrapper | undefined;
const flow = (): DiagFlow => ({ id: "network_attach", name: "入网连通性检查", kind: "checkup", verified: true, category: "network", execution_mode: "readonly", applies_to: [], steps: [{ id: "sim", title: "SIM 卡", command: "AT+CPIN?" }] } as unknown as DiagFlow);
const report = (): DiagReport => ({ flow_id: "network_attach", flow_name: "入网连通性检查", overall: "fail", steps: [{ id: "sim", title: "SIM 卡", result: "fail", detail: "SIM 未就绪", reason: "未检测到 SIM", suggestion: "检查卡槽后复测", raw: ["+CME ERROR: 10"], collected: [], elapsed_ms: 20 }] } as DiagReport);
function setup(component: any, connected = false) {
  const pinia = createPinia(); setActivePinia(pinia);
  const serial = useSerialStore(); serial.listening = true; serial.quickLoaded = true; serial.quickGuardReady = true;
  serial.knowledge = { profiles: [], flows: [flow()], error_codes: {}, profile_error_codes: {}, at_docs: [], warnings: [] } as unknown as KnowledgeStore;
  serial.defaultProfileId = "generic";
  if (connected) {
    serial.sessions = [{ port: "COM_A", baud: 115200, generation: 7, connected: true, profileId: "generic", firmware: "" }, { port: "COM_B", baud: 115200, generation: 9, connected: true, profileId: "generic", firmware: "" }];
    serial.activePort = "COM_A";
  }
  const host = defineComponent({ components: { UnderTest: component, NConfigProvider, NMessageProvider, NDialogProvider }, template: "<NConfigProvider><NMessageProvider><NDialogProvider><UnderTest /></NDialogProvider></NMessageProvider></NConfigProvider>" });
  wrapper = mount(host, { attachTo: document.body, global: { plugins: [pinia] } });
  return serial;
}
async function button(text: string) {
  await flushPromises();
  const b = [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === text);
  expect(b, text).toBeTruthy(); return b!;
}
beforeEach(() => {
  mocks.invoke.mockReset(); mocks.save.mockReset(); mocks.open.mockReset();
  mocks.invoke.mockImplementation(async (command, args) => {
    if (command === "list_serial_ports") return [{ name: "COM_A", description: "Test device" }, { name: "COM_B", description: "Log port" }];
    if (command === "open_serial") return { port: args.port, baud: args.baud, generation: 10 };
    if (command === "list_applicable_flows") return ["network_attach"];
    if (command === "run_diagnostic") return report();
    if (command === "cancel_diagnostic" || command === "save_text_file" || command === "export_session_log" || command === "export_legacy") return null;
    throw new Error(`Unexpected IPC: ${command}`);
  });
  useWorkspace().connectionOpen.value = false; useWorkspace().historyOpen.value = false;
  useUiPrefs().quickPanelPreference.value = "auto";
});
afterEach(() => {
  wrapper?.unmount(); wrapper = undefined;
  useWorkspace().connectionOpen.value = false; useWorkspace().historyOpen.value = false;
  document.body.innerHTML = "";
});

describe("shared connection flow", () => {
  it("validates custom baud, shows connection errors, and retries without losing input", async () => {
    const serial = setup(Connection);
    useWorkspace().connectionOpen.value = true;
    await flushPromises();
    const baud = document.querySelector<HTMLInputElement>("#connect-baud")!;
    baud.value = "0"; baud.dispatchEvent(new Event("input", { bubbles: true }));
    expect((await button("连接并识别")).disabled).toBe(true);
    baud.value = "57600"; baud.dispatchEvent(new Event("input", { bubbles: true }));
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(async (command, args) => { if (command === "open_serial") throw new Error("access denied"); return original(command, args); });
    (await button("连接并识别")).click(); await flushPromises();
    await vi.waitFor(() => expect(document.querySelector('[role="alert"]')?.textContent).toContain("端口占用"));
    expect(useWorkspace().connectionOpen.value).toBe(true);
    mocks.invoke.mockImplementation(original);
    (await button("连接并识别")).click(); await flushPromises();
    expect(mocks.invoke).toHaveBeenCalledWith("open_serial", { port: "COM_A", baud: 57600 });
    expect(serial.activePort).toBe("COM_A");
    expect(serial.sessions).toHaveLength(1);
    expect(useWorkspace().connectionOpen.value).toBe(false);
  });
  it("reconnects a disconnected device without duplicating its session", async () => {
    const serial = setup(Connection, true);
    serial.sessions[0].connected = false;
    serial.selPort = "COM_A";
    useWorkspace().connectionOpen.value = true; await flushPromises();
    expect((await button("连接并识别")).disabled).toBe(false);
    (await button("连接并识别")).click(); await flushPromises();
    expect(serial.sessions.filter((s) => s.port === "COM_A")).toHaveLength(1);
    expect(serial.activeSession?.generation).toBe(10);
  });
  it("welcome emits navigation before dismissing", async () => {
    setup(Welcome);
    (await button("打开串口控制台")).click(); await flushPromises();
    expect(wrapper!.findComponent(Welcome).emitted("navigate")).toEqual([["console"]]);
    expect(wrapper!.text()).not.toContain("30 秒");
  });
});

describe("device-bound diagnosis and review", () => {
  it("offers direct connection while offline, without an independent port selector", async () => {
    setup(Diagnosis); await flushPromises();
    (await button("连接设备")).click();
    expect(useWorkspace().connectionOpen.value).toBe(true);
    expect(wrapper!.findAllComponents(NSelect)).toHaveLength(1);
    expect((await button("开始检查")).disabled).toBe(true);
  });
  it("keeps in-flight results and export attached to their original device", async () => {
    let finish!: (r: DiagReport) => void;
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(async (command, args) => command === "run_diagnostic" ? new Promise((resolve) => { finish = resolve; }) : original(command, args));
    const serial = setup(Diagnosis, true); await flushPromises();
    (await button("开始检查")).click(); await flushPromises();
    expect(mocks.invoke).toHaveBeenCalledWith("run_diagnostic", expect.objectContaining({ port: "COM_A", sessionGeneration: 7 }));
    serial.activePort = "COM_B"; await flushPromises();
    finish(report()); await flushPromises();
    expect(wrapper!.find(".run-target").text()).toContain("COM_A");
    expect(wrapper!.find(".run-target").text()).toContain("当前设备已变化");
    expect((await button("处理后复测")).disabled).toBe(true);
    expect((await button("AI 分析异常")).disabled).toBe(true);
    mocks.save.mockResolvedValue("report.html");
    (await button("导出报告")).click(); await flushPromises();
    const exported = mocks.invoke.mock.calls.find(([command]) => command === "save_text_file")![1].content;
    expect(exported).toContain("端口：COM_A"); expect(exported).not.toContain("端口：COM_B");
    serial.activePort = "COM_A"; await flushPromises();
    expect(wrapper!.find(".result-review").exists()).toBe(true);
    expect((await button("处理后复测")).disabled).toBe(false);
  });
  it("prefills AI from the result without sending and repeats the same diagnostic on explicit retest", async () => {
    setup(Diagnosis, true); await flushPromises();
    (await button("开始检查")).click(); await flushPromises();
    (await button("AI 分析异常")).click(); await flushPromises();
    expect(useExecutionStore().pendingAiPrompt).toContain("COM_A");
    expect(useExecutionStore().pendingAiPrompt).toContain("+CME ERROR: 10");
    expect(useUiPrefs().activeView.value).toBe("ai");
    expect(mocks.invoke.mock.calls.some(([command]) => command === "ai_chat")).toBe(false);
    (await button("处理后复测")).click(); await flushPromises();
    const runs = mocks.invoke.mock.calls.filter(([command]) => command === "run_diagnostic");
    expect(runs).toHaveLength(2);
    expect(runs[1][1]).toMatchObject({ port: "COM_A", sessionGeneration: 7, flowId: "network_attach" });
  });
  it("allows time to read confirmation and rejects a changed target before sending commands", async () => {
    const serial = setup(Diagnosis, true);
    serial.knowledge!.flows[0].verified = false;
    await flushPromises();
    (await button("开始检查")).click(); await flushPromises();
    await new Promise((resolve) => setTimeout(resolve, 1400));
    serial.activePort = "COM_B"; await flushPromises();
    (await button("确认并运行")).click(); await flushPromises();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "run_diagnostic")).toBe(false);
    expect(document.body.textContent).toContain("当前设备已变化");
  });
});


it("does not send serial data when Enter confirms an IME candidate", async () => {
  const serial = setup(Console, true);
  const write = vi.spyOn(serial, "write").mockResolvedValue();
  await flushPromises();
  const input = wrapper!.find(".input-bar textarea");
  await input.setValue("AT");
  await input.trigger("keydown", { key: "Enter", isComposing: true });
  await input.trigger("keydown", { key: "Enter", keyCode: 229 });
  await flushPromises();
  expect(write).not.toHaveBeenCalled();
  expect((input.element as HTMLTextAreaElement).value).toBe("AT");
  await input.trigger("keydown", { key: "Enter" });
  await flushPromises();
  expect(write).toHaveBeenCalledOnce();
});

it("starts without an empty quick panel and preserves a user's closed choice when commands arrive", async () => {
  const serial = setup(Console);
  await flushPromises();
  expect(wrapper!.find(".quick").isVisible()).toBe(false);
  (await button("展开快捷指令")).click(); await flushPromises();
  expect(wrapper!.find(".quick").isVisible()).toBe(true);
  (await button("收起快捷指令")).click(); await flushPromises();
  serial.quickCmds = { schema: "modulink/quickcmds@1", name: "saved", groups: [{ name: "default", items: [{ text: "AT", format: "text", selected: false, append_newline: true, note: "" }] }], loop: { times: 1, interval_ms: 300 }, warnings: [] };
  await flushPromises();
  expect(wrapper!.find(".quick").isVisible()).toBe(false);
  (await button("展开快捷指令")).click(); await flushPromises();
  expect(wrapper!.find(".quick-text").text()).toBe("AT");
});

it("separates serial connection from model identification and device-specific progress", async () => {
  const serial = setup(DeviceContextBar, true);
  await flushPromises();
  expect(wrapper!.find(".connection-state").text()).toBe("串口已连接");
  expect(wrapper!.find(".model-label").text()).toContain("型号待识别");
  expect(wrapper!.find(".device-availability").text()).toContain("通用基础诊断");
  serial.sessions[0].identification = "detecting"; await flushPromises();
  expect(wrapper!.find(".model-label").text()).toContain("正在识别型号");
  serial.activePort = "COM_B"; await flushPromises();
  expect(wrapper!.find(".model-label").text()).toContain("型号待识别");
  serial.sessions[1].connected = false; await flushPromises();
  expect(wrapper!.find(".connection-state").text()).toBe("串口已断开");
});

it("shows failed knowledge loading and allows retry instead of an endless loading message", async () => {
  setup(DeviceContextBar, true); await flushPromises();
  expect(wrapper!.text()).toContain("读取失败");
  expect(wrapper!.text()).not.toContain("正在读取");
  mocks.invoke.mockResolvedValue({ effectiveManualMode: "preview" });
  (await button("重试")).click(); await flushPromises();
  expect(wrapper!.text()).toContain("摘要参考");
  expect(wrapper!.text()).not.toContain("读取失败");
});

it("exports the originally selected session even if the current device changes while choosing a file", async () => {
  const serial = setup(Console, true);
  let choose!: (path: string) => void;
  mocks.save.mockReturnValue(new Promise(resolve => { choose = resolve; }));
  (await button("保存日志")).click(); await flushPromises();
  serial.activePort = "COM_B";
  choose("original.log"); await flushPromises();
  expect(mocks.invoke).toHaveBeenCalledWith("export_session_log", { port: "COM_A", sessionGeneration: 7, path: "original.log" });
});

it("retains logs after a failed export, shows a local error, and permits retry or cancellation", async () => {
  const serial = setup(Console, true);
  serial.framesByPort.COM_A = [{ generation: 7, seq: 1, ts: 1, port: 'COM_A', dir: 'rx', kind: 'response', text: 'retained' }];
  mocks.save.mockResolvedValue("capture.log");
  mocks.invoke.mockRejectedValueOnce(new Error("disk full"));
  (await button("保存日志")).click(); await flushPromises();
  expect(wrapper!.find('[role="alert"]').text()).toContain("disk full");
  expect(serial.framesByPort.COM_A).toHaveLength(1);
  mocks.invoke.mockResolvedValue(undefined);
  (await button("保存日志")).click(); await flushPromises();
  expect(wrapper!.find('[role="alert"]').exists()).toBe(false);
  mocks.invoke.mockClear(); mocks.save.mockResolvedValue(null);
  (await button("保存日志")).click(); await flushPromises();
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("exports a command snapshot and keeps editing available after an export failure", async () => {
  const serial = setup(Console);
  serial.quickCmds = { schema: "modulink/quickcmds@1", name: "saved", groups: [{ name: "default", items: [{ text: "AT", format: "text", selected: false, append_newline: true, note: "" }] }], loop: { times: 1, interval_ms: 300 }, warnings: [] };
  (await button("展开快捷指令")).click(); await flushPromises();
  let choose!: (path: string) => void;
  mocks.save.mockReturnValue(new Promise(resolve => { choose = resolve; }));
  (await button("导出")).click(); await flushPromises();
  serial.quickCmds.groups[0].items[0].text = "AT+CSQ";
  mocks.invoke.mockRejectedValueOnce(new Error("write denied"));
  choose("commands.ini"); await flushPromises();
  const call = mocks.invoke.mock.calls.find(([command]) => command === "export_legacy")!;
  expect(call[1].data.groups[0].items[0].text).toBe("AT");
  expect(wrapper!.find('.transfer-error').text()).toContain("write denied");
  expect(serial.quickCmds.groups[0].items[0].text).toBe("AT+CSQ");
  mocks.save.mockResolvedValue("retry.ini"); mocks.invoke.mockResolvedValue(undefined);
  (await button("导出")).click(); await flushPromises();
  expect(wrapper!.find('.transfer-error').exists()).toBe(false);
});
