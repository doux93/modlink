import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdir, mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
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
  if (await page.evaluate(() => localStorage.getItem("modulink-onboarding-completed") !== "1")) await closeWelcome.waitFor();
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
  await page.locator(".quick").waitFor({ state: "hidden" });
  await page.getByRole("button", { name: "展开快捷指令", exact: true }).click();
  checks.push("empty quick panel starts collapsed");
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
  await page.getByRole("button", { name: "收起快捷指令", exact: true }).click();
  await page.locator(".quick").waitFor({ state: "hidden" });
  await page.reload();
  await page.getByRole("button", { name: "展开快捷指令", exact: true }).waitFor();
  await page.getByRole("button", { name: "展开快捷指令", exact: true }).click();
  await page.locator(".quick-text").filter({ hasText: "AT+CSQ" }).waitFor();
  checks.push("quick panel collapse preference survives reload", "quick commands preserved when panel is hidden");
  const userDataDirectory = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("plugin:path|resolve_directory", { directory: 14 }));
  const quickFile = path.join(userDataDirectory, "quickcmds/current.json");
  const originalQuick = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("load_quickcmds"));
  await page.evaluate(data => window.__TAURI_INTERNALS__.invoke("save_quickcmds", { data: { ...data, name: "backup smoke" } }), originalQuick);
  await writeFile(quickFile, "{broken");
  await page.reload();
  await page.locator(".quick-save-alert").waitFor();
  await page.screenshot({ path: path.join(output, "quick-file-recovery.png"), animations: "disabled" });
  await page.getByRole("button", { name: "恢复上次备份", exact: true }).click();
  await page.locator(".quick-save-alert").waitFor({ state: "hidden" });
  assert.deepEqual(JSON.parse(await readFile(quickFile, "utf8")), originalQuick);
  checks.push("corrupt quick commands remain visible as a recoverable error", "explicit backup recovery restores previous commands");
  const configDirectory = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("plugin:path|resolve_directory", { directory: 13 }));
  const settingsFile = path.join(configDirectory, "settings.json");
  const previousSettings = await readFile(settingsFile).catch(error => { if (error.code === "ENOENT") return null; throw error; });
  try {
    await writeFile(settingsFile, "{broken");
    const outcomes = await page.evaluate(async () => {
      const failed = async (command, args) => { try { await window.__TAURI_INTERNALS__.invoke(command, args); return false; } catch { return true; } };
      return [await failed("get_settings"), await failed("save_settings", { settings: { ai: { model: "replacement" } } })];
    });
    assert.deepEqual(outcomes, [true, true]);
    assert.equal(await readFile(settingsFile, "utf8"), "{broken");
  } finally { if (previousSettings) await writeFile(settingsFile, previousSettings); else await rm(settingsFile, { force: true }); }
  checks.push("corrupt settings reject reads and writes without overwriting the original");
  await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("close_serial", { port: "MODULINK_SMOKE_ABSENT", sessionGeneration: 1 }));
  checks.push("generation-bound close is idempotent for an absent port");
  await page.getByRole("link", { name: "AI 助手", exact: true }).click();
  await page.locator(".ai-head").getByRole("button", { name: "配置", exact: true }).click();
  await page.waitForFunction(() => {
    const input = document.querySelector(".cfg-modal input");
    return input && !input.disabled;
  });
  const savedAiHeader = await page.locator(".ai-head .sub").innerText();
  await page.getByPlaceholder("Base URL，如 https://onerouter.cmaiot.cn/v1", { exact: true }).fill("https://unsaved.invalid/v1");
  await page.locator(".cfg-modal").getByRole("button", { name: "取消", exact: true }).click();
  await page.locator(".cfg-modal").waitFor({ state: "hidden" });
  assert.equal(await page.locator(".ai-head .sub").innerText(), savedAiHeader);
  checks.push("cancelled AI settings do not alter the active configuration");
  assert.equal(await page.locator("#global-error-overlay").count(), 0);
  assert.deepEqual(errors, []);
  checks.push("history drawer", "quick-command disk persistence", "disconnected send disabled", "WebView reload");
  const desktopUrl = page.url();
  await page.getByRole("link", { name: "串口控制台", exact: true }).click();
  if (!await page.locator(".quick").isVisible()) await page.getByRole("button", { name: "展开快捷指令", exact: true }).click();
  await page.getByRole("button", { name: "＋ 新增指令", exact: true }).click();
  await page.getByPlaceholder("AT 指令或数据", { exact: true }).fill("AT+CGMI");
  // Extend only the autosave debounce in this disposable test so closing must flush it.
  await page.evaluate(() => {
    const schedule = window.setTimeout.bind(window);
    window.setTimeout = (callback, milliseconds, ...args) => schedule(callback, milliseconds === 800 ? 10000 : milliseconds, ...args);
    const save = [...document.querySelectorAll(".n-modal button")].find(button => button.textContent.trim() === "保存");
    save.click();
  });
  const closed = page.waitForEvent("close");
  const closeRequest = page.evaluate(() => window.__TAURI_INTERNALS__.invoke("plugin:window|close", { label: "main" }));
  const closeResults = await Promise.allSettled([closed, closeRequest]);
  assert.equal(closeResults[0].status, "fulfilled", "The window must close after flushing commands");
  const afterClose = JSON.parse(await readFile(quickFile, "utf8"));
  assert(afterClose.groups.some(group => group.items.some(item => item.text === "AT+CGMI")));
  checks.push("native window close flushes pending quick commands before destruction");
  await writeFile(path.join(output, "result.json"), JSON.stringify({ commit: process.env.GITHUB_SHA, passed: true, checks, url: desktopUrl }, null, 2));
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
