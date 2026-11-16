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
        .timeout(std::time::Duration::from_secs(90))
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

/// OpenAI 兼容 chat/completions 代理（WebView 直连内网 API 会被 CORS 拦，走 Rust 转发）。
/// 网络策略：内网 API 一律直连（.no_proxy()），不走系统代理——
/// 本机开着 Clash 等代理工具也不会劫持内网请求。
#[tauri::command]
pub async fn ai_chat(
    cfg: AiConfig,
    mut body: serde_json::Value,
) -> Result<serde_json::Value, String> {
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
}
