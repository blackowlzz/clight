use serde::Serialize;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
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

/// Starts a program that must not outlive this one. On Linux the kernel ends it when this
/// process dies, even when that is a crash or a kill. The signal follows the thread that started
/// the program, so it is started from a thread that lasts as long as the process.
pub fn spawn_tied(cmd: Command) -> std::io::Result<Child> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        use std::sync::{mpsc, OnceLock};
        type Job = (Command, mpsc::Sender<std::io::Result<Child>>);
        static SPAWNER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
        let sender = SPAWNER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::spawn(move || {
                for (mut cmd, back) in rx {
                    unsafe {
                        cmd.pre_exec(|| {
                            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                            Ok(())
                        });
                    }
                    let _ = back.send(cmd.spawn());
                }
            });
            Mutex::new(tx)
        });
        let (back, got) = mpsc::channel();
        let gone = || std::io::Error::other("The process starter is gone");
        sender.lock().unwrap_or_else(std::sync::PoisonError::into_inner).send((cmd, back)).map_err(|_| gone())?;
        got.recv().map_err(|_| gone())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut cmd = cmd;
        cmd.spawn()
    }
}

/// Ends what the previous run left behind, if it was killed before it could stop its helpers.
pub fn end_leftover(pid_file: &std::path::Path, exe_name: &str, home: &std::path::Path) {
    let Some(pid) = std::fs::read_to_string(pid_file).ok().and_then(|t| t.trim().parse::<u32>().ok()) else {
        return;
    };
    let _ = std::fs::remove_file(pid_file);
    // Only a process that still is that program: the number may have been given to another.
    #[cfg(unix)]
    {
        let same = std::fs::read_link(format!("/proc/{pid}/exe")).ok().and_then(|p| p.file_name().map(|n| n == exe_name)).unwrap_or(false);
        // And one that was started by this app, on a model of this app: its command line says so.
        let ours = std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| String::from_utf8_lossy(&c).contains(&*home.to_string_lossy()));
        if same && ours {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = home;
        let mut list = Command::new("tasklist");
        clean_env(&mut list);
        let Ok(out) = list.args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output() else { return };
        let name = format!("{exe_name}.exe").to_lowercase();
        if String::from_utf8_lossy(&out.stdout).to_lowercase().contains(&name) {
            let mut kill = Command::new("taskkill");
            clean_env(&mut kill);
            let _ = kill.args(["/PID", &pid.to_string(), "/F"]).output();
        }
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

#[cfg(all(test, target_os = "linux"))]
mod leftover_tests {
    use super::*;

    #[test]
    fn a_leftover_helper_is_ended_only_when_it_is_the_one_we_started() {
        let dir = std::env::temp_dir().join(format!("leftover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = Command::new("sleep").arg("4321").spawn().unwrap();
        // Until it has become `sleep`, /proc still shows the program that started it. On a busy
        // machine (the whole test suite at once) that can take a few seconds.
        for _ in 0..1000 {
            let exe = std::fs::read_link(format!("/proc/{}/exe", child.id())).ok();
            if exe.is_some_and(|p| p.file_name().is_some_and(|n| n == "sleep")) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let pid_file = dir.join("helper.pid");
        std::fs::write(&pid_file, child.id().to_string()).unwrap();
        // Another program's folder: left alone.
        end_leftover(&pid_file, "sleep", std::path::Path::new("/somewhere/else"));
        assert!(child.try_wait().unwrap().is_none());
        std::fs::write(&pid_file, child.id().to_string()).unwrap();
        // Another name: left alone.
        end_leftover(&pid_file, "llama-server", std::path::Path::new("4321"));
        assert!(child.try_wait().unwrap().is_none());
        std::fs::write(&pid_file, child.id().to_string()).unwrap();
        end_leftover(&pid_file, "sleep", std::path::Path::new("4321"));
        let mut ended = false;
        for _ in 0..1000 {
            if child.try_wait().unwrap().is_some() {
                ended = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !ended {
            let _ = child.kill();
        }
        assert!(ended, "the leftover was not ended");
        assert!(!pid_file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
