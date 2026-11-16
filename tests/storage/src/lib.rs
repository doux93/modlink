//! Runs the actual production storage adapter without a WebView or serial port.
#[path = "../../../src-tauri/src/storage.rs"]
pub mod storage;

#[path = "../../../src-tauri/src/atomic_file.rs"]
pub mod atomic_file;
#[path = "../../../src-tauri/src/quickcmd_file.rs"]
pub mod quickcmd_file;
#[path = "../../../src-tauri/src/settings_commit.rs"]
pub mod settings_commit;

#[path = "../../../src-tauri/src/config.rs"]
pub mod config;
