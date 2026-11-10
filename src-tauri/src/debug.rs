use serde_json::{json, Map, Value};

pub fn parse_results(s: &str) -> Value {
    let mut p = P {
        b: s.as_bytes(),
        i: 0,
    };
    let mut map = Map::new();
    p.fields(&mut map, 0);
    Value::Object(map)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> u8 {
        self.b.get(self.i).copied().unwrap_or(0)
    }

    fn fields(&mut self, map: &mut Map<String, Value>, close: u8) {
        while self.i < self.b.len() && self.peek() != close {
            if self.peek() == b',' {
                self.i += 1;
                continue;
            }
            let start = self.i;
            while self.i < self.b.len() && self.peek() != b'=' {
                self.i += 1;
            }
            let name = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
            self.i += 1;
            let v = self.value();
            map.insert(name, v);
        }
    }

    fn value(&mut self) -> Value {
        match self.peek() {
            b'"' => Value::String(self.string()),
            b'{' => {
                self.i += 1;
                let mut m = Map::new();
                self.fields(&mut m, b'}');
                self.i += 1;
                Value::Object(m)
            }
            b'[' => {
                self.i += 1;
                let mut items = vec![];
                while self.i < self.b.len() && self.peek() != b']' {
                    match self.peek() {
                        b',' => self.i += 1,
                        b'"' | b'{' | b'[' => items.push(self.value()),
                        _ => {
                            while self.i < self.b.len() && self.peek() != b'=' {
                                self.i += 1;
                            }
                            self.i += 1;
                            items.push(self.value());
                        }
                    }
                }
                self.i += 1;
                Value::Array(items)
            }
            _ => {
                self.i += 1;
                Value::Null
            }
        }
    }

    fn string(&mut self) -> String {
        self.i += 1;
        let mut out = vec![];
        while self.i < self.b.len() && self.peek() != b'"' {
            let c = self.peek();
            self.i += 1;
            if c != b'\\' {
                out.push(c);
                continue;
            }
            let e = self.peek();
            self.i += 1;
            match e {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'0'..=b'7' => {
                    let mut v = (e - b'0') as u32;
                    for _ in 0..2 {
                        if (b'0'..=b'7').contains(&self.peek()) {
                            v = v * 8 + (self.peek() - b'0') as u32;
                            self.i += 1;
                        }
                    }
                    out.push(v as u8);
                }
                e => out.push(e),
            }
        }
        self.i += 1;
        String::from_utf8_lossy(&out).into_owned()
    }
}

#[derive(Debug, PartialEq)]
pub enum Record {
    Result {
        token: Option<u64>,
        class: String,
        data: Value,
    },
    Async {
        kind: &'static str,
        class: String,
        data: Value,
    },
    Stream(String),
    Other,
}

pub fn parse_line(line: &str) -> Record {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let token = line[..digits].parse().ok();
    let rest = &line[digits..];
    let Some(mark) = rest.bytes().next() else {
        return Record::Other;
    };
    let body = &rest[1..];
    let split = |b: &str| -> (String, Value) {
        match b.split_once(',') {
            Some((c, r)) => (c.to_string(), parse_results(r)),
            None => (b.to_string(), json!({})),
        }
    };
    match mark {
        b'^' => {
            let (class, data) = split(body);
            Record::Result { token, class, data }
        }
        b'*' => {
            let (class, data) = split(body);
            Record::Async {
                kind: "exec",
                class,
                data,
            }
        }
        b'=' => {
            let (class, data) = split(body);
            Record::Async {
                kind: "notify",
                class,
                data,
            }
        }
        b'~' | b'@' | b'&' if body.starts_with('"') => Record::Stream(
            P {
                b: body.as_bytes(),
                i: 0,
            }
            .string(),
        ),
        _ => Record::Other,
    }
}

pub use session::Debugger;

mod session {
    use super::*;
    use crate::term::{open_pty, tty_path};
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::{AppHandle, Emitter};

    pub struct Debugger {
        stdin: Mutex<ChildStdin>,
        child: Mutex<Child>,
        next: AtomicU64,
        pending: Mutex<HashMap<u64, Sender<Result<Value, String>>>>,
        tty: Mutex<File>,
        pub tty_name: String,
        _slave: File,
    }

