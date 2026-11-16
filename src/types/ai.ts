import type { ExecutionTarget } from "../types";

export interface ProposeStep {
  cmd: string;
  expect?: string;
  delay_ms?: number;
  risk?: string;
  checked: boolean;
}
export interface AiExecutionTarget extends ExecutionTarget {
  profileId: string;
}
export interface UiItem {
  kind: "user" | "assistant" | "note" | "card" | "diagnostic";
  text?: string;
  card?: {
    toolCallId: string;
    purpose: string;
    steps: ProposeStep[];
    status: "pending" | "running" | "done" | "cancelled" | "failed";
    results: string[];
    source: "llm" | "rule";
    target?: AiExecutionTarget;
  };
  diagnostic?: {
    toolCallId: string;
    flowId: string;
    purpose: string;
    status: "pending" | "running" | "done" | "cancelled" | "failed";
    result: string;
    details: string[];
    target?: AiExecutionTarget;
  };
}
