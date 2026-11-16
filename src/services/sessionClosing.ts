import { invoke } from "@tauri-apps/api/core";
import type { DeviceSession, DeviceGroup } from "../types/device";

interface Workspace {
  sessions: DeviceSession[];
  deviceGroups: DeviceGroup[];
  activePort: string;
}
const sessionCloses = new WeakMap<DeviceSession, Promise<void>>();
const groupCloses = new WeakMap<DeviceGroup, Promise<void>>();

function disconnect(workspace: Workspace, session: DeviceSession, generation: number): Promise<void> {
  if (session.generation !== generation || workspace.sessions.some((s) => s.port === session.port && s !== session))
    return Promise.reject(new Error(`${session.port} 会话已变化，请确认当前连接后重试`));
  const pending = sessionCloses.get(session);
  if (pending) return pending;
  session.closing = true;
  const task = Promise.resolve().then(async () => {
    if (session.connected) await invoke("close_serial", { port: session.port, sessionGeneration: generation });
    if (session.generation !== generation || workspace.sessions.some((s) => s.port === session.port && s !== session)) {
      throw new Error(`${session.port} 会话已变化，请确认当前连接后重试`);
    }
    session.connected = false;
  }).finally(() => { session.closing = false; sessionCloses.delete(session); });
  sessionCloses.set(session, task);
  return task;
}

function preserveSelection(workspace: Workspace) {
  if (workspace.sessions.some((s) => s.port === workspace.activePort)) return;
  const logPorts = new Set(workspace.deviceGroups.map((g) => g.logPort));
  workspace.activePort = workspace.deviceGroups[0]?.atPort
    ?? workspace.sessions.find((s) => !logPorts.has(s.port))?.port ?? "";
}

export function closeGroup(workspace: Workspace, group: DeviceGroup, targets: { session: DeviceSession; generation: number }[]): Promise<void> {
  if (!workspace.deviceGroups.includes(group)) return Promise.resolve();
  const pending = groupCloses.get(group);
  if (pending) return pending;
  group.closing = true;
  const task = Promise.resolve().then(async () => {
    const errors: string[] = [];
    for (const { session, generation } of targets) {
      try { await disconnect(workspace, session, generation); }
      catch (error) { errors.push(`${session.port}: ${String(error)}`); }
    }
    if (errors.length) throw new Error(`设备未完全关闭，已保留连接状态，可重试。${errors.join("；")}`);
    // A reconnect may finish while another member of the group is closing.
    if (targets.some(({ session, generation }) => workspace.sessions.some((current) =>
      current.port === session.port && (current !== session || current.generation !== generation)))) {
      throw new Error("设备连接已变化，已保留新连接，请确认后重试");
    }
    workspace.deviceGroups = workspace.deviceGroups.filter((g) => g !== group);
    workspace.sessions = workspace.sessions.filter((s) => !targets.some((t) => t.session === s));
    preserveSelection(workspace);
  }).finally(() => { group.closing = false; groupCloses.delete(group); });
  groupCloses.set(group, task);
  return task;
}

export async function closeSession(workspace: Workspace, session: DeviceSession, generation: number): Promise<void> {
  await disconnect(workspace, session, generation);
  if (session.generation !== generation) throw new Error(`${session.port} 会话已变化，已保留当前连接`);
  workspace.sessions = workspace.sessions.filter((s) => s !== session);
  preserveSelection(workspace);
}
