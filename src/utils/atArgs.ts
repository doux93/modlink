/**
 * AT 指令参数解析工具。
 *
 * splitAtArguments — 把 `AT+CMD=p1,"a,b",p2` 拆成 ["p1", "a,b", "p2"]。
 * 当前为最小可用版：不处理转义引号、空格 trim、Unicode 引号等。
 * 后续如需更强语义，从这里扩展；调用方不变。
 */

/**
 * 解析 AT 指令等号后的参数列表。
 * 支持：
 *  - 双引号包裹可保留内部逗号：`AT+CMD="a,b",c` → ["a,b", "c"]
 *  - 自动 trim 前后空格
 *
 * 不支持（待扩展）：
 *  - 转义双引号 `\"`
 *  - 反斜杠转义
 *  - 中文/Unicode 引号
 *
 * 如果指令没有 `=`，返回空数组。
 */
export function splitAtArguments(command: string): string[] {
  const eqIdx = command.indexOf("=");
  if (eqIdx < 0) return [];
  const source = command.slice(eqIdx + 1);
  if (!source) return [];

  const result: string[] = [];
  let current = "";
  let quoted = false;
  for (const ch of source) {
    if (ch === '"') {
      quoted = !quoted;
    } else if (ch === "," && !quoted) {
      result.push(current.trim());
      current = "";
    } else {
      current += ch;
    }
  }
  if (source) result.push(current.trim());
  return result;
}