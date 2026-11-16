use crate::core::ai_requests::{AiRequests, MAX_REQUEST_TIMEOUT};
use crate::credentials;

#[derive(serde::Deserialize)]
pub struct AiConfig {
    base_url: String,
    model: String,
}

/// 错误原因链拼接（reqwest 的 Display 往往只有一层，chain 才能看到 DNS/TLS 细节）
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        out.push_str(" ← ");
        out.push_str(&s.to_string());
        src = s.source();
    }
    out
}

async fn ai_send_once(
    url: &str,
    key: &str,
    body: &serde_json::Value,
    proxy: Option<&str>,
) -> Result<(reqwest::StatusCode, serde_json::Value), String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(6))
        .timeout(MAX_REQUEST_TIMEOUT)
        .no_proxy(); // 显式控制：直连 或 指定代理，不受环境变量隐式影响
    if let Some(p) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(p).map_err(|e| e.to_string())?);
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .bearer_auth(key)
        .json(body)
        .send()
        .await
        .map_err(|e| error_chain(&e))?;
    let status = resp.status();
    let json: serde_json::Value = resp.json().await.map_err(|e| error_chain(&e))?;
    Ok((status, json))
}

/// Register before sending so a cancellation can also invalidate a not-yet-started request.
#[tauri::command]
pub fn prepare_ai_request(
    requests: tauri::State<'_, AiRequests>,
    request_id: String,
) -> Result<(), String> {
    requests.prepare(request_id)
}

#[tauri::command]
pub fn cancel_ai_request(requests: tauri::State<'_, AiRequests>, request_id: String) -> bool {
    requests.cancel(&request_id)
}

/// OpenAI 兼容 chat/completions 代理，固定直连，不使用环境变量或系统代理。
#[tauri::command]
pub async fn ai_chat(
    requests: tauri::State<'_, AiRequests>,
    request_id: String,
    timeout_ms: u64,
    cfg: AiConfig,
    mut body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    requests
        .run(
            &request_id,
            std::time::Duration::from_millis(timeout_ms),
            async move {
                let api_key = credentials::get_api_key()?
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| "尚未配置 AI API key".to_string())?;
                body["model"] = serde_json::Value::String(cfg.model.clone());
                let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
                let (status, json) = ai_send_once(&url, &api_key, &body, None)
                    .await
                    .map_err(|e| format!("请求失败(直连): {e}"))?;
                if !status.is_success() {
                    return Err(format!("API {status}: {json}"));
                }
                Ok(json)
            },
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    async fn assert_cancel_closes_connection(send_headers: bool) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/chat/completions", listener.local_addr().unwrap());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
            }
            let content_length = String::from_utf8(headers)
                .unwrap()
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|s| s.parse::<usize>().ok())
                })
                .unwrap();
            stream.read_exact(&mut vec![0; content_length]).unwrap();
            if send_headers {
                // Deliberately leave the body unfinished; cancel must cover json().await too.
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{").unwrap();
            }
            let _ = ready_tx.send(());
            let mut byte = [0];
            match stream.read(&mut byte) {
                Ok(0) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) => {}
                other => panic!("cancelled HTTP connection remained open: {other:?}"),
            }
        });
        let requests = AiRequests::default();
        requests.prepare("http-test".into()).unwrap();
        let runner = requests.clone();
        let task = tokio::spawn(async move {
            runner
                .run("http-test", MAX_REQUEST_TIMEOUT, async {
                    ai_send_once(
                        &url,
                        "test-only",
                        &serde_json::json!({"messages": []}),
                        None,
                    )
                    .await
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), ready_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(requests.cancel("http-test"));
        assert!(task.await.unwrap().unwrap_err().contains("取消"));
        // Let Hyper's connection task run while the server observes the close.
        tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancellation_closes_http_while_waiting_for_headers() {
        assert_cancel_closes_connection(false).await;
    }

    #[tokio::test]
    async fn cancellation_closes_http_while_reading_body() {
        assert_cancel_closes_connection(true).await;
    }
}
