# ModuLink Studio 架构约定

## 分层

```text
Vue views/components
  -> Pinia session store
  -> Tauri application commands/events
     -> SerialManager / diagnostic runner / knowledge loader / AI proxy
     -> modulink-core (protocol, diagnostics, import/export)
```

`modulink-core` 不依赖 Tauri 或 Vue。串口、网络和文件系统属于应用适配层，核心库只接收抽象执行器和数据模型。

## 会话模型

- 每个物理端口对应一个后端 `Session`，同一端口的所有写入都经过命令锁。
- 诊断和 AI 序列会持有端口独占锁，避免控制台写入插入命令序列。
- 画像归属于前端 `SessionInfo.profileId`，不能使用全局画像解释多个设备。
- 诊断事件必须携带 `run_id`、端口和流程 id；取消是协作式的，在当前 AT 命令结束后生效。

## 数据目录

- `data/` 是打包的只读知识资源：profiles、diagnostics、error_codes、手册索引。
- Tauri `app_config_dir` 保存设置；Tauri `app_data_dir` 保存快捷指令和用户数据。
- API key 使用 Windows Credential Manager，不写入 JSON。
- 首次启动会迁移旧版 exe 旁 `data/settings.json` 与 `data/quickcmds/current.json`。
- 发布包内置脱敏手册摘要索引，不依赖开发机本地 KB；完整正文通过知识中心激活外部
  知识包。当前交付格式 `modulink/knowledge-pack@3` 按正文哈希去重、保留 ditamap owner
  scope，并区分 exact、series 与 unknown 参考资料；运行时只为 manifest 声明的精确型号
  开放全文。加载器继续兼容 `@1` 全量包和 `@2` 型号包；契约和生成方式见 `knowledge-pack.md`。

## 事件和日志

串口读线程按小批量发送 `serial://frames` 事件，前端只负责显示和过滤；后端保留有界历史并提供导出命令。需要长期采集时应继续扩展为后端文件捕获，而不是无限增长 Vue 数组。

## 数据校验

知识库加载按文件名排序，并检查重复 id、重复型号、正则、诊断条件、结果值和画像引用。单画像使用 `modulink/profile@1`；同一套公共规则下的大批型号使用 `modulink/profile-catalog@1`，加载时将 `defaults` 与每个型号条目合并。目录可用 `detect` 根据 `model_codes` 统一生成“精确型号 + 可选连字符后缀”的识别规则；若目录中已有正式的连字符子型号，基础型号不会再接受任意后缀，避免多画像同时命中。识别优先级数值越大越先匹配，表示规则具体程度，不表示画像可信度。`verified` 表示内置行为已实机验证，`reference` 表示已有精确型号离线资料但尚未完成实机验证。加载告警保存在 `KnowledgeStore.warnings`，热重载会同时清空手册索引缓存。
