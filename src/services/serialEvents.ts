import type { Frame } from "../types";
import type { DeviceSession } from "../types/device";

export interface SerialError { port: string; generation: number; error: string }
interface Workspace {
  sessions: DeviceSession[];
  framesByPort: Record<string, Frame[]>;
  lastError: string;
  txBytes: number;
  rxBytes: number;
}
type Event = Frame | SerialError;
const LIMIT = 20000;

/** Buffer events only while their session is being registered; never infer a generation. */
export function createSerialEvents(workspace: Workspace) {
  let initializing = false;
  const opening = new Set<string>();
  const buffered = new Map<string, Event[]>();
  function deliver(event: Event) {
    const session = workspace.sessions.find(s => s.port === event.port && s.generation === event.generation);
    if (!session) return;
    if ("error" in event) {
      session.connected = false;
      workspace.lastError = `${event.port}: ${event.error}`;
      return;
    }
    const bucket = workspace.framesByPort[event.port] ??= [];
    bucket.push(event);
    if (bucket.length > LIMIT) bucket.splice(0, bucket.length - LIMIT);
    const size = event.hex ? event.hex.replace(/\s/g, "").length / 2 : new TextEncoder().encode(event.text).length;
    if (event.dir === "tx") workspace.txBytes += size;
    else workspace.rxBytes += size;
  }
  function receive(event: Event) {
    if (!Number.isSafeInteger(event.generation) || event.generation <= 0) return;
    if (initializing || opening.has(event.port)) {
      if (!buffered.has(event.port) && buffered.size >= 32) return;
      const events = buffered.get(event.port) ?? [];
      // Preserve a disconnect even if a final batch follows it.
      if (events.length >= LIMIT) events.splice(Math.max(0, events.findIndex(e => !("error" in e))), 1);
      events.push(event); buffered.set(event.port, events);
    } else deliver(event);
  }
  function flush(port: string) {
    if (initializing || opening.has(port)) return;
    const events = buffered.get(port) ?? [];
    buffered.delete(port);
    events.forEach(deliver);
  }
  return {
    receive,
    beginInit() { initializing = true; },
    endInit() { initializing = false; [...buffered.keys()].forEach(flush); },
    beginOpen(port: string) {
      if (opening.has(port)) throw new Error(`${port} 正在连接，请稍候`);
      opening.add(port);
    },
    endOpen(port: string) { opening.delete(port); flush(port); },
  };
}
