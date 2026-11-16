import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import type {
  AtResult,
  ExecutionKind,
  ExecutionLifecycleEvent,
  ExecutionRisk,
  ExecutionSource,
  ExecutionStatus,
  ExecutionStep,
  ExecutionSummary,
  ExecutionTarget,
} from "../types";

/// 后端 SQLite 返回的执行记录结构（与 db.rs ExecutionRecord 的 camelCase 序列化一致）。
interface ExecutionRecord {
  id: string;
  sessionId?: string | null;
  projectId?: string | null;
  source: string;
  kind: string;
  status: string;
  startedAt: number;
  finishedAt: number;
  elapsedMs: number;
  stepCount: number;
  port?: string | null;
  summary?: ExecutionSummary | null;
  results?: AtResult[] | null;
  resultsTruncated?: boolean;
  error?: string | null;
}

/// 后端扁平记录 → 前端结构化生命周期事件。sessionGeneration 无法从库还原，用 0 占位。
function fromRecord(rec: ExecutionRecord): ExecutionLifecycleEvent {
  const target: ExecutionTarget = {
    port: rec.port ?? "",
    sessionGeneration: 0,
  };
  return {
    executionId: rec.id,
    kind: rec.kind as ExecutionKind,
    target,
    source: rec.source as ExecutionSource,
    risk: "unknown" as ExecutionRisk,
    status: rec.status as ExecutionStatus,
    stepCount: rec.stepCount,
    startedAt: rec.startedAt,
    elapsedMs: rec.elapsedMs,
    error: rec.error ?? undefined,
    summary: rec.summary ?? undefined,
    results: rec.results ?? undefined,
    resultsTruncated: rec.resultsTruncated,
  };
}

const initializations = new WeakMap<object, Promise<void>>();

export const useExecutionStore = defineStore("execution", {
  state: () => ({
    records: [] as ExecutionLifecycleEvent[],
    listening: false,
    /** 失败记录跳转 AI 预填的问题文本 */
    pendingAiPrompt: null as string | null,
    /** 失败记录跳转日志分析时指定的端口 */
    pendingLogPort: null as string | null,
    persistenceError: "",
  }),
  getters: {
    runningCount(state): number {
      return state.records.filter((record) => record.status === "running").length;
    },
  },
  actions: {
    async init() {
      const previous = initializations.get(this);
      if (previous) return previous;
      const initialize = (async () => {
        const unlisteners: Array<() => void> = [];
        try {
          unlisteners.push(await listen<string>("execution://persistence-error", (event) => {
            this.persistenceError = event.payload;
          }));
          unlisteners.push(await listen<ExecutionLifecycleEvent>("execution://lifecycle", (event) => {
            const incoming = event.payload;
            const index = this.records.findIndex(
              (record) => record.executionId === incoming.executionId
            );
            if (index >= 0) {
              this.records[index] = incoming;
            } else {
              this.records.unshift(incoming);
            }
          }));
          this.listening = true;
          try {
            const remote = await invoke<ExecutionRecord[]>("list_executions", { limit: 200 });
            const liveIds = new Set(this.records.map((record) => record.executionId));
            this.records = [...this.records, ...remote.filter((record) => !liveIds.has(record.id)).map(fromRecord)]
              .sort((a, b) => b.startedAt - a.startedAt);
          } catch (error) {
            this.persistenceError = `加载执行历史失败：${error}`;
          }
        } catch (error) {
          unlisteners.forEach((unlisten) => unlisten());
          this.listening = false;
          console.error("Failed to listen for execution lifecycle events", error);
        }
      })();
      initializations.set(this, initialize);
      await initialize;
      if (!this.listening) initializations.delete(this);
    },
    clearFinished() {
      this.records = this.records.filter((record) => record.status === "running");
    },
    /** 统一停止运行中的任务（诊断/序列/压测） */
    async stop(executionId: string): Promise<boolean> {
      return invoke<boolean>("stop_execution", { executionId });
    },
    /** 重新执行已完成任务，绑定当前会话代理 */
    async rerun(
      record: ExecutionLifecycleEvent,
      target: ExecutionTarget,
      executeSequence: (target: ExecutionTarget, steps: ExecutionStep[], source: ExecutionSource) => Promise<void>
    ): Promise<void> {
      const results = record.results;
      if (!results || results.length === 0) return;
      const steps: ExecutionStep[] = results.map((r: AtResult) => ({
        cmd: r.request,
        timeoutMs: 8000,
      }));
      // 释放统一标志为控制台来源，避免历史来源（ai/profile_detection 等）造成归属混淆
      await executeSequence(target, steps, "console");
    },
    /** 导出单次执行报告为文本文件，返回是否实际写入 */
    async exportReport(record: ExecutionLifecycleEvent): Promise<boolean> {
      const lines: string[] = [];
      lines.push("ModuLink Studio - 执行报告");
      lines.push("=".repeat(48));
      lines.push(`执行 ID: ${record.executionId}`);
      lines.push(`类型: ${record.kind}`);
      lines.push(`来源: ${record.source}`);
      lines.push(`端口: ${record.target.port}  会话 #${record.target.sessionGeneration}`);
      lines.push(`风险: ${record.risk}`);
      lines.push(`状态: ${record.status}`);
      lines.push(`开始: ${new Date(record.startedAt).toLocaleString("zh-CN")}`);
      lines.push(`耗时: ${record.elapsedMs} ms`);
      if (record.error) lines.push(`错误: ${record.error}`);
      if (record.summary) {
        const s = record.summary;
        lines.push(`计划: ${s.planned}  完成: ${s.completed}  通过: ${s.passed}  失败: ${s.failed}`);
        lines.push(`取消: ${s.cancelled}  失败中止: ${s.stoppedOnFailure}`);
      }
      lines.push("");
      const results = record.results;
      if (results && results.length > 0) {
        lines.push(`步骤明细 (${results.length} 条):`);
        lines.push("-".repeat(48));
        results.forEach((r: AtResult, i: number) => {
          lines.push(`[${i + 1}] 指令: ${r.request}`);
          lines.push(`    终结码: ${r.final_code}  耗时: ${r.elapsed_ms} ms`);
          if (r.lines.length > 0) {
            lines.push(`    响应:`);
            r.lines.forEach((l: string) => lines.push(`      ${l}`));
          }
          lines.push("");
        });
      } else {
        lines.push("(无步骤明细——诊断/压测的详细报告请查看对应页面)");
      }
      const content = lines.join("\n");
      const path = await saveDialog({
        defaultPath: `${record.executionId}.txt`,
        filters: [{ name: "文本文件", extensions: ["txt"] }],
      });
      if (!path) return false;
      await invoke("save_text_file", { path, content });
      return true;
    },
    /** 设置跳转 AI 的预填问题 */
    requestAiAnalysis(record: ExecutionLifecycleEvent) {
      const results = record.results;
      const detail = results && results.length > 0
        ? results.map((r: AtResult) => `> ${r.request}\n< ${r.lines.join("; ")} [${r.final_code}]`).join("\n")
        : record.error ?? "(无详细信息)";
      this.pendingAiPrompt = `请分析以下失败的 AT 执行记录，帮助定位原因?\n\n${detail}`;
    },
    /** 设置跳转日志分析的端口 */
    requestLogView(port: string) {
      this.pendingLogPort = port;
    },
    consumeAiPrompt(): string | null {
      const v = this.pendingAiPrompt;
      this.pendingAiPrompt = null;
      return v;
    },
    consumeLogPort(): string | null {
      const v = this.pendingLogPort;
      this.pendingLogPort = null;
      return v;
    },
  },
});
