import type { AtResult, ModuleProfile } from "../types";

export async function probeProfile(
  profiles: readonly ModuleProfile[],
  execute: (command: string) => Promise<AtResult>,
  isCurrent: () => boolean,
): Promise<ModuleProfile | null> {
  const rules = profiles.filter((p) => p.detect?.command && p.detect.match)
    .sort((a, b) => (b.detect?.priority ?? 0) - (a.detect?.priority ?? 0));
  let responded = false;
  for (const command of [...new Set(rules.map((p) => p.detect!.command))]) {
    if (!isCurrent()) return null;
    const result = await execute(command);
    if (!isCurrent()) return null;
    if (result.final_code !== "OK") continue;
    responded = true;
    const text = result.lines.join("\n");
    const matches = rules.filter((profile) => {
      if (profile.detect!.command !== command) return false;
      try { return new RegExp(profile.detect!.match).test(text); } catch { return false; }
    });
    if (!matches.length) continue;
    const priority = matches[0].detect!.priority ?? 0;
    const winners = matches.filter((p) => (p.detect!.priority ?? 0) === priority);
    if (winners.length > 1) throw new Error(`型号识别存在歧义：${winners.map((p) => p.name).join("、")}`);
    return winners[0];
  }
  if (rules.length && !responded) throw new Error("未收到有效的型号响应，请检查 AT 串口和波特率后重试。");
  return null;
}
