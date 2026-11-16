use crate::atomic_file;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn backup(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::symlink_metadata(path) {
                Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Ok(None),
                _ => Err(format!("快捷指令路径存在但无法读取，原内容已保留: {error}")),
            }
        }
        Err(error) => Err(format!("读取快捷指令失败，原文件已保留: {error}")),
    }
}

pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match read(path)? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("快捷指令文件损坏，原文件已保留，可尝试恢复备份: {e}")),
        None if backup(path).exists() => Err("快捷指令文件缺失，可尝试恢复备份".into()),
        None => Ok(None),
    }
}

pub fn save<T: Serialize + DeserializeOwned>(path: &Path, value: &T) -> Result<(), String> {
    // Never overwrite an unreadable or malformed source with an empty workspace.
    load::<T>(path)?;
    let json = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let staged = atomic_file::stage(path, &json)?;
    if let Some(previous) = read(path)? {
        atomic_file::write(&backup(path), &previous)?;
    }
    staged
        .commit(path)
        .map_err(|e| format!("保存快捷指令失败，原文件已保留: {e}"))
}

pub fn restore<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = read(&backup(path))?.ok_or("没有可恢复的快捷指令备份")?;
    let value =
        serde_json::from_slice(&bytes).map_err(|e| format!("备份无法读取，未修改原文件: {e}"))?;
    if let Some(original) = read(path)? {
        // Preserve damaged bytes for manual recovery. create_new avoids overwriting any archive.
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let archive = path.with_extension(format!(
            "json.damaged-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(archive)
            .map_err(|e| e.to_string())?;
        file.write_all(&original)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("保留损坏文件失败，未恢复: {e}"))?;
    }
    atomic_file::write(path, &bytes)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn temp() -> Temp {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "modulink-quick-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }
    #[test]
    fn missing_is_distinct_from_corruption_and_access_failure() {
        let dir = temp();
        let path = dir.0.join("current.json");
        assert_eq!(load::<Vec<String>>(&path).unwrap(), None);
        fs::write(&path, "broken").unwrap();
        assert!(load::<Vec<String>>(&path).is_err());
        assert!(save(&path, &vec!["new".to_string()]).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
        assert!(load::<Vec<String>>(&dir.0).is_err());
    }
    #[test]
    fn maintains_previous_revision_and_preserves_corrupt_source_during_recovery() {
        let dir = temp();
        let path = dir.0.join("current.json");
        save(&path, &vec!["first".to_string()]).unwrap();
        save(&path, &vec!["second".to_string()]).unwrap();
        assert_eq!(load::<Vec<String>>(&path).unwrap().unwrap(), vec!["second"]);
        fs::write(&path, "broken").unwrap();
        assert_eq!(restore::<Vec<String>>(&path).unwrap(), vec!["first"]);
        assert_eq!(load::<Vec<String>>(&path).unwrap().unwrap(), vec!["first"]);
        let archived = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().contains(".damaged-"))
            .unwrap();
        assert_eq!(fs::read_to_string(archived.path()).unwrap(), "broken");
    }
    #[test]
    fn failed_backup_write_preserves_current_file() {
        let dir = temp();
        let path = dir.0.join("current.json");
        save(&path, &vec![1]).unwrap();
        fs::create_dir(backup(&path)).unwrap();
        assert!(save(&path, &vec![2]).is_err());
        assert_eq!(load::<Vec<i32>>(&path).unwrap().unwrap(), vec![1]);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
    #[test]
    fn invalid_backup_never_overwrites_original() {
        let dir = temp();
        let path = dir.0.join("current.json");
        fs::write(&path, "original").unwrap();
        fs::write(backup(&path), "bad backup").unwrap();
        assert!(restore::<Vec<i32>>(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    }
}
