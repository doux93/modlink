import { applicableAtCommands } from "../knowledge";
import { splitAtArguments } from "../utils/atArgs";
import type { AtCommandDoc, Frame, KnowledgeCapabilities, KnowledgeStore, ManualContentLevel, ModuleProfile } from "../types";
import type { DeviceSession } from "../types/device";
import type { ManualSearchResult } from "../types/ai";

export interface AiEvidenceContext {
  profile?: ModuleProfile;
  session?: DeviceSession;
  knowledge: KnowledgeStore | null;
  frames: readonly Frame[];
  capabilities: KnowledgeCapabilities | null;
  persistedEvidence: string;
}

// Evidence selection and command checks contain no Vue, IPC, or conversation mutations.
export function createAiEvidence(getContext: () => AiEvidenceContext) {
const HIGH_RISK = /CFUN\s*=\s*1\s*,\s*1|CFUN\s*=\s*0|\+MRST|\+CRST|FACTORY|FORMAT|QPRTPARA/i;
function recentLog(lines: number, filter?: string): string {
  let fs = getContext().frames;
  if (filter) {
    try {
      const re = new RegExp(filter, "i");
      fs = fs.filter((f) => re.test(f.text));
    } catch {
      fs = fs.filter((f) => f.text.toLowerCase().includes(filter.toLowerCase()));
    }
  }
  return fs
    .slice(-lines)
    .map((f) => `${f.dir === "tx" ? ">>" : "<<"} ${f.hex ?? f.text}`)
    .join("\n");
}

/** 轻量检索：从画像 + 知识库批量文档里挑与问题/日志相关的指令文档 */
function retrieveDocs(question: string): string {
  const kb = [
    ...(getContext().profile?.at_commands ?? []).map((command) => ({
      command,
      applicability: "profile" as const,
    })),
    ...applicableAtCommands(getContext().knowledge, getContext().profile),
  ];
  const q = question.toUpperCase();
  const log = recentLog(40).toUpperCase();
  const inventoryIntent = /(?:哪些|所有|完整|清单|一览|汇总).*(?:指令|命令)|(?:指令|命令).*(?:哪些|所有|完整|清单|一览|汇总)/i.test(question);
  const protocol = ["HTTPS", "HTTP", "MQTT", "FTP", "FOTA", "TCP", "UDP", "SSL", "GNSS", "NTP"]
    .find((candidate) => q.includes(candidate));
  if (inventoryIntent && protocol) {
    const rank = (applicability: string) =>
      applicability === "profile" || applicability === "exact" ? 3 : applicability === "generic" ? 2 : 1;
    const commands = new Map<string, { command: AtCommandDoc; applicability: string }>();
    for (const item of kb) {
      if (item.applicability === "series") continue;
      const commandName = item.command.cmd.toUpperCase();
      const category = item.command.category?.toUpperCase() ?? "";
      if (!commandName.includes(protocol) && !category.includes(protocol)) continue;
      const existing = commands.get(commandName);
      if (!existing || rank(item.applicability) > rank(existing.applicability)) {
        commands.set(commandName, item);
      }
    }
    if (commands.size) {
      const CATALOG_LIMIT = 30;
      const sorted = [...commands.values()].sort((left, right) =>
        left.command.cmd.localeCompare(right.command.cmd)
      );
      const lines = sorted
        .slice(0, CATALOG_LIMIT)
        .map(({ command, applicability }) =>
          `- ${command.cmd} | 适用性: ${applicability === "profile" ? "exact" : applicability} | ${command.title ?? ""}`
        );
      // 证据块只放数据：怎么用这份目录由系统提示词的规则约束，别混进来。
      const header = sorted.length > CATALOG_LIMIT
        ? `### ${protocol} AT 命令目录（共 ${sorted.length} 条，按名称排序显示前 ${CATALOG_LIMIT} 条）`
        : `### ${protocol} AT 命令目录（共 ${sorted.length} 条，已完整列出）`;
      return `${header}\n${lines.join("\n")}`;
    }
  }
  const scored = kb
    .map(({ command: c, applicability }) => {
      let score = 0;
      const name = c.cmd.toUpperCase().replace(/^AT\+?/, "");
      if (name && q.includes(name)) score += 5;
      if (name && log.includes(name)) score += 3;
      for (const w of q.split(/[^A-Z0-9]+/)) {
        if (w.length >= 4 && (c.doc?.toUpperCase().includes(w) || c.title?.toUpperCase().includes(w))) {
          score += 1;
        }
      }
      if (score > 0) {
        score +=
          applicability === "profile" || applicability === "exact"
            ? 4
            : applicability === "series"
              ? 2
              : 0;
      }
      return { c, applicability, score };
    })
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, 3);
  if (!scored.length) return "";
  return scored
    .map(
      ({ c, applicability }) =>
        `### ${c.cmd} ${c.title ?? ""}\n适用性: ${applicability}\n语法: ${c.syntax}\n${(c.doc ?? "").slice(0, 800)}`
    )
    .join("\n\n");
}

