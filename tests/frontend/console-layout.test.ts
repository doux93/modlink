import { describe, expect, it } from "vitest";
import { resolveConsoleLayout } from "../../src/utils/consoleLayout";

const initial = { width: 1104, hasLog: false, hasCommands: false, preference: "auto" as const, quickWidth: 280, logWidth: 340 };
describe("console workspace priorities", () => {
  it("keeps an empty panel closed and shows saved commands only when there is space", () => {
    expect(resolveConsoleLayout(initial).quickVisible).toBe(false);
    expect(resolveConsoleLayout({ ...initial, hasCommands: true }).quickVisible).toBe(true);
    expect(resolveConsoleLayout({ ...initial, width: 852, hasCommands: true, hasLog: true }).quickVisible).toBe(false);
  });
  it("temporarily floats an explicitly opened panel without squeezing the AT terminal", () => {
    const narrow = { ...initial, width: 852, hasLog: true };
    const closed = resolveConsoleLayout(narrow);
    const opened = resolveConsoleLayout({ ...narrow, preference: "open" });
    expect(opened.quickVisible).toBe(true);
    expect(opened.quickFloating).toBe(true);
    expect(opened.logWidth).toBe(closed.logWidth);
    expect(resolveConsoleLayout({ ...narrow, width: 1300, preference: "open" }).quickFloating).toBe(false);
  });
  it("honors a saved closed choice after commands arrive or the window grows", () => {
    expect(resolveConsoleLayout({ ...initial, width: 1600, hasCommands: true, preference: "closed" }).quickVisible).toBe(false);
  });
  it("reserves readable terminal space despite oversized saved panel widths", () => {
    for (const width of [744, 852, 1104, 1500]) {
      for (const hasLog of [false, true]) {
        const layout = resolveConsoleLayout({ ...initial, width, hasLog, preference: "open", quickWidth: 720, logWidth: 720 });
        const occupied = (hasLog ? layout.logWidth + 5 : 0) + (layout.quickFloating ? 0 : layout.quickWidth + 5);
        expect(width - 16 - occupied).toBeGreaterThanOrEqual(440);
      }
    }
  });
});
