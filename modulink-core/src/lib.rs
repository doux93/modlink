//! ModuLink 核心业务逻辑。
//!
//! 本 crate 不依赖任何 UI 框架（tauri/electron），保证：
//! 1. 单元测试无需 WebView 运行时；
//! 2. 未来可复用于 CLI 版（modulink-cli）或迁移到其他壳。

pub mod at_protocol;
pub mod diag;
pub mod knowledge;
pub mod legacy_import;
pub mod log_analysis;
pub mod preview;
pub mod response_slot;
pub mod stress;
