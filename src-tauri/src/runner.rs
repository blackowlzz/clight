use serde::Serialize;
use std::io::{Read, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[cfg(unix)]
pub type RawFd = std::os::fd::RawFd;

const MAX_QUEUED: usize = 512 * 1024;

#[derive(Default)]
pub struct Runner {
    current: Mutex<Option<Running>>,
}

struct Running {
    pid: u32,
    stdin: Option<ChildStdin>,
}

#[derive(Serialize, Clone)]
struct Output {
    s: String,
}

pub fn clean_env(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.env_remove("JSC_useJIT");
    let Some(appdir) = std::env::var_os("APPDIR") else {
        return;
    };
    let appdir = appdir.to_string_lossy().into_owned();
    for (k, v) in std::env::vars_os() {
        let value = v.to_string_lossy();
        if !value.contains(&appdir) {
            continue;
        }
        let kept: Vec<&str> = value
            .split(':')
            .filter(|p| !p.is_empty() && !p.contains(&appdir))
            .collect();
        if kept.is_empty() {
            cmd.env_remove(&k);
        } else {
            cmd.env(&k, kept.join(":"));
        }
    }
    for k in ["APPDIR", "APPIMAGE", "ARGV0", "OWD", "GDK_BACKEND"] {
        cmd.env_remove(k);
    }
}

pub fn lower_priority(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, 10);
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    let _ = cmd;
}

#[derive(Default)]
struct Queue {
    bytes: Vec<u8>,
    dropped: bool,
}

fn pump(
    mut from: impl Read + Send + 'static,
    queue: Arc<Mutex<Queue>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = from.read(&mut buf) {
            if n == 0 {
                break;
            }
            let mut q = queue.lock().unwrap();
            if q.bytes.len() + n > MAX_QUEUED {
                q.dropped = true;
            } else {
                q.bytes.extend_from_slice(&buf[..n]);
            }
        }
    })
}

fn take_text(q: &mut Queue, final_flush: bool) -> String {
    let keep = match std::str::from_utf8(&q.bytes) {
        Ok(_) => 0,
        Err(e) if e.error_len().is_none() && !final_flush => q.bytes.len() - e.valid_up_to(),
        Err(_) => 0,
    };
    let cut = q.bytes.len() - keep;
    let rest = q.bytes.split_off(cut);
    let bytes = std::mem::replace(&mut q.bytes, rest);
    let mut s = String::from_utf8_lossy(&bytes).into_owned();
    if std::mem::take(&mut q.dropped) {
        s.push_str("\n… output too fast, some lines were dropped …\n");
    }
    s
}

impl Runner {
    pub fn start(
        self: &Arc<Self>,
        app: AppHandle,
        program: &str,
        args: &[String],
        cwd: &str,
        env: &[(String, String)],
        background: bool,
    ) -> Result<(), String> {
        let mut slot = self.current.lock().unwrap();
        if slot.is_some() {
            return Err("Another task is still running".into());
        }
        let mut cmd = Command::new(program);
        clean_env(&mut cmd);
        cmd.args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        if background {
            lower_priority(&mut cmd);
        }
        let mut child = cmd.spawn().map_err(|e| format!("{program}: {e}"))?;
        let stdin = child.stdin.take();
        let out = child.stdout.take().ok_or("no stdout")?;
        let err = child.stderr.take().ok_or("no stderr")?;
        *slot = Some(Running {
            pid: child.id(),
            stdin,
        });
        drop(slot);

        let queue = Arc::new(Mutex::new(Queue::default()));
        let readers = [pump(out, queue.clone()), pump(err, queue.clone())];
        let done = Arc::new(AtomicBool::new(false));

        let (q2, done2, app2) = (queue.clone(), done.clone(), app.clone());
        let flusher = std::thread::spawn(move || {
            while !done2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(60));
                let s = take_text(&mut q2.lock().unwrap(), false);
                if !s.is_empty() {
                    let _ = app2.emit("task-out", Output { s });
                }
            }
        });

        let me = self.clone();
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code());
            *me.current.lock().unwrap() = None;
            for r in readers {
                let _ = r.join();
            }
            done.store(true, Ordering::Relaxed);
            let _ = flusher.join();
            let s = take_text(&mut queue.lock().unwrap(), true);
            if !s.is_empty() {
                let _ = app.emit("task-out", Output { s });
            }
            let _ = app.emit("task-done", code);
        });
        Ok(())
    }

    pub fn write_stdin(&self, text: &str) -> Result<(), String> {
        let mut slot = self.current.lock().unwrap();
        let stdin = slot
            .as_mut()
            .and_then(|r| r.stdin.as_mut())
            .ok_or("Nothing is running")?;
        stdin
            .write_all(text.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|e| e.to_string())
    }

    pub fn kill(&self) {
        let pid = self.current.lock().unwrap().as_ref().map(|r| r.pid);
        let Some(pid) = pid else { return };
        #[cfg(unix)]
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        #[cfg(not(unix))]
        {
            let mut taskkill = Command::new("taskkill");
            clean_env(&mut taskkill);
            let _ = taskkill
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_utf8_character_waits_for_its_other_half() {
        let mut q = Queue {
            bytes: "aè".as_bytes()[..2].to_vec(),
            dropped: false,
        };
        assert_eq!(take_text(&mut q, false), "a");
        assert_eq!(q.bytes.len(), 1);
        q.bytes.extend_from_slice(&"è".as_bytes()[1..]);
        assert_eq!(take_text(&mut q, false), "è");
        assert!(q.bytes.is_empty());
    }

    #[test]
    fn invalid_bytes_do_not_stall_the_stream() {
        let mut q = Queue {
            bytes: vec![b'a', 0xFF, b'b'],
            dropped: false,
        };
        assert_eq!(take_text(&mut q, false), "a\u{fffd}b");
    }
}
