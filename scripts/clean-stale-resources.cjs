// Tauri 的 bundle.resources 是增量拷贝：源目录里删掉的文件不会从
// target/*/_up_/data 中移除，导致应用加载到陈旧数据（如已被转换器
// 删除的旧用例 YAML）。构建前清掉资源副本，让本次构建全新拷贝。
const fs = require("fs");
const path = require("path");

// 原始未脱敏索引已在 kb_convert.py 中输出到 tools/kb_index/，
// data/kb_index/ 仅保留脱敏后的 manual_topics_public.jsonl，无需在此删除。

for (const profile of ["release", "debug"]) {
  const dir = path.join(__dirname, "..", "src-tauri", "target", profile, "_up_", "data");
  if (fs.existsSync(dir)) {
    fs.rmSync(dir, { recursive: true, force: true });
    console.log(`[clean-stale-resources] removed ${dir}`);
  }
}
