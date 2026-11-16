import { cp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = path.join(root, "src-tauri/target/debug");
const destination = path.join(root, "artifacts/windows-preview");
await rm(destination, { recursive: true, force: true });
await mkdir(destination, { recursive: true });
await cp(path.join(source, "modulink-studio.exe"), path.join(destination, "modulink-studio.exe"));
// Use the resources produced by Tauri, not repository fallbacks.
await cp(path.join(source, "_up_/data"), path.join(destination, "data"), { recursive: true });
if (!(await readdir(path.join(destination, "data/profiles"))).length) throw new Error("Missing module profiles");
await readFile(path.join(destination, "data/kb_index/manual_topics_public.jsonl"));

const files = {};
async function hashDirectory(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) await hashDirectory(file);
    else files[path.relative(destination, file).replaceAll("\\", "/")] = createHash("sha256").update(await readFile(file)).digest("hex");
  }
}
await hashDirectory(destination);
await writeFile(path.join(destination, "build-info.json"), JSON.stringify({ commit: process.env.GITHUB_SHA ?? "local", configuration: "debug", files }, null, 2));
await writeFile(path.join(destination, "README.txt"), `ModuLink Studio Windows 测试版\n\n完整解压本目录，再双击 modulink-studio.exe。请保留旁边的 data 文件夹。\n需要 Windows x64 和 Microsoft Edge WebView2 Runtime。\n这是未签名的 debug 验收包，不是正式安装包；尚未完成真实串口设备验收。\n用户设置、快捷指令和数据库保存在 Windows 应用数据目录，包内不含 API key。\n提交：${process.env.GITHUB_SHA ?? "local"}\n`);
console.log(JSON.stringify({ directory: destination, files: Object.keys(files).length }));
