# 第三轮：AI 网络取消与 Windows 桌面验收

## AI 请求生命周期

此前界面取消只是结束前端等待，Rust 中的 HTTP 请求仍可能继续运行至 90 秒。
现在取消、30 秒总预算耗尽、清空对话、设备上下文变化和组件卸载都会中止对应请求。

- 每轮 HTTP 调用使用独立 UUID，先注册，再发送。若取消发生在注册过程中，注册返回后
  立即释放，不发送 HTTP；避免“取消先到、请求后到”的竞态。
- 前端 `AbortController` 与 Rust `AiRequests` 配合。后端取消会 drop 包括读取响应体在内的
  整个 reqwest future；客户端不再等待或接收结果。服务端已开始的模型计算能否停止取决于服务端。
- 后端 timeout 使用前端剩余总预算，并限制为最多 30 秒，不再另开 90 秒等待窗口。
- Rust guard 在成功、失败、超时及 future 被丢弃时统一回收注册；重复 claim 被拒绝。
  旧请求清理不影响同 ID 的新注册。最多保留 32 个注册，未启动项在 60 秒后过期，后续注册时清理。
- `ai_chat` IPC 新增 `requestId` / `timeoutMs`，并增加 `prepare_ai_request`、`cancel_ai_request`。
  前后端需成套更新。取消模型请求不撤回已经发送的串口字节。

## 回归检查

- 前端新增 10 项检查：注册竞态、超时、请求隔离、失败清理、剩余预算，以及真实 Vue 页面上的
  取消、清空、切换设备与卸载。原有测试继续保留，总计 31 项。
- Rust 独立测试 crate 直接引用生产请求管理器，覆盖 6 项生命周期行为，不需要凭据或 WebView。
- Windows 桌面测试增加 2 项本地 HTTP 连接检查：服务端不返回头部、返回不完整响应体时取消，
  确认连接关闭。仅使用临时回环端口和测试数据，不调用外部 AI 服务。

## Windows 验收包与实际 WebView2 检查

CI 在原有检查之后执行 `tauri build --debug --no-bundle`，生成包含前端资源的测试 exe。
`package-windows-preview.mjs` 将 exe 与 Tauri 实际生成的知识数据复制到独立目录，
写入提交标识及逐文件 SHA-256。包内不包含本地凭据或用户数据库。

`smoke-windows-desktop.mjs` 仅允许在一次性的 Windows GitHub Actions 账户运行：

1. 从仓库以外的工作目录启动验收包，确认不依赖 Vite 开发服务器或仓库数据回退路径。
2. 通过 WebView2 的 CDP 接口连接实际 Tauri 窗口，检查六个页面的深浅主题，共 12 张截图。
3. 检查执行历史抽屉、新建快捷指令、真实磁盘保存、页面重载恢复，以及未连接设备时禁用发送。
4. 任何页面异常浮层或组件解析失败都令 CI 失败，并保留失败截图和进程日志。

CDP 环境变量只传给 CI 启动的子进程；普通启动不会开放该调试端口。
实现参考 [Playwright 的 WebView2 文档](https://playwright.dev/docs/webview2)。
GitHub Windows runner 默认以管理员运行；WebView2 150 起忽略这种进程的环境变量调试参数。
验收步骤按 [Tauri 官方方案](https://github.com/tauri-apps/wry/issues/1782) 使用 gsudo 降为
普通用户权限运行，仅给 runner 当前用户补充工作目录权限，不改应用或系统的 WebView2 安全策略。

Actions 产物保留 14 天：`modulink-windows-preview` 是 debug 便携验收包，
`windows-desktop-smoke` 是截图、进程日志和 `result.json`。
产物下载入口在对应成功运行的 Artifacts 区域。没有创建正式 Release，也没有签名安装包。

自动化页面验收不代表真实串口或外部 AI 服务已经联调。原有 4 项硬件测试仍需实际设备，
屏幕布局截图也需要结合现场缩放比例与操作流程复核。