function activeManualMode(): ManualContentLevel {
  const capabilities = getContext().capabilities;
  if (capabilities?.effectiveManualMode) {
    return capabilities.effectiveManualMode;
  }
  const mode = getContext().capabilities?.manualMode ?? "none";
  if (mode !== "fulltext") return mode;
  const coverage = getContext().capabilities?.fulltextModelCoverage;
  // null/undefined 表示兼容 @1 全量包；@2 必须精确命中声明型号。
  if (coverage == null) return "fulltext";
  const activeModels = getContext().profile?.model_codes ?? [];
  return activeModels.some((model) =>
    coverage.some((covered) => covered.toUpperCase() === model.toUpperCase())
  )
    ? "fulltext"
    : "preview";
}


function evidenceScopeLabel(result: ManualSearchResult): string {
  if (result.applicability === "exact") return "exact（当前精确型号资料）";
  if (result.applicability === "series") return "series（同系列参考资料）";
  if (result.applicability === "unknown") return "unknown（参考资料，不可用于执行授权）";
  if (result.applicability === "generic") {
    return result.contentLevel === "fulltext" && result.packId !== "cmiot.core.builtin"
      ? "generic（当前型号包明确包含的通用资料）"
      : "generic（通用资料）";
  }
  return "unknown（参考资料，不可用于执行授权）";
}


function levenshtein(a: string, b: string): number {
  const previous = Array.from({ length: b.length + 1 }, (_, index) => index);
  for (let i = 1; i <= a.length; i++) {
    let diagonal = previous[0];
    previous[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const above = previous[j];
      previous[j] = Math.min(
        previous[j] + 1,
        previous[j - 1] + 1,
        diagonal + (a[i - 1] === b[j - 1] ? 0 : 1)
      );
      diagonal = above;
    }
  }
  return previous[b.length];
}

function modelCorrectionContext(question: string): string {
  const known = getContext().capabilities?.modelCoverage ?? [];
  const knownSet = new Set(known.map((model) => model.toUpperCase()));
  const mentioned = question.toUpperCase().match(/\b(?:ML|MN|MR)\d{3,4}(?:-[A-Z0-9]+|[A-Z])?\b/g) ?? [];
  const corrections = [...new Set(mentioned)]
    .filter((model) => !knownSet.has(model))
    .map((model) => {
      const nearest = known
        .map((candidate) => ({ candidate, distance: levenshtein(model, candidate.toUpperCase()) }))
        .sort((left, right) => left.distance - right.distance)[0];
      return nearest && nearest.distance <= 2
        ? `用户提到的 ${model} 不在型号目录中，最接近 ${nearest.candidate}；必须先纠正并请用户确认。`
        : `用户提到的 ${model} 不在型号目录中；必须明确提示无法识别，不能假设该型号存在。`;
    });
  return corrections.join("\n");
}

