use crate::runner::{clean_env, lower_priority};
use crate::tools;
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const IDLE: Duration = Duration::from_secs(300);
const START_TIMEOUT: Duration = Duration::from_secs(90);

struct Running {
    child: Child,
    port: u16,
    used: Instant,
    model: &'static str,
    ready: bool,
}

static ENGINE: Mutex<Option<Running>> = Mutex::new(None);
static STARTING: Mutex<()> = Mutex::new(());
static WATCHER: Once = Once::new();

#[derive(Serialize, Clone)]
struct Status {
    id: u32,
    text: String,
}

/// The tool id of a model size: "large" is the bigger one, anything else the small one.
pub fn model_id(size: &str) -> &'static str {
    if size == "large" {
        "ai-model-large"
    } else {
        "ai-model"
    }
}

pub fn installed(app: &AppHandle, size: &str) -> bool {
    tools::tool_path(app, "ai-engine").is_some() && tools::tool_path(app, model_id(size)).is_some()
}

/// Takes a model out, and the engine too when no model is left.
pub fn remove(app: &AppHandle, size: &str) {
    stop();
    tools::remove(app, model_id(size));
    if tools::tool_path(app, "ai-model").is_none()
        && tools::tool_path(app, "ai-model-large").is_none()
    {
        tools::remove(app, "ai-engine");
    }
}

fn alive(model: &str) -> Option<u16> {
    let mut guard = ENGINE.lock().unwrap();
    let running = guard.as_mut()?;
    if running.model != model || !running.ready {
        let mut old = guard.take()?;
        let _ = old.child.kill();
        let _ = old.child.wait();
        return None;
    }
    match running.child.try_wait() {
        Ok(None) => {
            running.used = Instant::now();
            Some(running.port)
        }
        _ => {
            *guard = None;
            None
        }
    }
}

/// Keeps the engine from being stopped while an answer is still being written.
pub fn touch() {
    if let Some(r) = ENGINE.lock().unwrap().as_mut() {
        r.used = Instant::now();
    }
}

pub fn stop() {
    let running = ENGINE.lock().unwrap().take();
    if let Some(mut r) = running {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}

fn healthy(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    if s.write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut head = [0u8; 12];
    let n = s.read(&mut head).unwrap_or(0);
    head[..n].ends_with(b" 200")
        || head[..n].starts_with(b"HTTP/1.1 200")
        || head[..n].starts_with(b"HTTP/1.0 200")
}

fn log_tail(path: &std::path::Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().rev().take(3).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" ")
}

fn base(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn start(app: &AppHandle, id: u32, size: &str) -> Result<String, String> {
    let _only_one = STARTING.lock().unwrap();
    let model_tool = model_id(size);
    if let Some(port) = alive(model_tool) {
        return Ok(base(port));
    }
    let exe = tools::tool_path(app, "ai-engine").ok_or("The AI isn't installed")?;
    let model = tools::tool_path(app, model_tool).ok_or("That AI model isn't installed")?;
    let _ = app.emit(
        "ai-status",
        Status {
            id,
            text: "Starting the AI…".into(),
        },
    );
    let port = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map_err(|e| e.to_string())?
        .port();
    let dir = exe.parent().ok_or("The engine folder is missing")?;
    let log_path = model
        .parent()
        .and_then(|p| p.parent())
        .map_or_else(|| dir.join("ai-engine.log"), |t| t.join("ai-engine.log"));
    let log = File::create(&log_path).map_err(|e| e.to_string())?;
    let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
    let mut cmd = Command::new(&exe);
    clean_env(&mut cmd);
    cmd.arg("-m")
        .arg(&model)
        .args(["--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .args(["-c", "4096", "-np", "1", "-t"])
        .arg(threads.to_string())
        .arg("--no-webui")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log));
    #[cfg(unix)]
    {
        let old = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        cmd.env("LD_LIBRARY_PATH", format!("{}:{old}", dir.display()));
    }
    lower_priority(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("Couldn't start the small AI: {e}"))?;
    // Registered at once, so quitting while it loads still stops it.
    *ENGINE.lock().unwrap() = Some(Running {
        child,
        port,
        used: Instant::now(),
        model: model_tool,
        ready: false,
    });
    let began = Instant::now();
    loop {
        {
            let mut guard = ENGINE.lock().unwrap();
            let Some(running) = guard.as_mut() else {
                return Err("The AI was stopped".into());
            };
            if let Ok(Some(status)) = running.child.try_wait() {
                *guard = None;
                return Err(format!(
                    "The small AI stopped while starting ({status}). {}",
                    log_tail(&log_path)
                ));
            }
        }
        if healthy(port) {
            if let Some(running) = ENGINE.lock().unwrap().as_mut() {
                running.ready = true;
                running.used = Instant::now();
            }
            break;
        }
        if began.elapsed() > START_TIMEOUT {
            stop();
            return Err("The small AI took too long to start".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    WATCHER.call_once(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(20));
            let idle = ENGINE
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|r| r.used.elapsed() > IDLE);
            if idle {
                stop();
            }
        });
    });
    Ok(base(port))
}

/// Address of the built-in engine, started if it isn't running.
pub async fn ensure(app: &AppHandle, id: u32, size: &str) -> Result<String, String> {
    let app = app.clone();
    let size = size.to_string();
    tauri::async_runtime::spawn_blocking(move || start(&app, id, &size))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_port_is_not_healthy() {
        let port = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(!healthy(port));
    }

    #[test]
    fn a_server_answering_200_is_healthy_and_503_is_not() {
        for (reply, want) in [
            ("HTTP/1.1 200 OK\r\n\r\n", true),
            ("HTTP/1.1 503 Loading\r\n\r\n", false),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let t = std::thread::spawn(move || {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 64];
                let _ = s.read(&mut buf);
                s.write_all(reply.as_bytes()).unwrap();
            });
            assert_eq!(healthy(port), want);
            t.join().unwrap();
        }
    }
}
