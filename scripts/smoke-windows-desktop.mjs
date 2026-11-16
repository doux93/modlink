import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { chromium } from "playwright-core";

// This writes a test quick-command through the UI: only run on a disposable CI account.
assert.equal(process.platform, "win32", "This test requires Windows WebView2");
assert.equal(process.env.GITHUB_ACTIONS, "true", "Run only on a disposable GitHub Actions runner");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const output = path.join(root, "artifacts/desktop-smoke");
await mkdir(output, { recursive: true });
const work = await mkdtemp(path.join(os.tmpdir(), "modulink-smoke-"));
const reserve = createServer();
await new Promise((resolve) => reserve.listen(0, "127.0.0.1", resolve));
const port = reserve.address().port;
await new Promise((resolve) => reserve.close(resolve));
const child = spawn(path.join(root, "artifacts/windows-preview/modulink-studio.exe"), [], {
  cwd: work,
  env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(work, "webview") },
  stdio: ["ignore", "pipe", "pipe"],
});
let processError;
child.on("error", (error) => { processError = error; });
let log = "";
child.stdout.on("data", (chunk) => { log += chunk; });
child.stderr.on("data", (chunk) => { log += chunk; });
let browser;
let page;
const errors = [];
const checks = [];
try {
  const endpoint = `http://127.0.0.1:${port}`;
  let ready = false;
  for (let attempt = 0; attempt < 180; attempt++) {
    if (processError) throw processError;
    if (child.exitCode !== null) throw new Error(`Desktop exited: ${child.exitCode}\n${log}`);
    try { ready = (await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(500) })).ok; } catch {}
    if (ready) break;
    await delay(250);
  }
  assert(ready, `WebView2 debugging endpoint did not start\n${log}`);
  browser = await chromium.connectOverCDP(endpoint);
  const context = browser.contexts()[0];
  page = context.pages()[0] ?? await context.waitForEvent("page", { timeout: 15_000 });
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (/Failed to resolve component/.test(message.text())) errors.push(message.text());
  });
  page.setDefaultTimeout(15_000);
  await page.locator(".shell").waitFor();
  assert(!page.url().includes(":1420"), "Packaged app must not depend on Vite");
  const knowledge = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("get_knowledge"));
  assert(knowledge.profiles.length > 0, "Packaged module profiles must load through real IPC");
  checks.push("packaged knowledge loaded through IPC");
  const closeWelcome = page.getByRole("button", { name: "关闭引导", exact: true });
  if (await closeWelcome.isVisible()) await closeWelcome.click();
  const pages = [["串口控制台", ".console"], ["设备诊断", ".diag"], ["AI 助手", ".ai"], ["知识中心", ".kb-page"], ["稳定性验证", ".stress"], ["日志分析", ".log-analysis"]];
  for (const theme of ["dark", "light"]) {
    if (await page.locator("html").getAttribute("data-theme") !== theme) await page.locator(".sidebar-footer [role=switch]").click();
    for (const [name, selector] of pages) {
      if (selector === ".stress" || selector === ".log-analysis") {
        await page.getByRole("button", { name: "高级工具", exact: true }).click();
        await page.getByRole("button", { name, exact: true }).click();
      } else {
        await page.getByRole("link", { name, exact: true }).click();
      }
      await page.locator(selector).waitFor();
      await page.evaluate(() => document.fonts.ready);
      assert.equal(await page.locator("#global-error-overlay").count(), 0, `Runtime error on ${name}`);
      await page.screenshot({ path: path.join(output, `${theme}-${selector.slice(1)}.png`), animations: "disabled" });
      checks.push(`${theme}: ${name}`);
    }
  }
  assert.equal(await page.locator('nav[aria-label="主导航"] [role=link]').count(), 4);
  await page.getByRole("link", { name: "设备诊断", exact: true }).click();
  await page.locator(".diag").getByRole("button", { name: "连接设备", exact: true }).click();
  await page.locator(".connection-dialog").waitFor();
  await page.screenshot({ path: path.join(output, "connection-dialog.png"), animations: "disabled" });
  await page.locator(".connection-dialog").getByRole("button", { name: "取消", exact: true }).click();
  await page.locator(".connection-dialog").waitFor({ state: "hidden" });
  await page.getByRole("button", { name: "收起导航", exact: true }).click();
  assert(await page.locator(".shell").evaluate((el) => el.classList.contains("sidebar-collapsed")));
  await page.getByRole("button", { name: "展开导航", exact: true }).click();
  await page.getByRole("button", { name: "查看新手引导", exact: true }).click();
  await page.getByRole("button", { name: "打开串口控制台", exact: true }).click();
  await page.locator(".console").waitFor();
  checks.push("four primary destinations", "shared connection dialog from diagnosis", "explicit navigation collapse", "welcome navigates to console");
  await page.getByRole("link", { name: "执行记录", exact: true }).click();
  await page.locator(".n-drawer").waitFor();
  await page.screenshot({ path: path.join(output, "execution-history.png"), animations: "disabled" });
  await page.keyboard.press("Escape");
  await page.locator(".n-drawer").waitFor({ state: "hidden" });
  await page.getByRole("link", { name: "串口控制台", exact: true }).click();
  await page.getByRole("button", { name: "或手动新建一条", exact: true }).click();
  await page.getByPlaceholder("AT 指令或数据", { exact: true }).fill("AT+CSQ");
  await page.locator(".n-modal").getByRole("button", { name: "保存", exact: true }).click();
  await page.locator(".quick-text").filter({ hasText: "AT+CSQ" }).waitFor();
  assert(await page.locator(".quick-item").getByRole("button", { name: "发送", exact: true }).isDisabled());
  // Confirm a real disk save, then reload WebView to exercise backend restoration.
  // waitForFunction treats an async predicate's Promise as truthy even when it
  // resolves false. Await IPC in Node and retry until the debounced save lands.
  let persisted = false;
  const saveDeadline = Date.now() + 15_000;
  while (Date.now() < saveDeadline) {
    const saved = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("load_quickcmds"));
    persisted = saved?.groups?.some((group) => group.items.some((item) => item.text === "AT+CSQ")) ?? false;
    if (persisted) break;
    await delay(100);
  }
  assert(persisted, "Quick command must be saved to disk before reloading");
  await page.reload();
  await page.locator(".quick-text").filter({ hasText: "AT+CSQ" }).waitFor();
  assert.equal(await page.locator("#global-error-overlay").count(), 0);
  assert.deepEqual(errors, []);
  checks.push("history drawer", "quick-command disk persistence", "disconnected send disabled", "WebView reload");
  await writeFile(path.join(output, "result.json"), JSON.stringify({ commit: process.env.GITHUB_SHA, passed: true, checks, url: page.url() }, null, 2));
  console.log(`Desktop smoke passed: ${checks.length} checks`);
} catch (error) {
  if (page) await page.screenshot({ path: path.join(output, "failure.png") }).catch(() => {});
  await writeFile(path.join(output, "result.json"), JSON.stringify({ passed: false, checks, errors, error: String(error) }, null, 2));
  throw error;
} finally {
  await writeFile(path.join(output, "process.log"), log);
  if (browser) await browser.close().catch(() => {});
  if (child.pid && child.exitCode === null) {
    try { execFileSync("taskkill", ["/pid", String(child.pid), "/T", "/F"], { stdio: "ignore" }); } catch {}
  }
}
