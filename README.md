# ModuLink Studio — 模组智能调测工具

蜂窝模组串口调测工具，替代 UART_V2.1。
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
npm run check:bundle        # 首次加载的静态 JS 依赖总量预算
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
页面拆分、按需加载和 Windows CI 改进见 [第二轮优化说明](docs/frontend-refactor-2026-11-16.md)。

## 无真机联调

1. 安装 [com0com](https://sourceforge.net/projects/com0com/) 创建虚拟串口对 COM10↔COM11（需管理员）
2. `python simulator/at_sim.py --port COM11 --scenario normal`
3. 应用内连接 COM10，即可完整走通收发/体检/压测流程
4. 模拟器运行中输入剧本名切换故障（`no_sim`/`weak_signal`/`apn_wrong`…），模拟现场拔卡

## 运行方式

- 开发：双击 `dev.cmd`（或 `npm run tauri dev`）
- 正式版：`src-tauri\target\release\modulink-studio.exe` 直接双击
  （打包命令 `npm run tauri build -- --no-bundle`；知识数据随资源打包，
  旧版本 exe 旁 `data/` 会在首次启动时迁移）
