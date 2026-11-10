use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const MAX_RAW: usize = 64 * 1024;

#[derive(Deserialize)]
pub struct Endpoint {
    pub url: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Clone)]
struct Chunk {
    id: u32,
    text: String,
}

static ACTIVE: Mutex<Option<HashMap<u32, Arc<AtomicBool>>>> = Mutex::new(None);

fn base(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url.to_string())
    } else {
        Err("The server address must start with http:// or https://".into())
    }
}

fn client() -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent("clight")
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())
}

fn authorized(req: reqwest::RequestBuilder, key: &str) -> reqwest::RequestBuilder {
    if key.trim().is_empty() {
        req
    } else {
        req.header("Authorization", format!("Bearer {}", key.trim()))
    }
}

fn explain(e: reqwest::Error) -> String {
    if e.is_connect() || e.is_timeout() {
        format!("Can't reach the server ({e}). Is it running?")
    } else {
        e.to_string()
    }
}

#[derive(Debug, PartialEq)]
enum Piece {
    Text(String),
    Done,
    Skip,
}

fn parse_line(line: &str) -> Piece {
    let Some(data) = line.trim().strip_prefix("data:") else {
        return Piece::Skip;
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Piece::Done;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return Piece::Skip;
    };
    let choice = &v["choices"][0];
    match choice["delta"]["content"]
        .as_str()
        .or_else(|| choice["text"].as_str())
    {
        Some(t) if !t.is_empty() => Piece::Text(t.to_string()),
        _ => Piece::Skip,
    }
}

/// Some servers ignore `stream: true` and answer with one JSON document.
fn whole_answer(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let choice = &v["choices"][0];
    choice["message"]["content"]
        .as_str()
        .or_else(|| choice["text"].as_str())
        .map(str::to_string)
}

fn error_text(status: reqwest::StatusCode, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .or_else(|| v["message"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(200).collect());
    match status.as_u16() {
        401 | 403 => format!("The server refused the API key ({status})"),
        404 => format!("Model or address not found ({status}). {detail}"),
        _ => format!("{status}: {detail}"),
    }
}

pub async fn chat(
    app: &AppHandle,
    id: u32,
    ep: Endpoint,
    messages: Vec<Message>,
) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    ACTIVE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(id, cancel.clone());
    let result = run(app, id, &ep, &messages, &cancel).await;
    if let Some(map) = ACTIVE.lock().unwrap().as_mut() {
        map.remove(&id);
    }
    result
}

pub fn cancel(id: u32) {
    if let Some(flag) = ACTIVE.lock().unwrap().as_ref().and_then(|m| m.get(&id)) {
        flag.store(true, Ordering::SeqCst);
    }
}

async fn run(
    app: &AppHandle,
    id: u32,
    ep: &Endpoint,
    messages: &[Message],
    cancel: &AtomicBool,
) -> Result<(), String> {
    let builtin = ep.url.trim() == "builtin";
    let root = if builtin {
        format!("{}/v1", crate::llama::ensure(app, id).await?)
    } else {
        base(&ep.url)?
    };
    let url = format!("{root}/chat/completions");
    let mut body = json!({ "messages": messages, "stream": true, "temperature": 0.2 });
    if builtin {
        body["max_tokens"] = json!(1024);
    } else if !ep.model.trim().is_empty() {
        body["model"] = json!(ep.model.trim());
    }
    let req = authorized(client()?.post(url), &ep.key)
        .header("Content-Type", "application/json")
        .body(body.to_string());
    let mut response = req.send().await.map_err(explain)?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(error_text(status, &text));
    }
    let emit = |text: String| {
        if builtin {
            crate::llama::touch();
        }
        let _ = app.emit("ai-chunk", Chunk { id, text });
    };
    let (mut pending, mut raw, mut got_text) = (Vec::<u8>::new(), String::new(), false);
    while let Some(bytes) = response.chunk().await.map_err(explain)? {
        if cancel.load(Ordering::SeqCst) {
            return Ok(());
        }
        pending.extend_from_slice(&bytes);
        if raw.len() < MAX_RAW {
            raw.push_str(&String::from_utf8_lossy(&bytes));
        }
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            match parse_line(&String::from_utf8_lossy(&line)) {
                Piece::Text(t) => {
                    got_text = true;
                    emit(t);
                }
                Piece::Done => return Ok(()),
                Piece::Skip => {}
            }
        }
    }
    if !got_text {
        match whole_answer(&raw) {
            Some(text) => emit(text),
            None => return Err("The server sent an answer clight doesn't understand".into()),
        }
    }
    Ok(())
}

pub async fn models(ep: Endpoint) -> Result<Vec<String>, String> {
    let url = format!("{}/models", base(&ep.url)?);
    let response = authorized(client()?.get(url), &ep.key)
        .send()
        .await
        .map_err(explain)?;
    let status = response.status();
    let text = response.text().await.map_err(explain)?;
    if !status.is_success() {
        return Err(error_text(status, &text));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|_| "The server didn't answer like an AI server".to_string())?;
    let list = v["data"].as_array().or_else(|| v["models"].as_array());
    let mut names: Vec<String> = list
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_lines_become_text_and_the_end_marker() {
        let line = r#"data: {"choices":[{"delta":{"content":"ciao"}}]}"#;
        assert_eq!(parse_line(line), Piece::Text("ciao".into()));
        assert_eq!(parse_line("data: [DONE]"), Piece::Done);
        assert_eq!(parse_line(": keep-alive"), Piece::Skip);
        assert_eq!(
            parse_line(r#"data: {"choices":[{"delta":{}}]}"#),
            Piece::Skip
        );
        assert_eq!(parse_line("data: not json"), Piece::Skip);
        let old = r#"data: {"choices":[{"text":"x"}]}"#;
        assert_eq!(parse_line(old), Piece::Text("x".into()));
    }

    #[test]
    fn a_non_streaming_answer_is_still_read() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"tutto qui"}}]}"#;
        assert_eq!(whole_answer(raw).as_deref(), Some("tutto qui"));
        assert_eq!(whole_answer("<html>"), None);
    }

    #[test]
    fn addresses_are_checked_and_cleaned() {
        assert_eq!(
            base(" http://localhost:11434/v1/ ").unwrap(),
            "http://localhost:11434/v1"
        );
        assert!(base("localhost:8080").is_err());
    }

    #[test]
    fn server_errors_are_explained() {
        let body = r#"{"error":{"message":"model 'x' not found"}}"#;
        assert!(error_text(reqwest::StatusCode::NOT_FOUND, body).contains("model 'x' not found"));
        assert!(error_text(reqwest::StatusCode::UNAUTHORIZED, "").contains("API key"));
    }
}
