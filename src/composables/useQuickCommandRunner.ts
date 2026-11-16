import { ref } from "vue";
import type { ExecutionReport, ExecutionStep, ExecutionTarget, QuickCommand } from "../types";

import { isAtCommand } from "../utils/atCommand";

interface QuickCommandExecutor {
  execute(target: ExecutionTarget, steps: ExecutionStep[], id: string): Promise<ExecutionReport>;
  write(target: ExecutionTarget, command: QuickCommand): Promise<void>;
  stop(id: string): Promise<unknown>;
}

/** Owns a fixed target and command snapshot for the lifetime of a batch. */
export function useQuickCommandRunner(executor: QuickCommandExecutor) {
  const looping = ref(false);
  const runningTarget = ref<ExecutionTarget>();
  let controller: AbortController | undefined;
  let executionId: string | undefined;

  async function stop() {
    controller?.abort();
    if (executionId) await executor.stop(executionId);
  }

  async function run(target: ExecutionTarget, selected: QuickCommand[], times: number, intervalMs: number) {
    if (looping.value) return;
    const snapshot = { ...target };
    const commands = selected.map((command) => ({ ...command }));
    const abort = new AbortController();
    controller = abort;
    looping.value = true;
    runningTarget.value = snapshot;
    try {
      for (let round = 0; round < times && !abort.signal.aborted; round++) {
        for (let index = 0; index < commands.length && !abort.signal.aborted;) {
          const steps: ExecutionStep[] = [];
          while (index < commands.length && steps.length < 100) {
            const command = commands[index];
            if (command.format === "hex" || !command.append_newline || !isAtCommand(command.text)) break;
            steps.push({ cmd: command.text, delayMs: command.delay_ms ?? intervalMs });
            index++;
          }
          if (steps.length) {
            const id = crypto.randomUUID();
            executionId = id;
            // The backend reserves this ID before queueing. Recheck after start
            // events in the view so an immediate stop cannot be lost.
            const report = await executor.execute(snapshot, steps, id);
            executionId = undefined;
            if (report.cancelled) return;
            const failed = report.results.find((result) => result.final_code !== "OK");
            if (report.stoppedOnFailure || failed) throw new Error(failed?.final_code ?? "序列已中止");
          } else {
            const command = commands[index++];
            await executor.write(snapshot, command);
            await delay(command.delay_ms ?? intervalMs, abort.signal);
          }
        }
      }
    } finally {
      executionId = undefined;
      controller = undefined;
      looping.value = false;
      runningTarget.value = undefined;
    }
  }

  function onExecutionStarted(id: string) {
    if (id === executionId && controller?.signal.aborted) void executor.stop(id);
  }

  return { looping, runningTarget, run, stop, onExecutionStarted };
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.resolve();
  return new Promise((resolve) => {
    const done = () => { clearTimeout(timer); signal.removeEventListener("abort", done); resolve(); };
    const timer = setTimeout(done, Math.max(0, Math.min(ms, 60_000)));
    signal.addEventListener("abort", done, { once: true });
  });
}
