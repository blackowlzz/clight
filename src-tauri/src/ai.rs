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
static SUGGESTING: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);

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
        format!("{}/v1", crate::llama::ensure(app, id, &ep.model).await?)
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

/// What the model sees to finish a line.
pub struct Snapshot {
    pub key: String,
    pub file: String,
    pub head: String,
    pub prefix: String,
    pub suffix: String,
    pub others: Vec<(String, String)>,
}

#[derive(Serialize, Clone)]
struct Suggestion {
    key: String,
    text: String,
}

/// The Qwen2.5-Coder format for filling in the middle, with the other open files as context.
pub fn fim_prompt(s: &Snapshot) -> String {
    let mut p = String::from("<|repo_name|>project\n");
    for (name, text) in &s.others {
        p.push_str(&format!("<|file_sep|>{name}\n{text}\n"));
    }
    let lead = if s.head.is_empty() {
        String::new()
    } else {
        format!("{}\n// …\n", s.head.trim_end())
    };
    p.push_str(&format!(
        "<|file_sep|>{}\n<|fim_prefix|>{lead}{}<|fim_suffix|>{}<|fim_middle|>",
        s.file, s.prefix, s.suffix
    ));
    p
}

/// Turns what the model wrote into something to show after the caret, or nothing.
pub fn clean(raw: &str, prefix: &str, suffix: &str) -> Option<String> {
    let mut text = raw.lines().next().unwrap_or("").trim_end().to_string();
    if text.chars().any(|c| c.is_control() && c != '\t') {
        return None;
    }
    // The line already ends with what the editor closed for the user.
    let closers = suffix.lines().next().unwrap_or("").trim_end();
    let mut cut = false;
    if !closers.is_empty() {
        if let Some(at) = text.find(closers) {
            text.truncate(at);
            cut = true;
        }
    }
    if prefix.ends_with(' ') || prefix.ends_with('\t') {
        text = text.trim_start().to_string();
    }
    // Something follows the caret, so a trailing space is part of the answer.
    let text = if cut {
        text
    } else {
        text.trim_end().to_string()
    };
    let worth = text.chars().filter(|c| !c.is_whitespace()).count() >= 1;
    (worth && text.chars().count() <= 200).then_some(text)
}

/// Starts asking the built-in AI to finish the line. The answer comes as an `ai-suggestion` event.
pub fn suggest(app: AppHandle, size: String, snap: Snapshot) {
    let handle = tauri::async_runtime::spawn(async move {
        let Ok(base) = crate::llama::ensure(&app, 0, &size).await else {
            return;
        };
        let Ok(client) = client() else { return };
        let body = json!({
            "prompt": fim_prompt(&snap),
            "n_predict": 48,
            "temperature": 0,
            "top_k": 1,
            "cache_prompt": true,
            "stream": false,
            "stop": ["\n", "<|file_sep|>", "<|fim_pad|>", "<|endoftext|>", "<|im_end|>"],
        });
        let Ok(response) = client
            .post(format!("{base}/completion"))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
        else {
            return;
        };
        if !response.status().is_success() {
            return;
        }
        let Ok(text) = response.text().await else {
            return;
        };
        crate::llama::touch();
        let content = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["content"].as_str().map(str::to_string))
            .unwrap_or_default();
        if let Some(text) = clean(&content, &snap.prefix, &snap.suffix) {
            let _ = app.emit(
                "ai-suggestion",
                Suggestion {
                    key: snap.key,
                    text,
                },
            );
        }
    });
    if let Some(old) = SUGGESTING.lock().unwrap().replace(handle) {
        old.abort();
    }
}

/// Drops the suggestion being asked for, which also stops the model from writing it.
pub fn cancel_suggest() {
    if let Some(old) = SUGGESTING.lock().unwrap().take() {
        old.abort();
    }
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

    fn snap() -> Snapshot {
        Snapshot {
            key: "1:2:3".into(),
            file: "main.c".into(),
            head: String::new(),
            prefix: "int main(void) {\n    printf(\"ins".into(),
            suffix: "\");\n}\n".into(),
            others: vec![("util.h".into(), "int add(int a, int b);".into())],
        }
    }

    #[test]
    fn the_prompt_uses_the_qwen_fill_in_the_middle_format() {
        assert_eq!(
            fim_prompt(&snap()),
            "<|repo_name|>project\n<|file_sep|>util.h\nint add(int a, int b);\n<|file_sep|>main.c\n<|fim_prefix|>int main(void) {\n    printf(\"ins<|fim_suffix|>\");\n}\n<|fim_middle|>"
        );
        let mut cut = snap();
        cut.head = "#include <stdio.h>\n".into();
        assert!(fim_prompt(&cut).contains("<|fim_prefix|>#include <stdio.h>\n// …\nint main"));
    }

    #[test]
    fn the_answer_is_cut_where_the_line_is_already_closed() {
        assert_eq!(
            clean("erisci i numeri: \");", "x(\"ins", "\");\n}").as_deref(),
            Some("erisci i numeri: ")
        );
        assert_eq!(
            clean("n = 5;\nint other;", "int ", "").as_deref(),
            Some("n = 5;")
        );
        assert_eq!(clean("  x", "a = ", "").as_deref(), Some("x"));
        assert_eq!(clean("   ", "a", ""), None);
        assert_eq!(clean("a\u{0}b", "a", ""), None);
    }
}
