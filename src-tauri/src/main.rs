#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
#[cfg(any(windows, test))]
mod cli;
#[cfg(unix)]
mod debug;
mod doc;
mod format;
mod git;
mod lexer;
mod lsp;
mod project;
mod runner;
mod suggest;
mod syntax;
mod term;
mod tools;
mod update;
mod words;

use doc::{Cmd, Doc, View};
use lsp::{parse_workspace_edit, path_from_uri, uri_from_path, FileEdits, Lsp};
use runner::Runner;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;

const MAX_FILE: u64 = 64 << 20;
const LSP_TIMEOUT: Duration = Duration::from_secs(8);
const RUN_TERM: u32 = 0;

pub struct AppState {
    pub docs: Mutex<HashMap<u32, Doc>>,
    next_doc: AtomicU32,
    lsp: Mutex<Option<Arc<Lsp>>>,
    runner: Arc<Runner>,
    terms: Mutex<HashMap<u32, Arc<term::Term>>>,
    next_term: AtomicU32,
    #[cfg(unix)]
    dbg: Mutex<Option<Arc<debug::Debugger>>>,
}

type Res<T> = Result<T, String>;

#[tauri::command]
fn app_ready(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if let Ok(icon) = tauri::image::Image::from_bytes(include_bytes!("../icons/128x128@2x.png"))
        {
            let _ = w.set_icon(icon);
        }
        let _ = w.show();
        #[cfg(feature = "selftest")]
        if let Some(code) =
            std::env::var_os("CLIGHT_SELFTEST").and_then(|p| fs::read_to_string(p).ok())
        {
            let _ = w.eval(code);
        }
    }
}

#[cfg(feature = "selftest")]
#[tauri::command]
fn selftest_log(msg: String) {
    println!("{msg}");
}

#[derive(Serialize)]
struct StartTarget {
    root: String,
    file: Option<String>,
}

#[tauri::command]
fn initial_path() -> Option<StartTarget> {
    let arg = std::env::args().nth(1)?;
    let path = match std::env::var_os("OWD") {
        Some(dir) if Path::new(&arg).is_relative() => Path::new(&dir).join(&arg),
        _ => Path::new(&arg).to_path_buf(),
    };
    let p = canon(&path.to_string_lossy()).ok()?;
    let text = |p: &Path| p.to_string_lossy().into_owned();
    if p.is_dir() {
        Some(StartTarget {
            root: text(&p),
            file: None,
        })
    } else {
        Some(StartTarget {
            root: text(p.parent()?),
            file: Some(text(&p)),
        })
    }
}