    impl Debugger {
        pub fn spawn(app: AppHandle, program: &str, cwd: &str) -> Result<Arc<Debugger>, String> {
            let (master, slave) = open_pty(120, 40)?;
            let tty_name = tty_path(&slave)?;
            let mut cmd = Command::new("gdb");
            crate::runner::clean_env(&mut cmd);
            let mut child = cmd
                .args(["-q", "-nh", "--interpreter=mi3", program])
                .current_dir(cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("gdb: {e}"))?;
            let stdin = child.stdin.take().ok_or("gdb: no stdin")?;
            let stdout = child.stdout.take().ok_or("gdb: no stdout")?;
            let mut out = master.try_clone().map_err(|e| e.to_string())?;
            let d = Arc::new(Debugger {
                stdin: Mutex::new(stdin),
                child: Mutex::new(child),
                next: AtomicU64::new(1),
                pending: Mutex::new(HashMap::new()),
                tty: Mutex::new(master),
                tty_name,
                _slave: slave,
            });

            let app2 = app.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = out.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let _ = app2.emit("dbg-out", String::from_utf8_lossy(&buf[..n]).into_owned());
                }
            });

            let me = d.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    match r.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    match parse_line(line.trim_end()) {
                        Record::Result {
                            token: Some(t),
                            class,
                            data,
                        } => {
                            if let Some(tx) = me.pending.lock().unwrap().remove(&t) {
                                let _ = tx.send(if class == "error" {
                                    Err(data["msg"].as_str().unwrap_or("gdb error").to_string())
                                } else {
                                    Ok(data)
                                });
                            }
                        }
                        Record::Async { kind, class, data } => {
                            let _ = app
                                .emit("dbg", json!({"kind": kind, "class": class, "data": data}));
                        }
                        Record::Stream(text) => {
                            let _ = app.emit("dbg", json!({"kind": "console", "text": text}));
                        }
                        _ => {}
                    }
                }
                me.pending.lock().unwrap().clear();
                let _ = app.emit("dbg", json!({"kind": "exit"}));
            });
            Ok(d)
        }

        pub fn command(&self, cmd: &str) -> Result<Value, String> {
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = channel();
            self.pending.lock().unwrap().insert(id, tx);
            {
                let mut w = self.stdin.lock().unwrap();
                writeln!(w, "{id}{cmd}")
                    .and_then(|_| w.flush())
                    .map_err(|e| e.to_string())?;
            }
            let res = rx
                .recv_timeout(Duration::from_secs(30))
                .map_err(|_| "gdb did not answer".to_string());
            self.pending.lock().unwrap().remove(&id);
            res?
        }

        pub fn send_input(&self, text: &str) -> Result<(), String> {
            self.tty
                .lock()
                .unwrap()
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())
        }

        pub fn stop(&self) {
            let mut c = self.child.lock().unwrap();
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_stop_record() {
        let line = r#"*stopped,reason="breakpoint-hit",bkptno="1",frame={addr="0x1",func="main",args=[],file="a.c",fullname="/x/a.c",line="4"},thread-id="1""#;
        let Record::Async { kind, class, data } = parse_line(line) else {
            panic!()
        };
        assert_eq!((kind, class.as_str()), ("exec", "stopped"));
        assert_eq!(data["frame"]["line"], "4");
        assert_eq!(data["frame"]["args"], json!([]));
    }

    #[test]
    fn named_list_items_become_plain_items() {
        let line = r#"12^done,stack=[frame={level="0",func="a"},frame={level="1",func="b"}]"#;
        let Record::Result { token, class, data } = parse_line(line) else {
            panic!()
        };
        assert_eq!(token, Some(12));
        assert_eq!(class, "done");
        assert_eq!(data["stack"][1]["func"], "b");
    }

    #[test]
    fn strings_unescape_including_utf8_octal() {
        assert_eq!(
            parse_line(r#"~"a\n\"b\" \303\250""#),
            Record::Stream("a\n\"b\" è".into())
        );
    }

    #[test]
    fn errors_keep_their_message() {
        let Record::Result { class, data, .. } =
            parse_line(r#"3^error,msg="No symbol \"x\" in current context.""#)
        else {
            panic!()
        };
        assert_eq!(class, "error");
        assert_eq!(data["msg"], "No symbol \"x\" in current context.");
    }

    #[test]
    fn prompt_is_ignored() {
        assert_eq!(parse_line("(gdb) "), Record::Other);
    }
}
