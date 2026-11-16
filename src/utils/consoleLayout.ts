export type QuickPanelPreference = "auto" | "open" | "closed";

/** Layout policy in workspace pixels; resizing never changes a saved preference. */
export function resolveConsoleLayout(input: {
  width: number; hasLog: boolean; hasCommands: boolean;
  preference: QuickPanelPreference; quickWidth: number; logWidth: number;
}) {
  const available = Math.max(0, input.width - 16); // body padding
  const mainMin = 440;
  const logMin = 260;
  const quickMin = 260;
  const split = 5;
  const inlineSpace = available >= mainMin + quickMin + split + (input.hasLog ? logMin + split : 0);
  const quickVisible = input.preference === "open"
    || (input.preference === "auto" && input.hasCommands && inlineSpace);
  const quickFloating = quickVisible && !inlineSpace;
  const quickWidth = quickFloating ? Math.min(340, Math.max(200, available - 32))
    : Math.min(input.quickWidth, Math.max(quickMin, available - mainMin - split - (input.hasLog ? logMin + split : 0)));
  const inlineQuick = quickVisible && !quickFloating ? quickWidth + split : 0;
  const logWidth = Math.min(input.logWidth, Math.max(200, available - mainMin - split - inlineQuick));
  return { quickVisible, quickFloating, quickWidth, logWidth };
}
