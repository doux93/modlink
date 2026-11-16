import type { ManualContentLevel } from "../types";

const TOOLS = [
  {
    type: "function",
    function: {
      name: "propose_at_sequence",
      description: "提议一组 AT 指令序列，由用户确认后执行，执行结果会返回给你",
      parameters: {
        type: "object",
        properties: {
          purpose: { type: "string", description: "这组指令的目的（给用户看）" },
          steps: {
            type: "array",
            items: {
              type: "object",
              properties: {
                cmd: { type: "string" },
                expect: { type: "string", description: "期望响应的正则，可选" },
                delay_ms: { type: "number" },
                risk: { type: "string", enum: ["none", "reboot", "erase", "config"] },
              },
              required: ["cmd"],
            },
          },
        },
        required: ["purpose", "steps"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "lookup_error_code",
      description: "查询本地错误码知识库（+CME/+CMS ERROR 码的含义、原因、建议）",
      parameters: {
        type: "object",
        properties: {
          code_type: { type: "string", enum: ["cme", "cms"] },
          code: { type: "number" },
        },
        required: ["code_type", "code"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "get_recent_log",
      description: "获取最近的串口日志（可选正则过滤），用于你主动翻日志分析",
      parameters: {
        type: "object",
        properties: {
          lines: { type: "number", description: "行数，默认 80，最大 300" },
          filter: { type: "string", description: "正则过滤，可选" },
        },
      },
    },
  },
  {
    type: "function",
    function: {
      name: "run_diagnostic",
      description: "运行推荐检查或测试用例（会经用户确认），返回结构化结果",
      parameters: {
        type: "object",
        properties: { flow_id: { type: "string", description: "默认 network_attach" } },
      },
    },
  },
  {
    type: "function",
    function: {
      name: "search_manuals",
      description: "DYNAMIC_MANUAL_DESC",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "检索关键词，如 '5G LAN 配置' 或 'MHTTPCFG'" },
          top_k: { type: "number", description: "返回条数，默认 3" },
        },
        required: ["query"],
      },
    },
  },
];

export function toolsForMode(mode: ManualContentLevel) {
  return mode === "none"
    ? TOOLS.filter((t) => t.function.name !== "search_manuals")
    : TOOLS.map((t) => {
        if (t.function.name === "search_manuals") {
          return {
            ...t,
            function: {
              ...t.function,
              description:
                mode === "fulltext"
                  ? "在本地手册知识库中全文检索。当问题涉及功能配置流程、指令详细用法、5G/蓝牙/HTTP 等专题时使用"
                  : "在内置手册摘要知识库中检索。返回的是摘要而非全文，涉及具体参数时需提示用户核对原手册",
            },
          };
        }
        return t;
      });
}
