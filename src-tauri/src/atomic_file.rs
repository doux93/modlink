use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct StagedFile(PathBuf);
impl StagedFile {
    pub fn commit(&self, path: &Path) -> Result<(), std::io::Error> {
        fs::rename(&self.0, path)
    }
}
impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn stage(path: &Path, bytes: &[u8]) -> Result<StagedFile, String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or("文件目录无效")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    let temporary = parent.join(format!(
        ".modulink-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| format!("创建临时文件失败: {e}"))?;
    let guard = StagedFile(temporary);
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file); // Close first so cleanup also works on Windows.
    result.map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(guard)
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    stage(path, bytes)?
        .commit(path)
        .map_err(|e| format!("提交文件失败，原文件已保留: {e}"))
}
