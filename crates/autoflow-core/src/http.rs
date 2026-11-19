//! Plain HTTP(S) requests (ureq, rustls) for `http_request` and the AI providers.

use anyhow::{anyhow, Context as _, Result};
use serde_json::Value;
use std::time::Duration;

/// Largest response body read into memory (downloads included).
const MAX_BODY: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn is_json(&self) -> bool {
        self.content_type.to_ascii_lowercase().contains("json")
    }

    /// The body as JSON, if it parses.
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// A short description of an error response (the API's own message when it has one).
    pub fn error_summary(&self) -> String {
        let detail = self
            .json()
            .and_then(|v| {
                let e = v.get("error").cloned().unwrap_or(v);
                match &e {
                    Value::String(s) => Some(s.clone()),
                    _ => e.get("message").and_then(Value::as_str).map(str::to_string),
                }
            })
            .unwrap_or_else(|| self.text());
        let detail: String = detail.trim().chars().take(400).collect();
        format!("HTTP {}: {detail}", self.status)
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .user_agent("AutoFlowMate")
        .build()
        .into()
}

/// Send a request. Error statuses (4xx/5xx) are returned as a normal `Response`.
pub fn send(method: &str, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>, timeout: Duration) -> Result<Response> {
    let method = method.trim().to_ascii_uppercase();
    let method = if method.is_empty() { "GET".to_string() } else { method };
    let mut b = ureq::http::Request::builder().method(method.as_str()).uri(url.trim());
    for (k, v) in headers {
        b = b.header(k.as_str(), v.as_str());
    }
    let agent = agent(timeout);
    let result = match body {
        Some(bytes) => agent.run(b.body(bytes).context("invalid request")?),
        None => agent.run(b.body(()).context("invalid request")?),
    };
    let mut resp = result.map_err(|e| anyhow!("{method} {url}: {e}"))?;
    let status = resp.status().as_u16();
    let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let body = resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().with_context(|| format!("read response of {url}"))?;
    Ok(Response { status, content_type, body })
}

/// POST a JSON body and return the JSON reply, failing on error statuses.
pub fn post_json(url: &str, headers: &[(String, String)], body: &Value, timeout: Duration) -> Result<Value> {
    let mut h = headers.to_vec();
    h.push(("content-type".into(), "application/json".into()));
    let r = send("POST", url, &h, Some(serde_json::to_vec(body)?), timeout)?;
    if r.status >= 400 {
        return Err(anyhow!(r.error_summary()));
    }
    r.json().ok_or_else(|| anyhow!("{url} did not return JSON: {}", r.text().chars().take(200).collect::<String>()))
}

/// GET a JSON document, failing on error statuses.
pub fn get_json(url: &str, headers: &[(String, String)], timeout: Duration) -> Result<Value> {
    let r = send("GET", url, headers, None, timeout)?;
    if r.status >= 400 {
        return Err(anyhow!(r.error_summary()));
    }
    r.json().ok_or_else(|| anyhow!("{url} did not return JSON"))
}
