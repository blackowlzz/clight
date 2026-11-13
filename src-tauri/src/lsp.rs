use crate::doc::{Diag, Doc};
use crate::lexer;
use crate::runner::{clean_env, lower_priority};
use crate::AppState;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const MAX_LSP_BYTES: usize = 1 << 20;

pub struct Lsp {
    sync_order: Mutex<()>,
    legend: Mutex<(Vec<String>, Vec<String>)>,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    next: AtomicI64,
    pending: Mutex<HashMap<i64, Sender<Result<Value, String>>>>,
}

#[derive(Serialize, Clone)]
pub struct FileEdits {
    pub path: String,
    pub edits: Vec<(usize, usize, usize, usize, String)>,
}

pub fn parse_workspace_edit(v: &Value) -> Vec<FileEdits> {
    let mut out = vec![];
    let n = |v: &Value| v.as_u64().unwrap_or(0) as usize;
    let mut add = |uri: &str, list: &Value| {
        let Some(path) = path_from_uri(uri) else {
            return;
        };
        let edits = list
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|e| {
                let r = &e["range"];
                (
                    n(&r["start"]["line"]),
                    n(&r["start"]["character"]),
                    n(&r["end"]["line"]),
                    n(&r["end"]["character"]),
                    e["newText"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        out.push(FileEdits {
            path: path.to_string_lossy().into_owned(),
            edits,
        });
    };
    if let Some(map) = v["changes"].as_object() {
        for (uri, list) in map {
            add(uri, list);
        }
    }
    if let Some(arr) = v["documentChanges"].as_array() {
        for d in arr {
            if let Some(uri) = d["textDocument"]["uri"].as_str() {
                add(uri, &d["edits"]);
            }
        }
    }
    out
}

fn same_file(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
    } else {
        a == b
    }
}

pub fn uri_from_path(p: &Path) -> String {
    let mut out = String::from("file://");
    let s = p.to_string_lossy().replace('\\', "/");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b':' if cfg!(windows) => out.push(':'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let s = String::from_utf8(out).ok()?;
    if cfg!(windows) {
        let mut p = s.trim_start_matches('/').replace('/', "\\");
        if let Some(first) = p.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        Some(PathBuf::from(p))
    } else {
        Some(PathBuf::from(s))
    }
}

impl Lsp {
    pub fn spawn(
        app: AppHandle,
        root: &Path,
        build_dir: &Path,
        index: bool,
    ) -> Result<Arc<Lsp>, String> {
        let mut cmd = Command::new("clangd");
        clean_env(&mut cmd);
        cmd.arg(format!("--background-index={index}"));
        cmd.args([
            "-j=1",
            "--pch-storage=disk",
            "--clang-tidy=false",
            "--header-insertion=never",
            "--limit-results=60",
            "--log=error",
        ]);
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        cmd.arg("--malloc-trim");
        let mingw = cfg!(windows)
            .then(|| crate::tools::find_on_path("gcc.exe"))
            .flatten();
        if let Some(dir) = &mingw {
            let dir = dir.to_string_lossy().replace('\\', "/");
            cmd.arg(format!("--query-driver={dir}/*"));
        }
        if build_dir.join("compile_commands.json").exists() {
            cmd.arg(format!("--compile-commands-dir={}", build_dir.display()));
        }
        cmd.current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        lower_priority(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "clangd is not installed".to_string()
            } else {
                format!("clangd: {e}")
            }
        })?;
        let stdin = child.stdin.take().ok_or("clangd: no stdin")?;
        let stdout = child.stdout.take().ok_or("clangd: no stdout")?;
        let lsp = Arc::new(Lsp {
            sync_order: Mutex::new(()),
            legend: Mutex::new((vec![], vec![])),
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            next: AtomicI64::new(1),
            pending: Mutex::new(HashMap::new()),
        });

        let reader = lsp.clone();
        std::thread::spawn(move || reader.read_loop(stdout, app));

        let init = lsp.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": uri_from_path(root),
                "initializationOptions": {
                    "fallbackFlags": if mingw.is_some() { json!(["--target=x86_64-w64-mingw32"]) } else { json!([]) },
                },
                "capabilities": {
                    "textDocument": {
                        "completion": { "completionItem": { "snippetSupport": false } },
                        "publishDiagnostics": {},
                        "hover": { "contentFormat": ["plaintext"] },
                        "semanticTokens": { "requests": { "full": true }, "tokenTypes": [], "tokenModifiers": [], "formats": ["relative"] },
                        "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                        "signatureHelp": { "signatureInformation": { "parameterInformation": { "labelOffsetSupport": true } } },
                        "codeAction": { "codeActionLiteralSupport": { "codeActionKind": { "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "info"] } } },
                    },
                    "workspace": { "applyEdit": true },
                    "general": { "positionEncodings": ["utf-16"] },
                },
            }),
            Duration::from_secs(20),
        )?;
        let strings = |v: &Value| -> Vec<String> {
            v.as_array().map_or(vec![], |a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            })
        };
        let legend = &init["capabilities"]["semanticTokensProvider"]["legend"];
        *lsp.legend.lock().unwrap() = (
            strings(&legend["tokenTypes"]),
            strings(&legend["tokenModifiers"]),
        );
        lsp.notify("initialized", json!({}))?;
        Ok(lsp)
    }

    fn write(&self, v: &Value) -> Result<(), String> {
        let body = v.to_string();
        let mut w = self.stdin.lock().unwrap();
        write!(w, "Content-Length: {}\r\n\r\n{}", body.len(), body).map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({"jsonrpc":"2.0","method":method,"params":params}))
    }

    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.write(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let res = rx
            .recv_timeout(timeout)
            .map_err(|_| "clangd timed out".to_string());
        self.pending.lock().unwrap().remove(&id);
        res?
    }

    fn read_loop(self: Arc<Self>, stdout: std::process::ChildStdout, app: AppHandle) {
        let mut r = BufReader::with_capacity(64 * 1024, stdout);
        let mut header = String::new();
        loop {
            let mut len = 0usize;
            loop {
                header.clear();
                match r.read_line(&mut header) {
                    Ok(0) | Err(_) => {
                        self.pending.lock().unwrap().clear();
                        let _ = app.emit("lsp-exit", ());
                        return;
                    }
                    Ok(_) => {}
                }
                let line = header.trim();
                if line.is_empty() {
                    break;
                }
                if let Some(v) = line.strip_prefix("Content-Length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            if r.read_exact(&mut body).is_err() {
                continue;
            }
            let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
                continue;
            };
            self.dispatch(msg, &app);
        }
    }

    fn dispatch(&self, msg: Value, app: &AppHandle) {
        match (msg.get("id"), msg.get("method").and_then(Value::as_str)) {
            (Some(id), Some("workspace/applyEdit")) => {
                let _ = app.emit("apply-edit", parse_workspace_edit(&msg["params"]["edit"]));
                let _ = self.write(&json!({"jsonrpc":"2.0","id":id,"result":{"applied":true}}));
            }
            (Some(id), Some(_)) => {
                let _ = self.write(&json!({"jsonrpc":"2.0","id":id,"result":null}));
            }
            (Some(id), None) => {
                let Some(id) = id.as_i64() else { return };
                if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
                    let _ = tx.send(match msg.get("error") {
                        Some(e) => Err(e
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("clangd error")
                            .to_string()),
                        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                    });
                }
            }
            (None, Some("textDocument/publishDiagnostics")) => {
                let params = &msg["params"];
                let Some(path) = params["uri"].as_str().and_then(path_from_uri) else {
                    return;
                };
                let state = app.state::<AppState>();
                let mut docs = state.docs.lock().unwrap();
                let Some((&id, doc)) = docs.iter_mut().find(|(_, d)| same_file(&d.path, &path))
                else {
                    return;
                };
                doc.diags = parse_diags(
                    doc,
                    params["diagnostics"].as_array().map_or(&[], Vec::as_slice),
                );
                let _ = app.emit("diag", id);
            }
            _ => {}
        }
    }

    pub fn sync(&self, state: &AppState, id: u32) -> Result<(), String> {
        let _order = self.sync_order.lock().unwrap();
        let (uri, lang, version, first, rope) = {
            let mut docs = state.docs.lock().unwrap();
            let doc = docs.get_mut(&id).ok_or("Document is closed")?;
            let Some(lang) = doc.lang.lsp_id() else {
                return Ok(());
            };
            if doc.rope.len_bytes() > MAX_LSP_BYTES {
                return Err("This file is too large for clangd (over 1 MB)".into());
            }
            if doc.lsp_rev == Some(doc.rev) {
                return Ok(());
            }
            let first = doc.lsp_rev.is_none();
            doc.lsp_ver += 1;
            doc.lsp_rev = Some(doc.rev);
            (
                uri_from_path(&doc.path),
                lang,
                doc.lsp_ver,
                first,
                doc.rope.clone(),
            )
        };
        let text = rope.to_string();
        if first {
            self.notify(
                "textDocument/didOpen",
                json!({"textDocument":{"uri":uri,"languageId":lang,"version":version,"text":text}}),
            )
        } else {
            self.notify(
                "textDocument/didChange",
                json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":text}]}),
            )
        }
    }

    pub fn decode_semantic(&self, data: &[Value]) -> Vec<(usize, usize, usize, u8)> {
        let raw: Vec<u64> = data.iter().map(|v| v.as_u64().unwrap_or(0)).collect();
        let (types, mods) = &*self.legend.lock().unwrap();
        decode_tokens(&raw, types, mods)
    }

    pub fn close(&self, doc: &Doc) {
        if doc.lsp_rev.is_some() {
            let _ = self.notify(
                "textDocument/didClose",
                json!({"textDocument":{"uri":uri_from_path(&doc.path)}}),
            );
        }
    }

    pub fn stop(&self) {
        let _ = self.request("shutdown", Value::Null, Duration::from_millis(400));
        let _ = self.notify("exit", Value::Null);
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn tag_for(kind: &str, global: bool) -> u8 {
    match kind {
        "function" | "method" => lexer::FUNCTION,
        "class" | "interface" | "enum" | "type" | "typeParameter" | "concept" => lexer::USER_TYPE,
        "property" | "enumMember" => lexer::MEMBER,
        "namespace" => lexer::NAMESPACE,
        "macro" => lexer::MACRO,
        "variable" if global => lexer::MEMBER,
        "variable" | "parameter" => lexer::VARIABLE,
        "comment" => lexer::COMMENT,
        _ => 0,
    }
}

fn decode_tokens(
    data: &[u64],
    types: &[String],
    mods: &[String],
) -> Vec<(usize, usize, usize, u8)> {
    let global_bits = mods
        .iter()
        .enumerate()
        .filter(|(_, m)| *m == "globalScope" || *m == "fileScope")
        .fold(0u64, |b, (i, _)| b | (1 << i));
    let (mut line, mut col) = (0usize, 0usize);
    let mut out = Vec::with_capacity(data.len() / 5);
    for t in data.as_chunks::<5>().0 {
        if t[0] > 0 {
            line += t[0] as usize;
            col = t[1] as usize;
        } else {
            col += t[1] as usize;
        }
        let kind = types.get(t[3] as usize).map_or("", String::as_str);
        out.push((
            line,
            col,
            t[2] as usize,
            tag_for(kind, t[4] & global_bits != 0),
        ));
    }
    out
}

fn parse_diags(doc: &Doc, list: &[Value]) -> Vec<Diag> {
    let n = |v: &Value| v.as_u64().unwrap_or(0) as usize;
    list.iter()
        .map(|d| {
            let (sl, el) = (
                n(&d["range"]["start"]["line"]),
                n(&d["range"]["end"]["line"]),
            );
            Diag {
                sl,
                sc: doc.col_from_utf16(sl, n(&d["range"]["start"]["character"])),
                el,
                ec: doc.col_from_utf16(el, n(&d["range"]["end"]["character"])),
                sev: d["severity"].as_u64().unwrap_or(1) as usize,
                msg: d["message"].as_str().unwrap_or("").to_string(),
                raw: d.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_tokens_decode_relative_positions() {
        let types = vec!["variable".to_string(), "class".to_string()];
        let mods = vec!["declaration".to_string(), "globalScope".to_string()];
        let data = [1, 4, 3, 1, 1, 0, 6, 2, 0, 2, 2, 0, 1, 0, 0];
        assert_eq!(
            decode_tokens(&data, &types, &mods),
            vec![
                (1, 4, 3, lexer::USER_TYPE),
                (1, 10, 2, lexer::MEMBER),
                (3, 0, 1, lexer::VARIABLE)
            ]
        );
    }

    #[test]
    fn uris_round_trip_with_spaces_and_accents() {
        let (p, want) = if cfg!(windows) {
            (PathBuf::from(r"C:\Users\user\Café C++\a b.cpp"), "file:///C:/Users/user/Caf%C3%A9%20C%2B%2B/a%20b.cpp")
        } else {
            (PathBuf::from("/home/user/Café C++/a b.cpp"), "file:///home/user/Caf%C3%A9%20C%2B%2B/a%20b.cpp")
        };
        let uri = uri_from_path(&p);
        assert_eq!(uri, want);
        assert_eq!(path_from_uri(&uri), Some(p));
    }
}
