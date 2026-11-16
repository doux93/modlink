import { describe, expect, it, vi } from "vitest";
import { createPinia } from "pinia";
import { createSerialEvents } from "../../src/services/serialEvents";
import type { Frame } from "../../src/types";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), handlers: new Map<string, (event: any) => void>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (name: string, handler: (event: any) => void) => { mocks.handlers.set(name, handler); return () => mocks.handlers.delete(name); } }));
import { useSerialStore } from "../../src/stores/serial";
const session = (generation: number, port = "COM3") => ({ port, generation, baud: 115200, connected: true, profileId: "generic", firmware: "" });
const frame = (generation: number, port = "COM3"): Frame => ({ port, generation, seq: generation, ts: 1, dir: "rx", kind: "response", text: "OK" });
function setup() {
  const state = { sessions: [session(2)], framesByPort: {} as Record<string, Frame[]>, txBytes: 0, rxBytes: 0, lastError: "" };
  return { state, events: createSerialEvents(state) };
}
describe("serial event generations", () => {
  it("ignores old disconnects and frames after reconnect without counting their bytes", () => {
    const { state, events } = setup();
    events.receive(frame(1)); events.receive({ port: "COM3", generation: 1, error: "old disconnect" });
    expect(state.sessions[0].connected).toBe(true); expect(state.framesByPort.COM3).toBeUndefined();
    expect(state.rxBytes).toBe(0); expect(state.lastError).toBe("");
    events.receive(frame(2)); expect(state.framesByPort.COM3).toHaveLength(1);
  });
  it("rejects unversioned and removed-port events", () => {
    const { state, events } = setup();
    events.receive({ ...frame(2), generation: undefined } as unknown as Frame);
    events.receive(frame(2, "COM9")); expect(Object.keys(state.framesByPort)).toHaveLength(0);
  });
  it("buffers early events until registration and replays only the opened connection", () => {
    const { state, events } = setup(); events.beginOpen("COM3");
    events.receive(frame(2)); events.receive(frame(3));
    events.receive({ port: "COM3", generation: 2, error: "old" });
    state.sessions = [session(3)]; events.endOpen("COM3");
    expect(state.framesByPort.COM3.map(f => f.generation)).toEqual([3]);
    expect(state.sessions[0].connected).toBe(true);
  });
  it("retains an early disconnect and isolates the other port during startup", () => {
    const { state, events } = setup(); events.beginInit();
    events.receive(frame(5, "COM4")); events.receive({ port: "COM4", generation: 5, error: "unplugged" });
    state.sessions.push(session(5, "COM4")); events.endInit();
    expect(state.sessions[1].connected).toBe(false); expect(state.sessions[0].connected).toBe(true);
    expect(state.framesByPort.COM4).toHaveLength(1);
  });
  it("does not lose a frame delivered before the open IPC resolves", async () => {
    mocks.invoke.mockImplementation(async (command, args) => {
      if (command === "get_knowledge") return { profiles: [{ id: "generic" }] };
      if (command === "load_quickcmds") throw new Error("corrupt file");
      if (command === "open_serial") {
        mocks.handlers.get("serial://frames")!({ payload: [frame(8)] });
        return { port: args.port, generation: 8, baud: args.baud };
      }
      return [];
    });
    const state = useSerialStore(createPinia()); await state.init();
    expect(state.listening).toBe(true); expect(state.quickError).toContain("corrupt file");
    await state.openSession("COM3");
    expect(state.framesByPort.COM3[0].generation).toBe(8);
    mocks.handlers.get("serial://error")!({ payload: { port: "COM3", generation: 7, error: "old" } });
    expect(state.activeSession?.connected).toBe(true);
  });
});
