# Knowledge Pack Contract

ModuLink Studio 的安装包内置公开手册索引，未安装外部知识包时仍可使用标题、摘要、
型号过滤和来源降级提示。完整 Markdown 正文不随应用交付，通过知识中心单独验证和激活。

## Directory Layout

```text
manuals/
  manifest.json
  topic_paths.jsonl
  ... Markdown files referenced by topic_paths.jsonl
```

新交付包使用 `modulink/knowledge-pack@3`。主题按实际正文哈希去重，并保留所有
`(repo,map_file)` owner；适用范围来自各 owner 的“适用型号”表，而不是手册标题：

```json
{
  "schema": "modulink/knowledge-pack@3",
  "id": "cmiot.manuals.ml307c",
  "version": "2026.08",
  "models": ["ML307C"],
  "generatedAt": "2026-08-04",
  "topicCount": 947,
  "includeSeries": true,
  "includeUnknownReference": true,
  "topicCounts": {
    "exact": 281,
    "series": 497,
    "unknown": 169
  }
}
```

以上计数是 `cmiot.manuals.ml307c` v2026.08 中文发布包的示例快照；每次发布应以导出器生成的
manifest 为准，不要把示例计数复制到其他型号或版本。

`models` 是获得全文授权的精确型号。`includeSeries` 表示为这些型号携带同系列参考资料，
`includeUnknownReference` 表示携带尚未解析出型号范围的参考资料。同系列资料不会反向授权未在 `models` 中
声明的兄弟型号：切换到未声明型号后，应用自动回退摘要模式。
unknown 资料可参与回答，但不能作为 AT 执行授权依据。AT 授权只查询
`command_catalog.jsonl` 中当前画像的 exact owner，不再扫描正文或接受 generic 命中。
命令目录还会编译手册正文中的型号级限制：`unsupported_models` 表示整条命令不支持，
`parameter_constraints` 表示当前型号只允许的参数值。运行时授权必须先应用这些限制，
不能因为命令或参数在通用语法中出现，就推断当前型号支持所有枚举值。

`topic_paths.jsonl` 每行映射一个公开索引主题：

```json
{"topic_id":"aa688a19d55bd302","path":"repo/topics/example.md"}
```

## Validation Rules

- schema 支持 `modulink/knowledge-pack@1`、`@2` 和 `@3`，ID 和版本不能为空。
- `@1` 为兼容的全量包，必须覆盖应用内置公开索引的全部主题。
- `@2` 的 `models` 不能为空、不能重复且必须是公开索引已收录型号；映射必须完整且只能
  覆盖“精确型号 + 可选同系列 + 可选通用”作用域。
- `@2` 的 `topicCounts` 必须与应用根据公开索引复算出的精确、系列、通用数量一致。
- `@3` 使用 owner scope 复算 exact、series、unknown；unknown 只能进入参考区。
- canonical topic 的主键是规范化正文 SHA-256。文件名 8 位后缀仅为 `sourceFragmentId`，
  不得直接用于正文去重。
- `topicCount` 必须大于 0，并等于去重后的路径映射数量。
- 映射 ID 必须与应用内置公开索引完全一致：不能缺少，也不能包含未知 ID。
- topic ID 不允许重复；路径必须是包目录内的相对 `.md` 文件。
- 绝对路径、父目录跳转、盘符路径、目录外符号链接和缺失文件都会拒绝激活。
- 验证失败不会覆盖当前激活配置；正文读取异常会清除验证缓存并回退摘要。

知识包应作为不可变发布制品使用。更新正文时生成新版本并重新激活，不要直接修改正在
使用的目录。

## Export A Model Pack

先运行一次 `manuals` 收割来更新公开索引和内部路径映射，然后按交付型号导出：

```powershell
python tools/kb_convert.py manuals `
  --root E:\documents\anythingllm_kb\manuals `
  --strict-quality
```

收割器默认只把中文主题写入 canonical 索引、命令目录和 AT 文档；英文
`Applicable Model` 主题仍参与 DITAMAP 作用域解析，但不会进入产物。只有明确需要双语产物时
才使用 `--include-english`。

```powershell
python tools/kb_convert.py export-pack `
  --root E:\documents\anythingllm_kb\manuals `
  --models ML307C `
  --output artifacts\knowledge-packs\cmiot-knowledge-ML307C-2026.08 `
  --pack-id cmiot.manuals.ml307c `
  --pack-version 2026.08 `
  --zip
```

导出器按索引精确复制 Markdown 并保持相对目录结构。任何缺失文件、不安全路径、重复 ID
或未知型号都会终止导出；输出目录必须不存在，避免覆盖已有发布物。`--zip` 额外生成
同名 ZIP 和 `.zip.sha256` 校验文件。默认包含同系列和 unknown 参考资料，可用
`--no-series` 或 `--no-unknown-reference` 显式关闭；`--no-generic` 仅作为
`--no-unknown-reference` 的旧版兼容别名保留。省略 `--pack-version` 时使用生成时的 `YYYY.MM`。

全量开发包通过 `manuals --install-map --strict-quality` 生成 `@3` manifest。同一 ditamap
内部出现互相矛盾的命令语法属于发布缺陷，会使严格构建失败；不同 ditamap 对同一型号声明
不同语法时会保留为跨 map 分歧并输出告警，等待人工裁决，不会直接阻断全量构建。

## Runtime Behavior

- `preview`：仅使用应用内置脱敏摘要，不读取外部 Markdown。
- `fulltext`：知识包完整验证通过且当前画像精确命中 `@2/@3 models`，检索结果携带外部包
  ID 和版本。
- `@2/@3` 包已激活但当前画像未覆盖：继续使用内置摘要，不读取包内同系列文件全文。
- `none`：公开索引缺失，应用不得宣称引用了手册内容。
- 当前串口会话中成功执行的 `AT+命令=?` 参数范围属于设备实测证据，优先级高于手册通用
  参数表；AI 回答和执行提议均不得超出该范围。

设备实测能力保存于应用用户数据目录的 `capability-evidence.json`，按 `profileId + 串口 +
固件版本 + 命令` 隔离。型号识别成功后应用读取 `AT+CGMR` 作为固件身份；之后成功执行的
`AT+命令=?` 响应会自动更新记录。该记录同时进入 AI 上下文和后端最终执行授权，重启后
仍然生效。能力记录不包含 FTP/MQTT 密码、业务载荷或普通 AT 交互。

构建器额外产出 `capability_restrictions.jsonl`，列出手册中所有已结构化的整命令不支持和
参数值限制，供质量审计抽查；汇总计数写入 `knowledge_quality.json`。

完整验证通过后，应用在配置目录保存一份绑定知识包路径、manifest 和 topic_paths 元数据
的验证凭据。同一不可变知识包在再次进入知识中心或应用重启后无需重新遍历数千个正文
文件；两个契约文件变化、正文读取失败或路径越界都会使凭据失效。

停用知识包只清除激活目录配置，不删除用户提供的知识包文件。
