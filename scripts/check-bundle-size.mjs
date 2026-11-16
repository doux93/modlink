import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { gzipSync } from "node:zlib";

const directory = resolve("dist");
const manifest = JSON.parse(readFileSync(resolve(directory, ".vite/manifest.json"), "utf8"));
const entry = Object.keys(manifest).find((key) => manifest[key].isEntry);
if (!entry) throw new Error("Build manifest contains no entry point");

// Count the full static import graph so splitting an eagerly loaded vendor
// file cannot hide an initial-load regression. Lazy views are measured separately.
const initial = new Set();
function collect(key) {
  if (initial.has(key)) return;
  if (!manifest[key]) throw new Error(`Missing manifest import: ${key}`);
  initial.add(key);
  for (const dependency of manifest[key].imports ?? []) collect(dependency);
}
collect(entry);
const files = [...initial].map((key) => manifest[key].file).filter((file) => file.endsWith(".js"));
const bytes = files.map((file) => readFileSync(resolve(directory, file)));
const raw = bytes.reduce((total, value) => total + value.length, 0);
const gzip = bytes.reduce((total, value) => total + gzipSync(value).length, 0);
const budget = { raw: 500_000, gzip: 160_000 };
console.log(JSON.stringify({ initialJavaScript: { files, raw, gzip }, budget }, null, 2));
if (raw > budget.raw || gzip > budget.gzip) {
  throw new Error("Initial JavaScript exceeds its size budget; inspect eager imports before increasing the budget");
}
