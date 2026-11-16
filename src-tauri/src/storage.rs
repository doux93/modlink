//! Transactional JSON documents in the application database. No UI dependency.
use rusqlite::{Connection, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;

pub struct JsonDocument<'a> {
    pub database: &'a Path,
    pub key: &'a str,
    pub legacy_file: &'a Path,
}

impl JsonDocument<'_> {
    fn connection<T: Serialize + DeserializeOwned>(
        &self,
        default: &T,
    ) -> Result<Connection, String> {
        if let Some(parent) = self.database.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = Connection::open(self.database).map_err(|error| error.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS json_documents (key TEXT PRIMARY KEY, payload TEXT NOT NULL)")
            .map_err(|error| error.to_string())?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM json_documents WHERE key=?1)",
                [self.key],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if !exists {
            let payload = match std::fs::read_to_string(self.legacy_file) {
                Ok(content) => {
                    // Never silently replace malformed legacy evidence with an empty allow-list.
                    serde_json::from_str::<T>(&content).map_err(|error| error.to_string())?;
                    content
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    serde_json::to_string(default).map_err(|error| error.to_string())?
                }
                Err(error) => return Err(error.to_string()),
            };
            conn.execute(
                "INSERT OR IGNORE INTO json_documents (key,payload) VALUES (?1,?2)",
                [self.key, &payload],
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(conn)
    }

    pub fn load<T: Serialize + DeserializeOwned>(&self, default: &T) -> Result<T, String> {
        let conn = self.connection(default)?;
        let payload: String = conn
            .query_row(
                "SELECT payload FROM json_documents WHERE key=?1",
                [self.key],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        serde_json::from_str(&payload).map_err(|error| error.to_string())
    }

    pub fn update<T: Serialize + DeserializeOwned>(
        &self,
        default: &T,
        apply: impl FnOnce(&mut T) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let mut conn = self.connection(default)?;
        // Acquire the write lock before reading: updates from another port or
        // application instance cannot disappear between load and save.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let payload: String = tx
            .query_row(
                "SELECT payload FROM json_documents WHERE key=?1",
                [self.key],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut value = serde_json::from_str(&payload).map_err(|error| error.to_string())?;
        let changed = apply(&mut value)?;
        if changed {
            let json = serde_json::to_string(&value).map_err(|error| error.to_string())?;
            tx.execute(
                "UPDATE json_documents SET payload=?1 WHERE key=?2",
                [&json, self.key],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn with_store(test: impl FnOnce(&JsonDocument<'_>)) {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "modulink-store-{}-{unique}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        test(&JsonDocument {
            database: &root.join("app.db"),
            key: "evidence",
            legacy_file: &root.join("legacy.json"),
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_ports_do_not_lose_updates() {
        with_store(|store| {
            store.load(&BTreeMap::<String, u32>::new()).unwrap();
            std::thread::scope(|scope| {
                for port in 0..12 {
                    scope.spawn(move || {
                        store
                            .update(&BTreeMap::<String, u32>::new(), |data| {
                                data.insert(format!("COM{port}"), port);
                                Ok(true)
                            })
                            .unwrap();
                    });
                }
            });
            assert_eq!(
                store.load(&BTreeMap::<String, u32>::new()).unwrap().len(),
                12
            );
        });
    }

    #[test]
    fn legacy_import_is_once_only_and_preserves_original() {
        with_store(|store| {
            std::fs::write(store.legacy_file, "[1,2]").unwrap();
            assert_eq!(store.load(&Vec::<u32>::new()).unwrap(), vec![1, 2]);
            store
                .update(&Vec::<u32>::new(), |data| {
                    data.push(3);
                    Ok(true)
                })
                .unwrap();
            assert_eq!(store.load(&Vec::<u32>::new()).unwrap(), vec![1, 2, 3]);
            assert_eq!(std::fs::read_to_string(store.legacy_file).unwrap(), "[1,2]");
        });
    }

    #[test]
    fn failed_update_rolls_back() {
        with_store(|store| {
            let result = store.update(&vec![1], |value| {
                value.push(2);
                Err("interrupted".into())
            });
            assert!(result.is_err());
            assert_eq!(store.load(&Vec::<u32>::new()).unwrap(), vec![1]);
        });
    }

    #[test]
    fn corrupt_legacy_evidence_is_not_silently_discarded() {
        with_store(|store| {
            std::fs::write(store.legacy_file, "{broken").unwrap();
            assert!(store.load(&Vec::<u32>::new()).is_err());
            assert_eq!(
                std::fs::read_to_string(store.legacy_file).unwrap(),
                "{broken"
            );
        });
    }
}
