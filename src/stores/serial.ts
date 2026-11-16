import { isAtCommand } from "../utils/atCommand";
import { probeProfile } from "../services/profileIdentification";
import type { DeviceSession, DeviceGroup } from "../types/device";
import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AtResult,
  ExecutionReport,
  ExecutionSource,
  ExecutionStep,
  ExecutionTarget,
  Frame,
  KnowledgeStore,
  ModuleProfile,
  OpenPortInfo,
  PortInfo,
  QuickCmdsFile,
} from "../types";

let persistTimer: ReturnType<typeof setTimeout> | null = null;
let serialInitPromise: Promise<void> | null = null;

const MAX_FRAMES_PER_PORT = 20000;

export type SessionInfo = DeviceSession;
// Requests belong to a session object. Reconnect/manual selection invalidates their result.
const identificationRuns = new WeakMap<SessionInfo, Promise<string | null>>();

export type { DeviceGroup } from "../types/device";

export const useSerialStore = defineStore("serial", {
  state: () => ({
    ports: [] as PortInfo[],
    /** 已打开的会话（多路串口，页签呈现） */
    sessions: [] as SessionInfo[],
    activePort: "" as string,
    /** 连接面板当前选择 */
    selPort: "" as string,
    baud: 115200,
    framesByPort: {} as Record<string, Frame[]>,
    listening: false,
    quickCmds: null as QuickCmdsFile | null,
    lastError: "" as string,
    txBytes: 0,
    rxBytes: 0,
    /** 知识库（M2） */
    knowledge: null as KnowledgeStore | null,
    defaultProfileId: "" as string,
    /** 设备组（M2 分屏） */
    deviceGroups: [] as DeviceGroup[],
  }),
  getters: {
    activeSession(state): SessionInfo | undefined {
      return state.sessions.find((s) => s.port === state.activePort);
    },
    activeFrames(state): Frame[] {
      return state.framesByPort[state.activePort] ?? [];
    },
    activeProfileId(state): string {
      return state.sessions.find((s) => s.port === state.activePort)?.profileId || state.defaultProfileId;
    },
    activeProfile(state): ModuleProfile | undefined {
      const id = state.sessions.find((s) => s.port === state.activePort)?.profileId || state.defaultProfileId;
      return state.knowledge?.profiles.find((p) => p.id === id);
    },
    activeErrorCodes(state) {
      const id = state.sessions.find((s) => s.port === state.activePort)?.profileId || state.defaultProfileId;
      return state.knowledge?.profile_error_codes?.[id] ?? state.knowledge?.error_codes ?? {};
    },
    /** 当前激活页签对应的设备组（若有） */
    activeDeviceGroup(state): DeviceGroup | undefined {
      return state.deviceGroups.find((g) => g.atPort === state.activePort);
    },
    /** 未归属任何设备组的独立会话（页签单独显示） */
    standaloneSessions(state): SessionInfo[] {
      const grouped = new Set(
        state.deviceGroups.flatMap((g) => [g.atPort, g.logPort].filter(Boolean))
      );
      return state.sessions.filter((s) => !grouped.has(s.port));
    },
  },
  actions: {
    async init() {
      if (this.listening) return;
      if (!serialInitPromise) {
        serialInitPromise = (async () => {
          const unlisteners: UnlistenFn[] = [];
          const appendFrame = (f: Frame) => {
            const bucket = (this.framesByPort[f.port] ??= []);
            bucket.push(f);
            const size = f.hex ? f.hex.replace(/\s/g, "").length / 2 : f.text.length;
            if (f.dir === "tx") this.txBytes += size;
            else this.rxBytes += size;
            if (bucket.length > MAX_FRAMES_PER_PORT) {
              bucket.splice(0, bucket.length - MAX_FRAMES_PER_PORT);
            }
          };
          try {
            unlisteners.push(await listen<Frame[]>("serial://frames", (e) => {
              for (const f of e.payload) appendFrame(f);
            }));
            unlisteners.push(await listen<Frame>("serial://frame", (e) => {
              appendFrame(e.payload);
            }));
            unlisteners.push(await listen<{ port: string; error: string }>("serial://error", (e) => {
              this.lastError = `${e.payload.port}: ${e.payload.error}`;
              const s = this.sessions.find((x) => x.port === e.payload.port);
              if (s) s.connected = false;
            }));
            await Promise.all([this.refreshPorts(), this.loadKnowledge()]);
            const open = await invoke<OpenPortInfo[]>("open_serial_ports");
            for (const p of open) {
              if (!this.sessions.some((s) => s.port === p.port)) {
                this.sessions.push({
                  port: p.port,
                  baud: p.baud,
                  generation: p.generation,
                  connected: true,
                  profileId: this.defaultProfileId,
                  firmware: "",
                });
                this.framesByPort[p.port] ??= [];
              }
            }
            if (!this.sessions.some((session) => session.port === this.activePort && session.connected)) {
              this.activePort = this.sessions.find((session) => session.connected)?.port ?? "";
            }
            const saved = await invoke<QuickCmdsFile | null>("load_quickcmds");
            if (saved) this.quickCmds = saved;
            this.listening = true;
          } catch (error) {
            for (const unlisten of unlisteners.reverse()) unlisten();
            this.listening = false;
            throw error;
          }
        })();
      }
      const currentInit = serialInitPromise;
      try {
        await currentInit;
      } catch (error) {
        if (serialInitPromise === currentInit) serialInitPromise = null;
        throw error;
      }
    },
    async refreshPorts() {
      this.ports = await invoke<PortInfo[]>("list_serial_ports");
      if (!this.selPort && this.ports.length) {
        this.selPort = this.ports[0].name;
      }
    },
    async loadKnowledge() {
      this.knowledge = await invoke<KnowledgeStore>("get_knowledge");
      if (!this.defaultProfileId && this.knowledge.profiles.length) {
        this.defaultProfileId =
          this.knowledge.profiles.find((profile) => profile.id === "generic")?.id ??
          this.knowledge.profiles.find((profile) => profile.support_level === "verified")?.id ??
          this.knowledge.profiles[0].id;
      }
    },
    async reloadKnowledge() {
      this.knowledge = await invoke<KnowledgeStore>("reload_knowledge");
      window.dispatchEvent(new Event("modulink:knowledge-capabilities-changed"));
    },
    /** 打开新会话（多路：每个端口一个页签），并异步探测模组型号 */
    async openSession(portOverride?: string) {
      const port = portOverride || this.selPort;
      if (!port) return;
      const existing = this.sessions.find((session) => session.port === port);
      if (existing?.connected) {
        this.activePort = port;
        return;
      }
      if (existing) {
        this.sessions = this.sessions.filter((session) => session.port !== port);
      }
      const opened = await invoke<OpenPortInfo>("open_serial", { port, baud: this.baud });
      this.sessions.push({
        port,
        baud: opened.baud,
        generation: opened.generation,
        connected: true,
        profileId: this.defaultProfileId,
        firmware: "",
      });
      this.framesByPort[port] ??= [];
      this.activePort = port;
      // 型号探测由调用方（UI）触发，便于展示识别结果提示
    },
    /** 型号自动识别：按画像声明的探测指令尝试，结果归属当前会话。 */
    async detectProfile(port: string): Promise<string | null> {
      const session = this.sessions.find((item) => item.port === port && item.connected);
      if (!session || !this.knowledge) return null;
      const pending = identificationRuns.get(session);
      if (pending) return pending;
      const target = this.executionTarget(port);
      session.identification = "detecting";
      session.identificationError = "";
      const isCurrent = () => this.sessions.includes(session) && session.connected
        && session.generation === target.sessionGeneration
        && identificationRuns.get(session) === task;
      // Defer execution until the request has been registered for synchronous failures too.
      const task: Promise<string | null> = Promise.resolve().then(async () => {
        try {
          const profile = await probeProfile(this.knowledge!.profiles,
            (command) => this.executeAt(target, command, "profile_detection", 2500), isCurrent);
          if (!isCurrent()) return null;
          if (!profile) { session.identification = "unmatched"; return null; }
          session.profileId = profile.id;
          session.profileSource = "detected";
          session.firmware = "";
          const firmware = await this.executeAt(target, "AT+CGMR", "profile_detection", 2500, profile.id).catch(() => undefined);
          if (!isCurrent()) return null;
          if (firmware?.final_code === "OK") {
            session.firmware = firmware.lines.map((line) => line.trim())
              .filter((line) => line && line.toUpperCase() !== "OK" && line.toUpperCase() !== "AT+CGMR").join(" / ");
          }
          session.identification = "matched";
          return profile.name;
        } catch (error) {
          if (isCurrent()) {
            session.identification = "failed";
            session.identificationError = error instanceof Error ? error.message : String(error);
          }
          return null;
        } finally {
          if (identificationRuns.get(session) === task) identificationRuns.delete(session);
        }
      });
      identificationRuns.set(session, task);
      return task;
    },
    /** 创建设备组：打开 AT 口（必选）+ 日志口（可选），绑定为一个页签 */
    async createDeviceGroup(name: string, atPort: string, atBaud: number, logPort: string, logBaud: number) {
      const openedAt = await invoke<OpenPortInfo>("open_serial", { port: atPort, baud: atBaud });
      this.sessions.push({
        port: atPort,
        baud: openedAt.baud,
        generation: openedAt.generation,
        connected: true,
        profileId: this.defaultProfileId,
        firmware: "",
      });
      this.framesByPort[atPort] ??= [];
      if (logPort) {
        try {
          const openedLog = await invoke<OpenPortInfo>("open_serial", { port: logPort, baud: logBaud });
          this.sessions.push({
            port: logPort,
            baud: openedLog.baud,
            generation: openedLog.generation,
            connected: true,
            profileId: "",
            firmware: "",
          });
          this.framesByPort[logPort] ??= [];
        } catch (e) {
          // 日志口打开失败不影响 AT 口使用
          this.lastError = `日志口 ${logPort}: ${e}`;
          logPort = "";
        }
      }
      this.deviceGroups.push({ id: atPort, name: name || atPort, atPort, logPort });
      this.activePort = atPort;
    },
    async closeDeviceGroup(id: string) {
      const group = this.deviceGroups.find((g) => g.id === id);
      if (!group) return;
      const targets = this.sessions.filter((s) => s.port === group.atPort || s.port === group.logPort)
        .map((session) => ({ session, generation: session.generation }));
      const { closeGroup } = await import("../services/sessionClosing");
      return closeGroup(this, group, targets);
    },
    async closeSession(port: string) {
      const group = this.deviceGroups.find((g) => g.atPort === port || g.logPort === port);
      if (group) return this.closeDeviceGroup(group.id);
      const session = this.sessions.find((s) => s.port === port);
      if (!session) return;
      const generation = session.generation;
      const { closeSession } = await import("../services/sessionClosing");
      return closeSession(this, session, generation);
    },
    executionTarget(port: string): ExecutionTarget {
      const session = this.sessions.find((item) => item.port === port);
      if (!session?.connected) throw new Error(`${port || "串口"} 会话未连接`);
      return { port: session.port, sessionGeneration: session.generation };
    },
    async executeAt(
      target: ExecutionTarget,
      cmd: string,
      source: ExecutionSource,
      timeoutMs = 8000,
      profileId?: string
    ): Promise<AtResult> {
      const report = await this.executeAtSequence(
        target,
        [{ cmd, timeoutMs }],
        source,
        "continue",
        profileId
      );
      return report.results[0];
    },
    async executeAtSequence(
      target: ExecutionTarget,
      steps: ExecutionStep[],
      source: ExecutionSource,
      failurePolicy: "continue" | "stop_on_error" = "continue",
      profileId?: string,
      executionId?: string
    ): Promise<ExecutionReport> {
      const sessionProfileId = this.sessions.find((session) => session.port === target.port)?.profileId;
      return invoke<ExecutionReport>("execute_at", {
        request: {
          executionId,
          target,
          source,
          steps,
          failurePolicy,
          profileId: profileId || sessionProfileId || this.defaultProfileId || null,
        },
      });
    },
    async write(data: string, isHex: boolean, appendNewline: boolean, source: ExecutionSource = "console") {
      return this.writeTo(this.executionTarget(this.activePort), data, isHex, appendNewline, source);
    },
    async writeTo(target: ExecutionTarget, data: string, isHex: boolean, appendNewline: boolean, source: ExecutionSource = "console") {
      if (!isHex && appendNewline && isAtCommand(data)) {
        const result = await this.executeAt(target, data, source);
        if (!result || result.final_code !== "OK") throw new Error(result?.final_code === "TIMEOUT" ? "指令超时；等待迟到响应结束后再发送，持续无响应时请复位设备后重连" : result?.final_code ?? "执行已取消");
        return;
      }
      await invoke("serial_write", {
        port: target.port,
        sessionGeneration: target.sessionGeneration,
        data,
        isHex,
        appendNewline,
      });
    },
    setActiveProfile(id: string) {
      const session = this.sessions.find((s) => s.port === this.activePort);
      if (session) {
        identificationRuns.delete(session);
        if (session.profileId !== id) session.firmware = "";
        session.profileId = id;
        session.profileSource = "manual";
        session.identification = undefined;
        session.identificationError = "";
      } else this.defaultProfileId = id;
    },
    clear() {
      if (this.activePort) this.framesByPort[this.activePort] = [];
    },
    async importLegacy(path: string) {
      this.quickCmds = await invoke<QuickCmdsFile>("import_legacy", { path });
      this.persistQuickCmds();
    },
    /** 防抖持久化：任何编辑后 800ms 自动落盘 */
    persistQuickCmds() {
      if (persistTimer) clearTimeout(persistTimer);
      persistTimer = setTimeout(() => {
        if (this.quickCmds) {
          invoke("save_quickcmds", { data: this.quickCmds }).catch((error) => {
            this.lastError = `快捷指令保存失败：${error}`;
          });
        }
      }, 800);
    },
    async exportLegacy(path: string, data: QuickCmdsFile) {
      await invoke("export_legacy", { path, data });
    },
  },
});
