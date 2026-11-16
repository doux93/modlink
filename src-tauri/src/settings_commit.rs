use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct StagedFile(PathBuf);
impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// Stage all fallible file writes before changing credentials. A failed rename
// restores the previous key; a key-only update never rewrites the settings file.
pub fn commit_settings(
    path: &Path,
    json: Option<&[u8]>,
    key_change: Option<Option<String>>,
    read_key: impl Fn() -> Result<Option<String>, String>,
    write_key: impl Fn(Option<&str>) -> Result<(), String>,
) -> Result<(), String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let staged = if let Some(json) = json {
        let parent = path.parent().ok_or("设置目录无效")?;
        fs::create_dir_all(parent).map_err(|e| format!("创建设置目录失败: {e}"))?;
        let temporary = parent.join(format!(
            ".settings-{}-{}.tmp",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("创建临时设置失败: {e}"))?;
        let guard = StagedFile(temporary);
        let write_result = file.write_all(json).and_then(|_| file.sync_all());
        // Windows cannot remove an open file: close it before the guard cleans up.
        drop(file);
        write_result.map_err(|e| format!("写入设置失败: {e}"))?;
        Some(guard)
    } else {
        None
    };
    let previous_key = if key_change.is_some() {
        read_key()?
    } else {
        None
    };
    if let Some(key) = &key_change {
        write_key(key.as_deref())?;
    }
    if let Some(staged) = staged {
        if let Err(error) = fs::rename(&staged.0, path) {
            if key_change.is_some() {
                if let Err(rollback) = write_key(previous_key.as_deref()) {
                    return Err(format!(
                        "提交设置失败: {error}；API key 恢复失败: {rollback}，请重新配置密钥"
                    ));
                }
            }
            return Err(format!("提交设置失败: {error}；已保留原配置"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn temp() -> Temp {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "modulink-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }
    #[test]
    fn replaces_existing_json_after_credentials_succeed() {
        let dir = temp();
        let path = dir.0.join("settings.json");
        fs::write(&path, b"old").unwrap();
        let key = RefCell::new(Some("old-key".to_string()));
        commit_settings(
            &path,
            Some(b"new"),
            Some(Some("new-key".into())),
            || Ok(key.borrow().clone()),
            |value| {
                *key.borrow_mut() = value.map(str::to_string);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(key.borrow().as_deref(), Some("new-key"));
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
    #[test]
    fn failed_credential_write_keeps_existing_file() {
        let dir = temp();
        let path = dir.0.join("settings.json");
        fs::write(&path, b"old").unwrap();
        assert!(commit_settings(
            &path,
            Some(b"new"),
            Some(None),
            || Ok(Some("old-key".into())),
            |_| Err("credential denied".into())
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
    #[test]
    fn staging_failure_never_changes_credentials() {
        let dir = temp();
        let parent = dir.0.join("blocked");
        fs::write(&parent, b"not a directory").unwrap();
        let touched = Cell::new(false);
        assert!(commit_settings(
            &parent.join("settings.json"),
            Some(b"new"),
            Some(None),
            || Ok(None),
            |_| {
                touched.set(true);
                Ok(())
            }
        )
        .is_err());
        assert!(!touched.get());
    }
    #[test]
    fn rename_failure_restores_previous_key() {
        let dir = temp();
        let path = dir.0.join("settings.json");
        fs::create_dir(&path).unwrap();
        let key = RefCell::new(Some("old-key".to_string()));
        assert!(commit_settings(
            &path,
            Some(b"new"),
            Some(None),
            || Ok(key.borrow().clone()),
            |value| {
                *key.borrow_mut() = value.map(str::to_string);
                Ok(())
            }
        )
        .is_err());
        assert_eq!(key.borrow().as_deref(), Some("old-key"));
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
    #[test]
    fn key_only_change_does_not_touch_the_filesystem() {
        let key = RefCell::new(Some("old-key".to_string()));
        commit_settings(
            Path::new("unused/settings.json"),
            None,
            Some(None),
            || Ok(key.borrow().clone()),
            |value| {
                *key.borrow_mut() = value.map(str::to_string);
                Ok(())
            },
        )
        .unwrap();
        assert!(key.borrow().is_none());
    }

    #[test]
    fn rollback_failure_reports_partial_state() {
        let dir = temp();
        let path = dir.0.join("settings.json");
        fs::create_dir(&path).unwrap();
        let writes = Cell::new(0);
        let error = commit_settings(
            &path,
            Some(b"new"),
            Some(None),
            || Ok(Some("old-key".into())),
            |_| {
                writes.set(writes.get() + 1);
                if writes.get() == 1 {
                    Ok(())
                } else {
                    Err("credential denied".into())
                }
            },
        )
        .unwrap_err();
        assert!(error.contains("API key 恢复失败"));
        assert!(error.contains("请重新配置密钥"));
        assert!(!error.contains("已保留原配置"));
        assert_eq!(writes.get(), 2);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
}