#[tauri::command(async)]
fn clipboard_read(app: AppHandle) -> Res<String> {
    app.clipboard().read_text().map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn clipboard_write(text: String, app: AppHandle) -> Res<()> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn doc_format(id: u32, style: String, state: State<'_, AppState>) -> Res<usize> {
    let (path, text, use_tabs, indent, lines, rev) = {
        let docs = state.docs.lock().unwrap();
        let doc = docs.get(&id).ok_or("Document is closed")?;
        if doc.lang.lsp_id().is_none() {
            return Err("Only C and C++ files can be formatted".into());
        }
        (
            doc.path.clone(),
            doc.rope.to_string(),
            doc.use_tabs,
            doc.tab,
            doc.selection_lines(),
            doc.rev,
        )
    };
    let edits = format::replacements(format::Request {
        path: &path,
        text,
        style: &style,
        use_tabs,
        indent,
        lines,
    })?;
    let mut docs = state.docs.lock().unwrap();
    let doc = docs.get_mut(&id).ok_or("Document is closed")?;
    if doc.rev != rev {
        return Err("The file changed while formatting".into());
    }
    Ok(doc.apply_byte_replacements(edits))
}

#[cfg(unix)]
#[tauri::command]
fn install_cli() -> Res<String> {
    use std::os::unix::fs::PermissionsExt;
    let exe = std::env::var("APPIMAGE")
        .or_else(|_| std::env::current_exe().map(|p| p.to_string_lossy().into_owned()))
        .map_err(|e| e.to_string())?;
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let bin = Path::new(&home).join(".local/bin");
    fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let script = format!("#!/bin/sh\nnohup \"{exe}\" \"$@\" >/dev/null 2>&1 </dev/null &\n");
    let mut installed = vec![];
    for name in ["clight", "cl"] {
        let target = bin.join(name);
        let meta = fs::symlink_metadata(&target);
        let ours = meta.as_ref().is_ok_and(|m| m.file_type().is_symlink())
            || fs::read_to_string(&target).is_ok_and(|s| s.contains("clight"));
        if meta.is_ok() && !ours {
            continue;
        }
        if meta.is_ok() {
            fs::remove_file(&target).map_err(|e| e.to_string())?;
        }
        fs::write(&target, &script).map_err(|e| e.to_string())?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        installed.push(name);
    }
    Ok(installed.join(" and "))
}

#[cfg(windows)]
#[tauri::command]
fn install_cli() -> Res<String> {
    cli::install()
}

fn stop_children(state: &AppState) {
    state.runner.kill();
    let terms: Vec<_> = state.terms.lock().unwrap().drain().collect();
    for (id, t) in terms {
        if id == RUN_TERM {
            t.stop();
        } else {
            t.kill();
        }
    }
    #[cfg(unix)]
    {
        let d = state.dbg.lock().unwrap().take();
        if let Some(d) = d {
            d.stop();
        }
    }
    let lsp = state.lsp.lock().unwrap().take();
    if let Some(l) = lsp {
        l.stop();
    }
}

#[tauri::command]
fn app_quit(app: AppHandle, state: State<AppState>) {
    stop_children(&state);
    app.exit(0);
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
async fn update_check(app: AppHandle) -> Res<Option<update::UpdateInfo>> {
    update::check(&app).await
}

#[tauri::command]
async fn update_install(app: AppHandle, state: State<'_, AppState>) -> Res<()> {
    update::install(&app).await?;
    stop_children(&state);
    app.restart()
}

#[tauri::command]
fn tools_status() -> Vec<tools::Status> {
    tools::status()
}

#[tauri::command]
async fn tools_install(ids: Vec<String>, app: AppHandle) -> Res<Vec<tools::Failure>> {
    tools::install(&app, &ids).await
}

#[tauri::command]
fn tool_available(exe: String) -> bool {
    tools::find_on_path(&exe).is_some()
}

#[derive(Serialize)]
struct Opened {
    id: u32,
    path: String,
    name: String,
}

fn read_text(path: &Path) -> Res<String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_FILE {
        return Err("File is larger than 64 MB".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err("This looks like a binary file".into());
    }
    String::from_utf8(bytes).map_err(|_| "Only UTF-8 files can be opened".to_string())
}

fn canon(path: &str) -> std::io::Result<std::path::PathBuf> {
    let p = fs::canonicalize(path)?;
    #[cfg(windows)]
    if let Some(short) = p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        return Ok(std::path::PathBuf::from(short));
    }
    Ok(p)
}

fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[tauri::command]
fn doc_open(path: String, state: State<AppState>) -> Res<Opened> {
    let canon = canon(&path).map_err(|e| format!("{path}: {e}"))?;
    let shown = canon.to_string_lossy().into_owned();
    let name = canon
        .file_name()
        .map_or_else(|| shown.clone(), |n| n.to_string_lossy().into_owned());
    let mut docs = state.docs.lock().unwrap();
    if let Some((id, _)) = docs.iter().find(|(_, d)| d.path == canon) {
        return Ok(Opened {
            id: *id,
            path: shown,
            name,
        });
    }
    let text = read_text(&canon)?;
    let id = state.next_doc.fetch_add(1, Ordering::Relaxed);
    let m = mtime(&canon);
    docs.insert(id, Doc::new(canon, &text, m));
    Ok(Opened {
        id,
        path: shown,
        name,
    })
}

#[tauri::command]
fn doc_close(id: u32, state: State<AppState>) {
    let doc = state.docs.lock().unwrap().remove(&id);
    if let (Some(doc), Some(l)) = (doc, state.lsp.lock().unwrap().clone()) {
        l.close(&doc);
    }
}

#[tauri::command]
fn doc_apply(
    id: u32,
    cmds: Vec<Cmd>,
    top: usize,
    rows: usize,
    reveal: bool,
    state: State<AppState>,
) -> Res<View> {
    let mut docs = state.docs.lock().unwrap();
    let doc = docs.get_mut(&id).ok_or("Document is closed")?;
    for c in cmds {
        doc.run(c)?;
    }
    Ok(doc.view(top, rows, reveal))
}

#[tauri::command]
fn doc_save(id: u32, force: bool, state: State<AppState>) -> Res<bool> {
    let mut docs = state.docs.lock().unwrap();
    let doc = docs.get_mut(&id).ok_or("Document is closed")?;
    if !force && doc.mtime.is_some() && mtime(&doc.path) != doc.mtime {
        return Ok(false);
    }
    let tmp = doc
        .path
        .with_extension(format!("clight-{}.tmp", std::process::id()));
    fs::write(&tmp, doc.contents_for_disk()).map_err(|e| e.to_string())?;
    if let Ok(meta) = fs::metadata(&doc.path) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, &doc.path).map_err(|e| e.to_string())?;
    doc.mtime = mtime(&doc.path);
    doc.saved_rev = doc.rev;
    Ok(true)
}

#[tauri::command]
fn doc_search(id: u32, q: String, case: bool, state: State<AppState>) -> Res<usize> {
    let mut docs = state.docs.lock().unwrap();
    Ok(docs
        .get_mut(&id)
        .ok_or("Document is closed")?
        .set_search(&q, case))
}

#[tauri::command]
fn docs_refresh(state: State<AppState>) -> Vec<u32> {
    let mut docs = state.docs.lock().unwrap();
    let mut changed = vec![];
    for (id, d) in docs.iter_mut() {
        let now = mtime(&d.path);
        if now.is_some() && now != d.mtime && !d.is_dirty() {
            if let Ok(text) = read_text(&d.path) {
                d.reload(&text, now);
                changed.push(*id);
            }
        }
    }
    changed
}

#[tauri::command(async)]
fn list_dir(path: String, hidden: bool) -> Res<Vec<project::Entry>> {
    project::list_dir(&path, hidden)
}

#[tauri::command(async)]
fn list_files(root: String) -> Vec<String> {
    project::list_files(&root)
}

#[tauri::command(async)]
fn grep(root: String, query: String) -> Vec<project::Hit> {
    project::grep(&root, &query)
}

#[tauri::command]
fn create_project(parent: String, name: String, cpp: bool) -> Res<String> {
    project::create_project(&parent, &name, cpp)
}

#[tauri::command]
fn fs_new_file(path: String) -> Res<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map(|_| ())
        .map_err(|e| format!("{path}: {e}"))
}