function manualEvidenceContext(evidence: ManualSearchResult[]): string {
  if (!evidence.length) return "（自动检索未命中，不得据此编造完整流程或参数。）";
  return evidence
    .map((result, index) =>
      `### [${index + 1}] ${result.title || "未命名章节"}\n` +
      `手册: ${result.manual || "-"} | 适用性: ${evidenceScopeLabel(result)} | 内容级别: ${result.contentLevel || "-"} | 知识包: ${result.packId || "-"}@${result.packVersion || "-"}\n` +
      `${result.modelRestrictions?.length ? `当前型号限制（必须优先遵守）:\n${result.modelRestrictions.map((note) => `- ${note}`).join("\n")}\n` : ""}` +
      `${(result.content || "").slice(0, 4000)}`
    )
    .join("\n\n");
}

function runtimeCapabilityContext(): string {
  const latest = new Map<string, { query: string; responses: string[] }>();
  let current: { name: string; query: string; responses: string[] } | null = null;
  for (const frame of getContext().frames) {
    const text = frame.text.trim();
    if (frame.dir === "tx") {
      const match = text.match(/^(AT(?:\+[A-Z0-9]+|[A-Z0-9]*))=\?$/i);
      current = match ? { name: match[1].toUpperCase(), query: text, responses: [] } : null;
      if (current) latest.set(current.name, current);
    } else if (current && text && text.toUpperCase() !== current.query.toUpperCase()) {
      current.responses.push(text);
    }
  }
  if (!latest.size) return "（当前会话没有参数能力查询记录。）";
  return [...latest.values()]
    .slice(-10)
    .map((probe) => `>> ${probe.query}\n${probe.responses.slice(0, 16).map((line) => `<< ${line}`).join("\n")}`)
    .join("\n\n");
}

