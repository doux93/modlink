use parking_lot::Mutex;
use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

const MAX_REQUESTS: usize = 32;
const PREPARE_TTL: Duration = Duration::from_secs(60);
pub const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

struct Entry {
    cancel: Arc<watch::Sender<bool>>,
    created: Instant,
    started: bool,
}

/// Two-phase registration prevents a cancel IPC from racing ahead of ai_chat.
/// UUIDs identify individual HTTP calls, including successive tool rounds.
#[derive(Clone, Default)]
pub struct AiRequests(Arc<Mutex<HashMap<String, Entry>>>);

impl AiRequests {
    pub fn prepare(&self, id: String) -> Result<(), String> {
        if id.is_empty()
            || id.len() > 128
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("无效的 AI 请求 ID".into());
        }
        let mut entries = self.0.lock();
        entries.retain(|_, entry| entry.started || entry.created.elapsed() < PREPARE_TTL);
        if entries.contains_key(&id) {
            return Err("AI 请求 ID 已存在".into());
        }
        if entries.len() >= MAX_REQUESTS {
            return Err("AI 请求过多，请稍后重试".into());
        }
        let (cancel, _) = watch::channel(false);
        entries.insert(
            id,
            Entry {
                cancel: Arc::new(cancel),
                created: Instant::now(),
                started: false,
            },
        );
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> bool {
        if let Some(entry) = self.0.lock().remove(id) {
            // Preserve cancellation even before the HTTP future starts waiting.
            entry.cancel.send_replace(true);
            true
        } else {
            false
        }
    }

    fn claim(&self, id: &str) -> Result<ActiveRequest, String> {
        let mut entries = self.0.lock();
        let entry = entries.get_mut(id).ok_or("AI 请求已取消或未注册")?;
        if entry.started {
            return Err("AI 请求正在执行，不能重复发送".into());
        }
        if entry.created.elapsed() >= PREPARE_TTL {
            entries.remove(id);
            return Err("AI 请求注册已过期".into());
        }
        entry.started = true;
        Ok(ActiveRequest {
            registry: self.clone(),
            id: id.to_owned(),
            cancel: entry.cancel.clone(),
        })
    }

    pub async fn run<T>(
        &self,
        id: &str,
        timeout: Duration,
        operation: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let lease = self.claim(id)?;
        let mut cancelled = lease.cancel.subscribe();
        if *cancelled.borrow() {
            return Err("AI 请求已取消".into());
        }
        let timeout = timeout.min(MAX_REQUEST_TIMEOUT);
        if timeout.is_zero() {
            return Err("AI 请求已超时".into());
        }
        // Dropping the losing future stops reqwest send AND response-body reads.
        // The lease also cleans up when the command future itself is dropped.
        tokio::select! {
            biased;
            _ = cancelled.changed() => Err("AI 请求已取消".into()),
            result = tokio::time::timeout(timeout, operation) => {
                result.map_err(|_| "AI 请求已超时".to_string())?
            }
        }
    }
}

struct ActiveRequest {
    registry: AiRequests,
    id: String,
    cancel: Arc<watch::Sender<bool>>,
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        let mut entries = self.registry.0.lock();
        // A late completion must never remove a newer registration with this ID.
        if entries
            .get(&self.id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.cancel, &self.cancel))
        {
            entries.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn cancellation_before_claim_never_polls_operation() {
        let requests = AiRequests::default();
        requests.prepare("first".into()).unwrap();
        assert!(requests.cancel("first"));
        let polled = AtomicBool::new(false);
        let result = requests
            .run("first", MAX_REQUEST_TIMEOUT, async {
                polled.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert!(result.unwrap_err().contains("取消"));
        assert!(!polled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancellation_drops_in_flight_operation_and_preserves_other_requests() {
        let requests = AiRequests::default();
        requests.prepare("first".into()).unwrap();
        requests.prepare("second".into()).unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = DropFlag(dropped.clone());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let runner = requests.clone();
        let task = tokio::spawn(async move {
            runner
                .run("first", MAX_REQUEST_TIMEOUT, async move {
                    let _guard = flag;
                    let _ = ready_tx.send(());
                    std::future::pending::<Result<(), String>>().await
                })
                .await
        });
        ready_rx.await.unwrap();
        assert!(requests.cancel("first"));
        assert!(task.await.unwrap().unwrap_err().contains("取消"));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(
            requests
                .run("second", MAX_REQUEST_TIMEOUT, async { Ok(42) })
                .await
                .unwrap(),
            42
        );
        assert!(requests.0.lock().is_empty());
    }

    #[tokio::test]
    async fn timeout_drops_operation_and_releases_registration() {
        let requests = AiRequests::default();
        requests.prepare("timeout".into()).unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = DropFlag(dropped.clone());
        let result = requests
            .run("timeout", Duration::from_millis(1), async move {
                let _guard = flag;
                std::future::pending::<Result<(), String>>().await
            })
            .await;
        assert!(result.unwrap_err().contains("超时"));
        assert!(dropped.load(Ordering::SeqCst));
        assert!(requests.0.lock().is_empty());
    }

    #[tokio::test]
    async fn failures_and_dropped_command_futures_release_registration() {
        let requests = AiRequests::default();
        requests.prepare("error".into()).unwrap();
        let result = requests
            .run("error", MAX_REQUEST_TIMEOUT, async {
                Err::<(), _>("network failure".into())
            })
            .await;
        assert_eq!(result.unwrap_err(), "network failure");
        requests.prepare("abort".into()).unwrap();
        let runner = requests.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            runner
                .run("abort", MAX_REQUEST_TIMEOUT, async {
                    let _ = ready_tx.send(());
                    std::future::pending::<Result<(), String>>().await
                })
                .await
        });
        ready_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(requests.0.lock().is_empty());
    }

    #[test]
    fn duplicate_claims_are_rejected_and_old_cleanup_cannot_remove_successor() {
        let requests = AiRequests::default();
        requests.prepare("same".into()).unwrap();
        assert!(requests.prepare("same".into()).is_err());
        let old = requests.claim("same").unwrap();
        assert!(requests.claim("same").is_err());
        requests.cancel("same");
        requests.prepare("same".into()).unwrap();
        drop(old);
        assert!(requests.claim("same").is_ok());
    }

    #[test]
    fn abandoned_registrations_expire_and_capacity_is_bounded() {
        let requests = AiRequests::default();
        assert!(requests.prepare(String::new()).is_err());
        assert!(requests.prepare("x".repeat(129)).is_err());
        for i in 0..MAX_REQUESTS {
            requests.prepare(format!("request-{i}")).unwrap();
        }
        assert!(requests.prepare("extra".into()).is_err());
        requests.0.lock().get_mut("request-0").unwrap().created = Instant::now() - PREPARE_TTL;
        requests.prepare("extra".into()).unwrap();
        assert!(requests.claim("request-0").is_err());
        requests.0.lock().get_mut("request-1").unwrap().created = Instant::now() - PREPARE_TTL;
        assert!(requests.claim("request-1").is_err());
        assert!(!requests.0.lock().contains_key("request-1"));
    }
}