#[tauri::command]
fn fs_new_dir(path: String) -> Res<()> {
    fs::create_dir(&path).map_err(|e| format!("{path}: {e}"))
}

#[tauri::command]
fn fs_rename(from: String, to: String) -> Res<()> {
    if Path::new(&to).exists() {
        return Err(format!("{to} already exists"));
    }
    fs::rename(&from, &to).map_err(|e| e.to_string())
}

#[tauri::command]
fn fs_delete(path: String) -> Res<()> {
    let p = Path::new(&path);
    if p.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn path_exists(path: String) -> bool {
    Path::new(&path).exists()
}

#[tauri::command]
fn sys_info() -> project::Sys {
    project::sys_info()
}

#[tauri::command]
fn task_start(
    program: String,
    args: Vec<String>,
    cwd: String,
    env: Vec<(String, String)>,
    background: bool,
    app: AppHandle,
    state: State<AppState>,
) -> Res<()> {
    state
        .runner
        .start(app, &program, &args, &cwd, &env, background)
}

#[tauri::command(async)]
fn task_stdin(text: String, state: State<AppState>) -> Res<()> {
    state.runner.write_stdin(&text)
}

#[tauri::command(async)]
fn task_kill(state: State<AppState>) {
    state.runner.kill();
    let run = state.terms.lock().unwrap().get(&RUN_TERM).cloned();
    if let Some(t) = run {
        t.stop();
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command(async)]
fn run_start(
    program: String,
    args: Vec<String>,
    cwd: String,
    env: Vec<(String, String)>,
    cols: usize,
    rows: usize,
    banner: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Res<()> {
    #[cfg(feature = "selftest")]
    if std::env::var_os("CLIGHT_NO_PTY").is_some() {
        return Err("disabled by the self test".into());
    }
    // Held across the spawn so a program that exits at once is already registered.
    let mut terms = state.terms.lock().unwrap();
    if let Some(old) = terms.remove(&RUN_TERM) {
        old.stop();
    }
    let launch = term::Launch {
        program,
        args,
        cwd,
        env,
    };
    let t = term::Term::run(app, RUN_TERM, launch, cols.max(2), rows.max(2), &banner)?;
    terms.insert(RUN_TERM, t);
    Ok(())
}

#[tauri::command]
fn term_note(id: u32, text: String, state: State<AppState>) -> Res<()> {
    let t = state
        .terms
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("Terminal is closed")?;
    t.note(&text);
    Ok(())
}

#[tauri::command]
fn cmake_prepare(build: String) -> Res<()> {
    project::request_codemodel(Path::new(&build))
}

#[tauri::command(async)]
fn cmake_targets(build: String) -> Vec<project::Target> {
    project::cmake_targets(Path::new(&build))
}

#[tauri::command]
fn unbuffered_helper(dir: String) -> Res<String> {
    project::write_unbuffered_helper(Path::new(&dir))
}

#[tauri::command]
fn make_dirs(path: String) -> Res<()> {
    fs::create_dir_all(&path).map_err(|e| e.to_string())
}

fn current_lsp(state: &State<AppState>) -> Res<Arc<Lsp>> {
    state
        .lsp
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "clangd is not running".to_string())
}

#[tauri::command(async)]
fn lsp_start(
    root: String,
    build: String,
    index: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Res<()> {
    let old = state.lsp.lock().unwrap().take();
    if let Some(old) = old {
        old.stop();
    }
    let lsp = Lsp::spawn(app, Path::new(&root), Path::new(&build), index)?;
    let ids: Vec<u32> = {
        let mut docs = state.docs.lock().unwrap();
        docs.values_mut().for_each(|d| d.lsp_rev = None);
        docs.keys().copied().collect()
    };
    for id in ids {
        let _ = lsp.sync(&state, id);
    }
    *state.lsp.lock().unwrap() = Some(lsp);
    Ok(())
}

#[tauri::command(async)]
fn lsp_stop(state: State<'_, AppState>) {
    let old = state.lsp.lock().unwrap().take();
    if let Some(l) = old {
        l.stop();
    }
    for d in state.docs.lock().unwrap().values_mut() {
        d.lsp_rev = None;
        d.diags.clear();
    }
}

#[tauri::command(async)]
fn lsp_sync(id: u32, state: State<'_, AppState>) -> Res<()> {
    current_lsp(&state)?.sync(&state, id)
}

fn at_cursor(id: u32, lsp: &Lsp, state: &State<AppState>) -> Res<(Value, String, String)> {
    lsp.sync(state, id)?;
    let docs = state.docs.lock().unwrap();
    let doc = docs.get(&id).ok_or("Document is closed")?;
    if doc.lang.lsp_id().is_none() {
        return Err("Not a C or C++ file".into());
    }
    let (line, col) = doc.cursor_pos();
    let params = json!({
        "textDocument": {"uri": uri_from_path(&doc.path)},
        "position": {"line": line, "character": doc.utf16_col(line, col)},
    });
    Ok((params, doc.prefix(), doc.before_prefix()))
}

#[derive(Serialize)]
struct Completion {
    label: String,
    insert: String,
    detail: String,
}

#[derive(Serialize)]
struct Completions {
    prefix: String,
    incomplete: bool,
    items: Vec<Completion>,
}

#[tauri::command(async)]
fn syntax_check(id: u32, app: AppHandle, state: State<'_, AppState>) -> Res<()> {
    let (path, text, cpp, rev) = {
        let docs = state.docs.lock().unwrap();
        let doc = docs.get(&id).ok_or("Document is closed")?;
        let is_source = doc
            .path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["c", "cc", "cpp", "cxx"].contains(&e));
        if !is_source || doc.lang.lsp_id().is_none() {
            return Ok(());
        }
        (
            doc.path.clone(),
            doc.rope.to_string(),
            doc.lang.lsp_id() == Some("cpp"),
            doc.rev,
        )
    };
    let issues = syntax::check(&path, text, cpp)?;
    let mut docs = state.docs.lock().unwrap();
    if let Some(doc) = docs.get_mut(&id).filter(|d| d.rev == rev) {
        doc.diags = syntax::to_diags(doc, &issues);
        let _ = app.emit("diag", id);
    }
    Ok(())
}

#[tauri::command(async)]
fn lsp_complete(id: u32, force: bool, state: State<'_, AppState>) -> Res<Completions> {
    let lsp = current_lsp(&state)?;
    let (params, prefix, before) = at_cursor(id, &lsp, &state)?;
    let member = before.ends_with('.') || before.ends_with("->") || before.ends_with("::");
    if !force && !member && prefix.chars().count() < 2 {
        return Ok(Completions {
            prefix,
            incomplete: false,
            items: vec![],
        });
    }
    let res = lsp.request("textDocument/completion", params, LSP_TIMEOUT)?;
    let incomplete = res["isIncomplete"].as_bool().unwrap_or(false);
    let list = res
        .get("items")
        .unwrap_or(&res)
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut sorted: Vec<&Value> = list.iter().collect();
    sorted.sort_by_key(|i| i["sortText"].as_str().unwrap_or(""));
    let items = sorted
        .iter()
        .take(60)
        .map(|i| {
            let label = i["label"]
                .as_str()
                .unwrap_or("")
                .trim()
                .trim_start_matches('•')
                .trim()
                .to_string();
            let insert = i["textEdit"]["newText"]
                .as_str()
                .or(i["insertText"].as_str())
                .unwrap_or(&label)
                .to_string();
            Completion {
                detail: i["detail"].as_str().unwrap_or("").to_string(),
                insert,
                label,
            }
        })
        .collect();
    Ok(Completions {
        prefix,
        incomplete,
        items,
    })
}

#[tauri::command(async)]
async fn ai_chat(
    id: u32,
    endpoint: ai::Endpoint,
    messages: Vec<ai::Message>,
    app: AppHandle,
) -> Res<()> {
    ai::chat(&app, id, endpoint, messages).await
}

#[tauri::command]
fn ai_cancel(id: u32) {
    ai::cancel(id);
}

#[tauri::command(async)]
async fn ai_models(endpoint: ai::Endpoint) -> Res<Vec<String>> {
    ai::models(endpoint).await
}

/// The open file as context for the chat: all of it when short, otherwise the part around the caret.
#[tauri::command]
fn doc_context(id: u32, max: usize, state: State<AppState>) -> Option<(String, String)> {
    let docs = state.docs.lock().unwrap();
    let doc = docs.get(&id)?;
    let name = doc.path.file_name()?.to_string_lossy().into_owned();
    let total = doc.rope.len_chars();
    if total <= max {
        return Some((name, doc.rope.to_string()));
    }
    let (line, _) = doc.cursor_pos();
    let around = doc
        .rope
        .line_to_char(line.min(doc.rope.len_lines().saturating_sub(1)));
    let from = around.saturating_sub(max / 2);
    let to = (from + max).min(total);
    Some((name, doc.rope.slice(from..to).to_string()))
}

#[derive(Serialize)]
struct LineGhost {
    text: String,
    key: String,
}

#[tauri::command(async)]
fn line_suggest(id: u32, root: String, state: State<'_, AppState>) -> Option<LineGhost> {
    let docs = state.docs.lock().unwrap();
    let doc = docs.get(&id)?;
    let (line, col, prefix, suffix) = doc.line_context()?;
    if !suggest::suffix_is_closers(&suffix) {
        return None;
    }
    let key = format!("{}:{}:{}", doc.rev, line, col + 1);
    let open: Vec<&Path> = docs.values().map(|d| d.path.as_path()).collect();
    let project = suggest::project_files(&root);
    let here = doc
        .lines_for_suggest()
        .filter(|(i, _)| *i != line)
        .map(|(_, t)| (3, t));
    let others = docs
        .iter()
        .filter(|(k, _)| **k != id)
        .flat_map(|(_, d)| d.lines_for_suggest().map(|(_, t)| (2, t)));
    let files = project
        .iter()
        .filter(|(p, _)| !open.contains(&p.as_path()))
        .flat_map(|(_, text)| text.lines().map(|l| (1, std::borrow::Cow::Borrowed(l))));
    let text = suggest::complete(
        &prefix,
        &suffix,
        here.chain(others).chain(files),
        doc.lang == lexer::Lang::Cpp,
    )?;
    Some(LineGhost { text, key })
}

#[tauri::command(async)]
fn word_complete(id: u32, force: bool, state: State<'_, AppState>) -> Res<Completions> {
    const MAX_SCANNED: usize = 512 * 1024;
    let mut docs = state.docs.lock().unwrap();
    let doc = docs.get_mut(&id).ok_or("Document is closed")?;
    let prefix = doc.prefix();
    let lang = doc.lang;
    let long_enough = prefix.chars().count() >= if force { 1 } else { 2 };
    if lang.lsp_id().is_none() || !long_enough || doc.after_comment_or_string() {
        return Ok(Completions {
            prefix,
            incomplete: false,
            items: vec![],
        });
    }
    let mut left = MAX_SCANNED;
    let mut sources = vec![];
    let active = docs.get(&id).into_iter();
    let others = docs.iter().filter(|(k, _)| **k != id).map(|(_, d)| d);
    for d in active.chain(others) {
        if d.lang.lsp_id().is_none() || left == 0 {
            continue;
        }
        let n = d.rope.len_chars().min(left);
        left -= n;
        sources.push(d.rope.slice(..n).to_string());
    }
    let items = words::suggest(&prefix, &lexer::vocabulary(lang), &sources)
        .into_iter()
        .map(|w| Completion {
            label: w.text.clone(),
            insert: w.text,
            detail: if w.keyword { "keyword" } else { "" }.to_string(),
        })
        .collect();
    Ok(Completions {
        prefix,
        incomplete: false,
        items,
    })
}

#[derive(Serialize)]
struct Location {
    path: String,
    line: usize,
    col: usize,
}

#[tauri::command(async)]
fn lsp_definition(id: u32, state: State<'_, AppState>) -> Res<Option<Location>> {
    let lsp = current_lsp(&state)?;
    let (params, _, _) = at_cursor(id, &lsp, &state)?;
    let res = lsp.request("textDocument/definition", params, LSP_TIMEOUT)?;
    let loc = if res.is_array() {
        res.get(0).cloned().unwrap_or(Value::Null)
    } else {
        res
    };
    let uri = loc["uri"].as_str().or(loc["targetUri"].as_str());
    let range = if loc["range"].is_object() {
        &loc["range"]
    } else {
        &loc["targetSelectionRange"]
    };
    Ok(uri.and_then(path_from_uri).map(|p| Location {
        path: p.to_string_lossy().into_owned(),
        line: range["start"]["line"].as_u64().unwrap_or(0) as usize,
        col: range["start"]["character"].as_u64().unwrap_or(0) as usize,
    }))
}

#[tauri::command(async)]
fn lsp_hover(id: u32, state: State<'_, AppState>) -> Res<String> {
    let lsp = current_lsp(&state)?;
    let (params, _, _) = at_cursor(id, &lsp, &state)?;
    let res = lsp.request("textDocument/hover", params, LSP_TIMEOUT)?;
    Ok(res["contents"]["value"].as_str().unwrap_or("").to_string())
}

#[tauri::command(async)]
fn lsp_format(id: u32, state: State<'_, AppState>) -> Res<()> {
    let lsp = current_lsp(&state)?;
    lsp.sync(&state, id)?;
    let (params, rev) = {
        let docs = state.docs.lock().unwrap();
        let doc = docs.get(&id).ok_or("Document is closed")?;
        if doc.lsp_rev != Some(doc.rev) {
            return Err("The file changed while formatting".into());
        }
        (
            json!({
                "textDocument": {"uri": uri_from_path(&doc.path)},
                "options": {"tabSize": doc.tab, "insertSpaces": !doc.use_tabs},
            }),
            doc.rev,
        )
    };
    let res = lsp.request("textDocument/formatting", params, LSP_TIMEOUT)?;
    let n = |v: &Value| v.as_u64().unwrap_or(0) as usize;
    let edits = res
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
    let mut docs = state.docs.lock().unwrap();
    let doc = docs.get_mut(&id).ok_or("Document is closed")?;
    if doc.rev != rev {
        return Err("The file changed while formatting".into());
    }
    doc.apply_utf16_edits(edits);
    Ok(())
}

#[tauri::command]
fn doc_open_virtual(key: String, name: String, text: String, state: State<AppState>) -> Opened {
    let mut docs = state.docs.lock().unwrap();
    let path = std::path::PathBuf::from(&key);
    let existing = docs.iter().find(|(_, d)| d.path == path).map(|(id, _)| *id);
    let id = existing.unwrap_or_else(|| state.next_doc.fetch_add(1, Ordering::Relaxed));
    let mut doc = Doc::new(path, &text, None);
    doc.lang = lexer::Lang::Diff;
    doc.readonly = true;
    docs.insert(id, doc);
    Opened {
        id,
        path: key,
        name,
    }
}

#[tauri::command]
fn doc_apply_edits(
    id: u32,
    edits: Vec<(usize, usize, usize, usize, String)>,
    state: State<AppState>,
) -> Res<()> {
    let mut docs = state.docs.lock().unwrap();
    docs.get_mut(&id)
        .ok_or("Document is closed")?
        .apply_utf16_edits(edits);
    Ok(())
}

#[derive(Serialize)]
struct DiagRow {
    line: usize,
    col: usize,
    sev: usize,
    msg: String,
}

#[tauri::command]
fn doc_diags(id: u32, state: State<AppState>) -> Vec<DiagRow> {
    let docs = state.docs.lock().unwrap();
    let Some(d) = docs.get(&id) else {
        return vec![];
    };
    let mut rows: Vec<DiagRow> = d
        .diags
        .iter()
        .map(|x| DiagRow {
            line: x.sl,
            col: d.utf16_col(x.sl.min(d.rope.len_lines() - 1), x.sc),
            sev: x.sev,
            msg: x.msg.clone(),
        })
        .collect();
    rows.sort_by_key(|r| (r.sev, r.line));
    rows
}

#[tauri::command]
fn doc_probe(
    id: u32,
    row: usize,
    vcol: usize,
    state: State<AppState>,
) -> (Option<String>, Option<String>) {
    let docs = state.docs.lock().unwrap();
    docs.get(&id).map_or((None, None), |d| {
        (d.diag_at(row, vcol), d.word_at(row, vcol))
    })
}

#[tauri::command]
fn doc_next_problem(id: u32, state: State<AppState>) {
    let mut docs = state.docs.lock().unwrap();
    if let Some(d) = docs.get_mut(&id) {
        if let Some((l, c)) = d.next_problem() {
            d.goto_chars(l, c);
        }
    }
}

#[tauri::command(async)]
fn lsp_references(id: u32, state: State<'_, AppState>) -> Res<Vec<project::Hit>> {
    let lsp = current_lsp(&state)?;
    let (mut params, _, _) = at_cursor(id, &lsp, &state)?;
    params["context"] = json!({"includeDeclaration": true});
    let res = lsp.request("textDocument/references", params, Duration::from_secs(20))?;
    let mut files: HashMap<std::path::PathBuf, Vec<String>> = HashMap::new();
    let mut hits = vec![];
    for loc in res.as_array().map_or(&[][..], Vec::as_slice) {
        let Some(path) = loc["uri"].as_str().and_then(path_from_uri) else {
            continue;
        };
        let line = loc["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
        let lines = files.entry(path.clone()).or_insert_with(|| {
            fs::read_to_string(&path)
                .map(|t| t.lines().map(String::from).collect())
                .unwrap_or_default()
        });
        let shown = path.to_string_lossy().into_owned();
        hits.push(project::Hit {
            rel: shown.clone(),
            path: shown,
            line,
            col: loc["range"]["start"]["character"].as_u64().unwrap_or(0) as usize,
            text: lines
                .get(line)
                .map_or(String::new(), |l| l.trim().chars().take(160).collect()),
        });
    }
    Ok(hits)
}

#[tauri::command(async)]
fn lsp_rename(id: u32, name: String, state: State<'_, AppState>) -> Res<Vec<FileEdits>> {
    let lsp = current_lsp(&state)?;
    let (mut params, _, _) = at_cursor(id, &lsp, &state)?;
    params["newName"] = json!(name);
    let res = lsp.request("textDocument/rename", params, Duration::from_secs(20))?;
    Ok(parse_workspace_edit(&res))
}

#[derive(Serialize)]
struct Symbol {
    name: String,
    detail: String,
    kind: u64,
    depth: usize,
    line: usize,
    col: usize,
}

fn flatten_symbols(list: &[Value], depth: usize, out: &mut Vec<Symbol>) {
    for s in list {
        let start = &s["selectionRange"]["start"];
        out.push(Symbol {
            name: s["name"].as_str().unwrap_or("").to_string(),
            detail: s["detail"].as_str().unwrap_or("").to_string(),
            kind: s["kind"].as_u64().unwrap_or(0),
            depth,
            line: start["line"].as_u64().unwrap_or(0) as usize,
            col: start["character"].as_u64().unwrap_or(0) as usize,
        });
        flatten_symbols(
            s["children"].as_array().map_or(&[], Vec::as_slice),
            depth + 1,
            out,
        );
    }
}

#[tauri::command(async)]
fn lsp_symbols(id: u32, state: State<'_, AppState>) -> Res<Vec<Symbol>> {
    let lsp = current_lsp(&state)?;
    let (params, _, _) = at_cursor(id, &lsp, &state)?;
    let res = lsp.request(
        "textDocument/documentSymbol",
        json!({"textDocument": params["textDocument"]}),
        LSP_TIMEOUT,
    )?;
    let mut out = vec![];
    flatten_symbols(res.as_array().map_or(&[], Vec::as_slice), 0, &mut out);
    Ok(out)
}

#[tauri::command(async)]
fn lsp_workspace_symbols(query: String, state: State<'_, AppState>) -> Res<Vec<project::Hit>> {
    let lsp = current_lsp(&state)?;
    let res = lsp.request("workspace/symbol", json!({"query": query}), LSP_TIMEOUT)?;
    Ok(res
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .take(60)
        .filter_map(|s| {
            let path = s["location"]["uri"].as_str().and_then(path_from_uri)?;
            let shown = path.to_string_lossy().into_owned();
            Some(project::Hit {
                rel: shown.clone(),
                path: shown,
                line: s["location"]["range"]["start"]["line"].as_u64()? as usize,
                col: s["location"]["range"]["start"]["character"].as_u64()? as usize,
                text: format!(
                    "{}{}",
                    s["containerName"].as_str().unwrap_or(""),
                    s["name"]
                        .as_str()
                        .map_or(String::new(), |n| format!("::{n}"))
                )
                .trim_start_matches("::")
                .to_string(),
            })
        })
        .collect())
}

#[derive(Serialize)]
struct CodeAction {
    title: String,
    edit: Vec<FileEdits>,
    command: Option<Value>,
}

#[tauri::command(async)]
fn lsp_code_actions(id: u32, state: State<'_, AppState>) -> Res<Vec<CodeAction>> {
    let lsp = current_lsp(&state)?;
    let (params, _, _) = at_cursor(id, &lsp, &state)?;
    let diagnostics: Vec<Value> = {
        let docs = state.docs.lock().unwrap();
        let doc = docs.get(&id).ok_or("Document is closed")?;
        let (line, _) = doc.cursor_pos();
        doc.diags
            .iter()
            .filter(|d| d.sl <= line && line <= d.el)
            .map(|d| d.raw.clone())
            .collect()
    };
    let req = json!({
        "textDocument": params["textDocument"],
        "range": {"start": params["position"], "end": params["position"]},
        "context": {"diagnostics": diagnostics},
    });
    let res = lsp.request("textDocument/codeAction", req, LSP_TIMEOUT)?;
    Ok(res
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|a| CodeAction {
            title: a["title"].as_str().unwrap_or("").to_string(),
            edit: parse_workspace_edit(&a["edit"]),
            command: if a["command"].is_object() {
                Some(a["command"].clone())
            } else {
                a["arguments"].is_array().then(|| a.clone())
            },
        })
        .collect())
}

#[tauri::command(async)]
fn lsp_exec_command(command: Value, state: State<'_, AppState>) -> Res<()> {
    let lsp = current_lsp(&state)?;
    lsp.request(
        "workspace/executeCommand",
        json!({"command": command["command"], "arguments": command["arguments"]}),
        LSP_TIMEOUT,
    )
    .map(|_| ())
}

type Signature = (String, Option<(usize, usize)>);

#[tauri::command(async)]
fn lsp_signature(id: u32, state: State<'_, AppState>) -> Res<Option<Signature>> {
    let lsp = current_lsp(&state)?;
    let (params, _, _) = at_cursor(id, &lsp, &state)?;
    let res = lsp.request("textDocument/signatureHelp", params, LSP_TIMEOUT)?;
    let i = res["activeSignature"].as_u64().unwrap_or(0) as usize;
    let sig = &res["signatures"][i];
    let Some(label) = sig["label"].as_str() else {
        return Ok(None);
    };
    let param = res["activeParameter"].as_u64().unwrap_or(0) as usize;
    let range = sig["parameters"][param]["label"]
        .as_array()
        .and_then(|r| Some((r.first()?.as_u64()? as usize, r.get(1)?.as_u64()? as usize)));
    Ok(Some((label.to_string(), range)))
}

#[tauri::command(async)]
fn lsp_switch_header(id: u32, state: State<'_, AppState>) -> Res<Option<String>> {
    let lsp = current_lsp(&state)?;
    let (params, _, _) = at_cursor(id, &lsp, &state)?;
    let res = lsp.request(
        "textDocument/switchSourceHeader",
        json!({"uri": params["textDocument"]["uri"]}),
        LSP_TIMEOUT,
    )?;
    Ok(res
        .as_str()
        .and_then(path_from_uri)
        .map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command(async)]
fn lsp_semantic(id: u32, state: State<'_, AppState>) -> Res<()> {
    let lsp = current_lsp(&state)?;
    lsp.sync(&state, id)?;
    let (uri, rev) = {
        let docs = state.docs.lock().unwrap();
        let doc = docs.get(&id).ok_or("Document is closed")?;
        if doc.lang.lsp_id().is_none() {
            return Ok(());
        }
        (uri_from_path(&doc.path), doc.rev)
    };
    let res = lsp.request(
        "textDocument/semanticTokens/full",
        json!({"textDocument": {"uri": uri}}),
        LSP_TIMEOUT,
    )?;
    let tokens = lsp.decode_semantic(res["data"].as_array().map_or(&[], Vec::as_slice));
    let mut docs = state.docs.lock().unwrap();
    if let Some(doc) = docs.get_mut(&id).filter(|d| d.rev == rev) {
        doc.set_semantic(tokens);
    }
    Ok(())
}

#[tauri::command(async)]
fn git_status(root: String) -> Option<git::Status> {
    git::status(&root).ok()
}

#[tauri::command(async)]
fn git_stage(root: String, paths: Vec<String>) -> Res<()> {
    git::stage(&root, &paths)
}

#[tauri::command(async)]
fn git_unstage(root: String, paths: Vec<String>) -> Res<()> {
    git::unstage(&root, &paths)
}

#[tauri::command(async)]
fn git_discard(root: String, path: String, untracked: bool) -> Res<()> {
    git::discard(&root, &path, untracked)
}

#[tauri::command(async)]
fn git_commit(root: String, message: String, amend: bool) -> Res<String> {
    git::commit(&root, &message, amend)
}

#[tauri::command(async)]
fn git_diff(root: String, path: String, untracked: bool) -> String {
    git::diff(&root, &path, untracked)
}

#[tauri::command(async)]
fn git_log(root: String) -> Res<Vec<git::Commit>> {
    git::log(&root)
}

#[tauri::command(async)]
fn git_show(root: String, hash: String) -> Res<String> {
    git::show(&root, &hash)
}

#[tauri::command(async)]
fn git_branches(root: String) -> Res<Vec<String>> {
    git::branches(&root)
}

#[tauri::command(async)]
fn git_clone(url: String, parent: String) -> Res<String> {
    git::clone(&url, &parent)
}

#[tauri::command(async)]
fn git_checkout(root: String, name: String, create: bool) -> Res<()> {
    git::checkout(&root, &name, create)
}

#[tauri::command(async)]
fn git_refresh(id: u32, state: State<'_, AppState>) {
    let path = state
        .docs
        .lock()
        .unwrap()
        .get(&id)
        .filter(|d| d.mtime.is_some())
        .map(|d| d.path.clone());
    let Some(path) = path else { return };
    let marks = git::line_marks(&path);
    if let Some(d) = state.docs.lock().unwrap().get_mut(&id) {
        d.git_marks = marks;
    }
}

#[tauri::command]
fn term_open(
    cwd: String,
    cols: usize,
    rows: usize,
    app: AppHandle,
    state: State<AppState>,
) -> Res<u32> {
    let id = state.next_term.fetch_add(1, Ordering::Relaxed);
    let t = term::Term::spawn(app, id, &cwd, cols.max(2), rows.max(2))?;
    state.terms.lock().unwrap().insert(id, t);
    Ok(id)
}

#[tauri::command]
fn term_view(id: u32, offset: usize, full: bool, state: State<AppState>) -> Res<term::TermView> {
    let t = state
        .terms
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("Terminal is closed")?;
    t.notified.store(false, Ordering::Release);
    let v = t.inner.lock().unwrap().grid.view(offset, full);
    Ok(v)
}

#[tauri::command]
fn term_input(id: u32, data: String, state: State<AppState>) -> Res<()> {
    let t = state
        .terms
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("Terminal is closed")?;
    t.write(data.as_bytes())
}

#[tauri::command]
fn term_resize(id: u32, cols: usize, rows: usize, state: State<AppState>) {
    if let Some(t) = state.terms.lock().unwrap().get(&id).cloned() {
        t.resize(cols.max(2), rows.max(2));
    }
}

#[tauri::command(async)]
fn term_info(id: u32, state: State<'_, AppState>) -> Res<term::TermInfo> {
    let t = state
        .terms
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("Terminal is closed")?;
    Ok(t.info())
}

#[tauri::command(async)]
fn term_close(id: u32, state: State<AppState>) {
    if let Some(t) = state.terms.lock().unwrap().remove(&id) {
        t.kill();
    }
}

#[cfg(unix)]
#[tauri::command(async)]
fn dbg_start(
    program: String,
    cwd: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Res<String> {
    let old = state.dbg.lock().unwrap().take();
    if let Some(old) = old {
        old.stop();
    }
    let d = debug::Debugger::spawn(app, &program, &cwd)?;
    let tty = d.tty_name.clone();
    *state.dbg.lock().unwrap() = Some(d);
    Ok(tty)
}

#[cfg(unix)]
#[tauri::command(async)]
fn dbg_cmd(cmd: String, state: State<'_, AppState>) -> Res<Value> {
    let d = state
        .dbg
        .lock()
        .unwrap()
        .clone()
        .ok_or("The debugger is not running")?;
    d.command(&cmd)
}

#[cfg(unix)]
#[tauri::command]
fn dbg_input(text: String, state: State<AppState>) -> Res<()> {
    let d = state
        .dbg
        .lock()
        .unwrap()
        .clone()
        .ok_or("The debugger is not running")?;
    d.send_input(&text)
}

#[cfg(unix)]
#[tauri::command(async)]
fn dbg_stop(state: State<'_, AppState>) {
    let d = state.dbg.lock().unwrap().take();
    if let Some(d) = d {
        d.stop();
    }
}

#[cfg(target_os = "linux")]
fn tune_webkit() {
    if std::env::var_os("JSC_useJIT").is_none() {
        std::env::set_var("JSC_useJIT", "0");
    }
}

fn main() {
    #[cfg(windows)]
    if let Some(code) = cli::handle_flag() {
        std::process::exit(code);
    }

    #[cfg(target_os = "linux")]
    tune_webkit();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            tools::extend_path(app.handle());
            Ok(())
        })
        .manage(AppState {
            docs: Mutex::new(HashMap::new()),
            next_doc: AtomicU32::new(1),
            lsp: Mutex::new(None),
            runner: Arc::new(Runner::default()),
            terms: Mutex::new(HashMap::new()),
            next_term: AtomicU32::new(1),
            #[cfg(unix)]
            dbg: Mutex::new(None),
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("close-requested", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            #[cfg(feature = "selftest")]
            selftest_log,
            term_open,
            term_view,
            term_input,
            term_resize,
            term_info,
            term_close,
            #[cfg(unix)]
            dbg_start,
            #[cfg(unix)]
            dbg_cmd,
            #[cfg(unix)]
            dbg_input,
            #[cfg(unix)]
            dbg_stop,
            #[cfg(any(unix, windows))]
            install_cli,
            syntax_check,
            word_complete,
            line_suggest,
            ai_chat,
            ai_cancel,
            ai_models,
            doc_context,
            tools_status,
            tools_install,
            tool_available,
            clipboard_read,
            clipboard_write,
            doc_format,
            app_version,
            update_check,
            update_install,
            lsp_semantic,
            doc_open_virtual,
            doc_apply_edits,
            doc_diags,
            doc_probe,
            doc_next_problem,
            lsp_references,
            lsp_rename,
            lsp_symbols,
            lsp_workspace_symbols,
            lsp_code_actions,
            lsp_exec_command,
            lsp_signature,
            lsp_switch_header,
            git_status,
            git_stage,
            git_unstage,
            git_discard,
            git_commit,
            git_diff,
            git_log,
            git_show,
            git_branches,
            git_checkout,
            git_clone,
            git_refresh,
            app_ready,
            app_quit,
            initial_path,
            doc_open,
            doc_close,
            doc_apply,
            doc_save,
            doc_search,
            docs_refresh,
            list_dir,
            list_files,
            grep,
            create_project,
            fs_new_file,
            fs_new_dir,
            fs_rename,
            fs_delete,
            path_exists,
            sys_info,
            task_start,
            task_stdin,
            task_kill,
            run_start,
            term_note,
            cmake_prepare,
            cmake_targets,
            make_dirs,
            unbuffered_helper,
            lsp_start,
            lsp_stop,
            lsp_sync,
            lsp_complete,
            lsp_definition,
            lsp_hover,
            lsp_format
        ])
        .run(tauri::generate_context!())
        .expect("failed to start clight");
}