function systemPrompt(question: string, manualEvidence: ManualSearchResult[]): string {
  const profile = getContext().profile;
  const session = getContext().session;
  const conn = session?.connected
    ? `已连接 ${session.port} @ ${session.baud}`
    : "串口未连接";
  const docs = retrieveDocs(question);
  const mode = activeManualMode();
  const modelCorrection = modelCorrectionContext(question);

  const manualRule = mode === "fulltext"
    ? "6. 涉及功能配置流程、指令详细用法、专题功能（5G/蓝牙/HTTP/FOTA 等）时，先调 search_manuals 检索官方手册再回答，可以表述“根据手册正文”，并注明出处手册名和章节。"
    : mode === "preview"
      ? "6. 涉及功能配置流程、指令详细用法时，可调 search_manuals 检索内置摘要。但摘要可能不完整，涉及具体参数和完整步骤时必须提示用户核对原手册，不得声称已根据手册全文。"
      : "6. 当前未安装手册知识包，不得调用 search_manuals，也不得宣称有任何手册依据。涉及手册内容时建议用户查阅官方文档。";

  const modeLabel = mode === "fulltext" ? "全文手册" : mode === "preview" ? "摘要手册" : "无手册";
  const coreStatus = getContext().capabilities?.coreAvailable ? "已就绪" : "未就绪";

  return `你是蜂窝通信模组 AT 调试专家助手，嵌入在 ModuLink Studio 调试工具中，帮助用户（嵌入式工程师/FAE/测试）排查模组问题。

规则：
1. 需要执行 AT 指令时，必须调用 propose_at_sequence 工具提议，绝不让用户手敲，也绝不假设已执行；工具会经用户确认后执行并把结果返回给你。
2. 重启/擦除/恢复出厂类指令必须在 steps 里标 risk 字段（reboot/erase）。
3. 不得虚构 AT 指令；不确定的指令先说明依据或建议查询手册。
4. 回答用中文，简洁、结论先行；错误码优先调 lookup_error_code。
5. 需要更多日志时调 get_recent_log；怀疑入网问题时可提议 run_diagnostic 跑"入网连通性推荐检查"。
${manualRule}
7. 证据适用性必须准确表达：exact 是当前精确型号资料；series 只代表同系列参考，不得声称已在当前型号验证；unknown 是适用范围未知的参考资料，不得用于 AT 执行授权；generic 若来自当前已验证型号包的全文，是该型号包明确携带的通用适用资料，可以据此回答和提议指令，但不得描述成当前型号专属或实测资料。禁止把这种 generic 全文误报为“知识库缺少当前型号依据”。
8. AI 提议 AT 指令前，应结合内置 AT 卡片校验指令格式和风险，不能只凭手册摘要生成。上下文若给出"AT 命令目录"，它就是当前型号的权威命令清单：清单类问题以它为准，不得把目录中已有的命令说成未检索到，也不得跨命令族混用。
9. 下方“自动检索手册证据”是本轮问题的优先依据。涉及完整流程时必须按证据回答；未命中时明确说明，禁止用常识补齐指令。某次 AT 提议被策略拒绝，只表示所提议的具体命令缺少依据，不代表整个功能没有手册；此时应重新检索协议专用手册并改用正文中实际出现的命令，禁止宣称该功能“无手册资料”。
10. 型号目录校验提示必须执行：未知或疑似拼错的型号先纠正并确认，不能静默接受。
11. “命令存在”不等于“该型号支持该命令的全部参数值”。当前型号限制说明必须高于通用参数表；串口 AT+命令=? 的当前模组实测范围优先级最高。若实测只返回 0，不得建议值 1/2，也不得据参数名推断功能受支持。

## 当前环境
模组画像: ${profile?.name ?? "未选择"} (model: ${profile?.model_codes?.join(", ") || "-"}; series: ${profile?.series || "-"}; support: ${profile?.support_level || "-"})
连接状态: ${conn}
知识能力: 核心包${coreStatus} | 手册: ${modeLabel}
${modelCorrection ? `型号目录校验:\n${modelCorrection}` : ""}
${docs ? `\n## 相关手册摘录\n${docs}` : ""}

## 自动检索手册证据
${manualEvidenceContext(manualEvidence)}

## 当前模组参数能力实测（优先级高于手册通用范围）
${getContext().persistedEvidence}

## 当前会话参数查询（尚未持久化的即时证据）
${runtimeCapabilityContext()}`;
}


function localRisk(cmd: string): string | undefined {
  if (HIGH_RISK.test(cmd)) return "reboot";
  const body = cmd.trim().toUpperCase().split(/[=?]/)[0];
  const docs = [
    ...(getContext().profile?.at_commands ?? []),
    ...applicableAtCommands(getContext().knowledge, getContext().profile).map((item) => item.command),
  ];
  const risk = docs.find((doc) => doc.cmd.toUpperCase() === body)?.risk;
  return risk && risk !== "none" ? risk : undefined;
}


function runtimeProbeDenial(command: string): string | undefined {
  const name = command.trim().toUpperCase().match(/^(AT(?:\+[A-Z0-9]+|[A-Z0-9]*))/)?.[1];
  if (!name || command.trim().endsWith("=?")) return undefined;
  let response: string[] = [];
  let collecting = false;
  for (const frame of getContext().frames) {
    const text = frame.text.trim();
    if (frame.dir === "tx") {
      collecting = text.toUpperCase() === `${name}=?`;
      if (collecting) response = [];
    } else if (collecting && text.toUpperCase() !== `${name}=?`) {
      response.push(text);
    }
  }
  if (!response.length) return undefined;
  const args = splitAtArguments(command);
  for (let index = 0; index + 1 < args.length; index++) {
    const selector = args[index].replace(/^"|"$/g, "");
    if (!/^[a-z_][a-z0-9_]*$/i.test(selector)) continue;
    const proposed = Number(args[index + 1]);
    if (!Number.isInteger(proposed)) continue;
    const line = response.find((item) =>
      new RegExp(`["']${selector}["']`, "i").test(item)
    );
    if (!line) continue;
    const tail = line.slice(line.search(new RegExp(`["']${selector}["']`, "i")) + selector.length + 2);
    const range = tail.match(/\(?\s*(-?\d+)\s*(?:-\s*(-?\d+))?\s*\)?/);
    if (!range) continue;
    const minimum = Number(range[1]);
    const maximum = range[2] == null ? minimum : Number(range[2]);
    if (proposed < minimum || proposed > maximum) {
      return `当前模组 ${name}=? 实测显示参数 ${selector} 仅支持 ${minimum === maximum ? minimum : `${minimum}-${maximum}`}，已拒绝值 ${proposed}`;
    }
  }
  return undefined;
}

  return { recentLog, activeManualMode, systemPrompt, localRisk, runtimeProbeDenial };
}
