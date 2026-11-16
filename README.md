# ModuLink Studio — 模组智能调测工具

新一代蜂窝模组（Cat1/Cat4/RedCap）串口调测工具，替代 UART_V2.1。
定位：以设备会话为中心、基于手册和实测证据、带安全授权与复盘能力的模组调测工作台。

## 技术栈

- **壳/后端**: Tauri 2 + Rust（`serialport-rs` 串口引擎）
- **核心逻辑**: `modulink-core/` 独立 crate，不依赖 UI 框架（可复用于 CLI）
- **前端**: Vue 3 + TypeScript + Naive UI + Vite
- **模拟器**: `simulator/at_sim.py`（Python + pyserial，剧本驱动的模组行为状态机）

## 目录结构

```
src-tauri/        Tauri 应用（串口引擎、AT 会话层、IPC 命令）
modulink-core/    核心业务 crate（老 ini 导入导出、AT 协议基础；单测在此跑）
src/              Vue3 前端（设备工作区、控制台、诊断、AI、压测、日志与知识中心）
data/             数据驱动目录（profiles、diagnostics、error_codes、scripts）
simulator/        Python AT 模组模拟器（无真机开发/演示兜底）
tests/fixtures/   老工具真实 ini 文件（导入器测试基准）
tests/frontend/   前端设备绑定、AI 状态与执行历史回归测试
tests/storage/    SQLite 事务存储测试（直接复用生产实现，不依赖 Tauri）
docs/             文档
```

## 开发

```bash
npm ci                      # 按 lockfile 安装前端依赖
npm run tauri dev           # 开发模式（热重载）
npm test                    # 前端回归测试
npm run build               # 前端类型检查 + 构建
cargo test --locked --manifest-path modulink-core/Cargo.toml
cargo test --locked --manifest-path tests/storage/Cargo.toml
python -m pip install -r tools/requirements.txt
python -m unittest discover -s tools -p "test_*.py"
```

Rust 由 `rust-toolchain.toml` 固定为 **1.94.0**，通过 rustup 自动选择；建议使用
Node.js 20 和 Windows MSVC 工具链。固定版本避免现有 Tauri 构建脚本与更新版
Cargo 构建目录布局不兼容。完整桌面测试在 Windows PowerShell 运行：

```powershell
$env:MODULINK_TEST_MANIFEST = "1"
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
Remove-Item Env:MODULINK_TEST_MANIFEST
```

此变量仅为 MSVC 测试程序嵌入 Common Controls v6 manifest，解决测试启动时的
`STATUS_ENTRYPOINT_NOT_FOUND`；普通应用构建继续由 Tauri 管理 manifest。
涉及真实串口的 ignored 测试需要按测试说明配置设备后单独运行。

当前修复、存储迁移和实机验收步骤见 [修复说明](docs/repair-notes-2026-11-16.md)。

## 无真机联调

1. 安装 [com0com](https://sourceforge.net/projects/com0com/) 创建虚拟串口对 COM10↔COM11（需管理员）
2. `python simulator/at_sim.py --port COM11 --scenario normal`
3. 应用内连接 COM10，即可完整走通收发/体检/压测流程
4. 模拟器运行中输入剧本名切换故障（`no_sim`/`weak_signal`/`apn_wrong`…），模拟现场拔卡

## 里程碑

- [x] M0 骨架 + 模拟器雏形
- [x] M1 经典控制台 + 老 ini 零成本迁移（导入/导出，4 个真实文件验证）
- [x] M1.5 全局错误浮层（白屏防复发）+ release 打包（`--no-bundle`，NSIS 下载
      走代理会失败，正式安装包留 M6 处理）
- [x] M2 全部完成：模组画像 YAML（ML307R/ML305/ML302/通用）+ 错误码知识库
      （行内解读+悬停建议）+ URC 标签 + AT 指令语法提示 + 知识库热重载 +
      多会话页签 + 型号自动识别（AT+CGMM）+ **设备组分屏（AT 口+日志口）**+
      快捷指令编辑（增删改/分组/自动持久化到 data/quickcmds/current.json）
- [x] M3 一键入网体检：诊断引擎（数据驱动解释器+表达式求值，26 单测含
      5 故障剧本）+ 六步体检流程 YAML + 步骤卡片 UI + HTML 报告导出
- [x] M4 AI 助手 v1：聊天面板（自动携带画像+最近日志上下文）、4 个工具
      （propose_at_sequence 确认卡 / lookup_error_code / get_recent_log /
      run_diagnostic）、高危指令红标+本地正则双保险、规则引擎离线降级、
      轻量知识检索（画像+at_docs 评分注入）。内网网关 onerouter 已实测：
      模型名须为 `Auto`（大小写敏感），function calling 支持完整。
      配置写入应用配置目录，API key 使用 Windows 系统凭据，面板右上角 ⚙ 可改
- [x] 型号画像：3 个实机验证画像 + 37 个离线资料画像，覆盖当前手册索引识别出的
      40 个型号；支持下拉搜索、`AT+CGMM` 精确识别和型号级知识过滤
- [x] 知识库深度对接：`kb_convert.py manuals` 从 111 个资料仓、91 个手册标题编译出
      **575 条带 DITAMAP scope 的 AT 指令变体**（411 个唯一指令）+ 1,650 条中文 canonical 主题；
      AI 新增 search_manuals 工具（完全本地检索，按画像执行“精确型号 > 同系列 >
      通用”筛选）。内置发布包只携带脱敏摘要索引；完整 Markdown 通过知识中心安装
      外部知识包，未安装时自动降级为摘要模式；推荐使用
      `modulink/knowledge-pack@3` 按客户选用型号导出组合包，加载器兼容 `@1/@2` 旧包。
      知识包契约见 `docs/knowledge-pack.md`，型号与画像缺口见 `docs/kb-coverage.md`
- [x] M5 调测闭环：全局设备上下文、诊断/场景压测、日志分析、统一执行记录、
      型号级手册能力和实测证据回流
- [ ] M5 迭代：AI 流式与 Markdown、会话/项目持久化、知识包安装和更新
- [ ] M6 产品化：MSVC/CI、正式安装包、升级迁移、发布验收与答辩

## 运行方式

- 开发：双击 `dev.cmd`（或 `npm run tauri dev`）
- 正式版：`src-tauri\target\release\modulink-studio.exe` 直接双击
  （打包命令 `npm run tauri build -- --no-bundle`；知识数据随资源打包，
  旧版本 exe 旁 `data/` 会在首次启动时迁移）
