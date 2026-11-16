import type { ExecutionTarget, ManualContentLevel } from "../types";

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

export interface ManualSearchResult {
  title?: string;
  manual?: string;
  models?: string[];
  applicability?: "exact" | "series" | "generic" | "unknown";
  content?: string;
  contentLevel?: ManualContentLevel;
  packId?: string;
  packVersion?: string;
  error?: string;
  modelRestrictions?: string[];
}

export interface AtCommandAuthorization {
  command: string;
  authorized: boolean;
  applicability?: "exact" | "generic" | "standard" | "runtime";
  manual?: string;
  topicId?: string;
  packId?: string;
  packVersion?: string;
  reasonCode: string;
  reason: string;
}
