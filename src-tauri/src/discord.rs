//! Discord Rich Presence. It talks to the Discord app running on this computer over its local
//! socket (a Unix socket, or a named pipe on Windows), so nothing leaves the machine from here.
//! If Discord isn't running, nothing happens, and it connects as soon as Discord shows up.

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The id of the application made at discord.com/developers. Its name is what Discord shows
/// after "Playing". While it's empty the presence stays off.
const CLIENT_ID: &str = "1570709799635517513";
/// The name of an image uploaded under Rich Presence > Art Assets of that application.
const LOGO: &str = "clight";
/// The button under the activity: where people who see it can get the app.
const BUTTON: (&str, &str) = ("Get clight", "https://github.com/blackowlzz/clight");
const NAME: &str = "clight";

/// Discord allows about five updates every twenty seconds.
const MIN_GAP: Duration = Duration::from_secs(4);
const RETRY_GAP: Duration = Duration::from_secs(10);
const MAX_FRAME: usize = 64 * 1024;

#[derive(Deserialize, Clone, PartialEq, Debug)]
pub struct Activity {
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub state: String,
    /// When the elapsed time shown on the profile starts, in seconds since 1970.
    pub start: Option<i64>,
}

static TX: OnceLock<Mutex<Sender<Option<Activity>>>> = OnceLock::new();

/// Shows this activity, or removes the presence with `None`. It returns at once.
pub fn set(activity: Option<Activity>) {
    if CLIENT_ID.is_empty() {
        return;
    }
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::spawn(move || run(rx, CLIENT_ID, open));
        Mutex::new(tx)
    });
    let _ = tx.lock().unwrap().send(activity);
}

fn run(rx: std::sync::mpsc::Receiver<Option<Activity>>, client_id: &str, open: impl Fn() -> Option<Box<dyn Stream>>) {
    let mut conn: Option<Conn<Box<dyn Stream>>> = None;
    let mut want: Option<Activity> = None;
    // What Discord has on this connection. The outer `None` means nothing was sent yet.
    let mut shown: Option<Option<Activity>> = None;
    let mut last_try: Option<Instant> = None;
    let mut last_send: Option<Instant> = None;
    loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(a) => {
                want = a;
                while let Ok(a) = rx.try_recv() {
                    want = a;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if shown.as_ref() == Some(&want) || last_send.is_some_and(|t| t.elapsed() < MIN_GAP) {
            continue;
        }
        if conn.is_none() {
            if want.is_none() {
                shown = Some(None);
                continue;
            }
            if last_try.is_some_and(|t| t.elapsed() < RETRY_GAP) {
                continue;
            }
            last_try = Some(Instant::now());
            conn = open().and_then(|s| Conn::start(s, client_id).ok());
            shown = None;
        }
        if let Some(c) = conn.as_mut() {
            last_send = Some(Instant::now());
            match c.activity(want.as_ref()) {
                Ok(()) => shown = Some(want.clone()),
                Err(_) => {
                    conn = None;
                    shown = None;
                }
            }
        }
    }
}

pub trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

#[cfg(unix)]
fn open() -> Option<Box<dyn Stream>> {
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    let mut bases: Vec<PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    bases.push("/tmp".into());
    let mut dirs = Vec::new();
    for b in &bases {
        // The Flatpak and Snap builds keep their socket in a folder of its own.
        dirs.push(b.clone());
        dirs.push(b.join("app/com.discordapp.Discord"));
        dirs.push(b.join("app/com.discordapp.DiscordCanary"));
        dirs.push(b.join("snap.discord"));
    }
    for d in dirs {
        for i in 0..10 {
            if let Ok(s) = UnixStream::connect(d.join(format!("discord-ipc-{i}"))) {
                let t = Some(Duration::from_secs(5));
                let _ = s.set_read_timeout(t);
                let _ = s.set_write_timeout(t);
                return Some(Box::new(s));
            }
        }
    }
    None
}

#[cfg(windows)]
fn open() -> Option<Box<dyn Stream>> {
    for i in 0..10 {
        let pipe = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}"));
        if let Ok(f) = pipe {
            return Some(Box::new(f));
        }
    }
    None
}

#[cfg(not(any(unix, windows)))]
fn open() -> Option<Box<dyn Stream>> {
    None
}

const HANDSHAKE: u32 = 0;
const FRAME: u32 = 1;
const CLOSE: u32 = 2;
const PING: u32 = 3;
const PONG: u32 = 4;

pub struct Conn<S: Read + Write> {
    s: S,
    nonce: u64,
}

fn write_frame(s: &mut impl Write, op: u32, body: &Value) -> io::Result<()> {
    let data = body.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + data.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    s.write_all(&out)?;
    s.flush()
}

fn read_frame(s: &mut impl Read) -> io::Result<(u32, Value)> {
    let mut head = [0u8; 8];
    s.read_exact(&mut head)?;
    let op = u32::from_le_bytes(head[..4].try_into().unwrap());
    let len = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut data = vec![0u8; len];
    s.read_exact(&mut data)?;
    Ok((op, serde_json::from_slice(&data).unwrap_or(Value::Null)))
}

impl<S: Read + Write> Conn<S> {
    pub fn start(mut s: S, client_id: &str) -> io::Result<Self> {
        write_frame(&mut s, HANDSHAKE, &json!({ "v": 1, "client_id": client_id }))?;
        let (op, body) = read_frame(&mut s)?;
        if op != FRAME || body["evt"] != "READY" {
            return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "Discord refused the handshake"));
        }
        Ok(Conn { s, nonce: 0 })
    }

    pub fn activity(&mut self, a: Option<&Activity>) -> io::Result<()> {
        self.nonce += 1;
        let body = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": a.map(payload) },
            "nonce": self.nonce.to_string(),
        });
        write_frame(&mut self.s, FRAME, &body)?;
        // Discord answers every command; read until that answer so the socket never fills up.
        loop {
            let (op, reply) = read_frame(&mut self.s)?;
            match op {
                PING => write_frame(&mut self.s, PONG, &reply)?,
                CLOSE => return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Discord closed the connection")),
                _ if reply["nonce"].as_str().and_then(|n| n.parse().ok()) == Some(self.nonce) => {
                    return if reply["evt"] == "ERROR" {
                        Err(io::Error::other("Discord rejected the activity"))
                    } else {
                        Ok(())
                    };
                }
                _ => {}
            }
        }
    }
}

/// Discord only takes text of 2 to 128 characters and ignores the whole activity otherwise.
fn text(s: &str) -> Option<String> {
    let s: String = s.trim().chars().take(120).collect();
    (s.chars().count() >= 2).then_some(s)
}

fn payload(a: &Activity) -> Value {
    let mut v = json!({
        "assets": { "large_image": LOGO, "large_text": NAME },
        "buttons": [{ "label": BUTTON.0, "url": BUTTON.1 }],
    });
    if let Some(d) = text(&a.details) {
        v["details"] = d.into();
    }
    if let Some(s) = text(&a.state) {
        v["state"] = s.into();
    }
    if let Some(t) = a.start {
        v["timestamps"] = json!({ "start": t });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, FRAME, &json!({ "a": 1 })).unwrap();
        assert_eq!(&buf[..4], &1u32.to_le_bytes());
        let (op, v) = read_frame(&mut buf.as_slice()).unwrap();
        assert_eq!((op, v["a"].as_i64()), (FRAME, Some(1)));
    }

    #[test]
    fn oversized_frames_are_refused() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_frame(&mut buf.as_slice()).is_err());
    }

    #[test]
    fn short_and_long_text() {
        assert_eq!(text("a"), None);
        assert_eq!(text("  "), None);
        assert_eq!(text("ok").as_deref(), Some("ok"));
        assert_eq!(text(&"è".repeat(300)).unwrap().chars().count(), 120);
    }

    #[test]
    fn payload_leaves_out_what_is_empty() {
        let v = payload(&Activity { details: "Editing main.cpp".into(), state: String::new(), start: Some(5) });
        assert_eq!(v["details"], "Editing main.cpp");
        assert!(v.get("state").is_none());
        assert_eq!(v["timestamps"]["start"], 5);
        assert_eq!(v["buttons"][0]["label"], BUTTON.0);
        assert!(BUTTON.0.chars().count() <= 32 && BUTTON.1.starts_with("https://"), "Discord's limits for a button");
    }

    /// The worker connects when Discord is there, sends only the latest activity and clears it.
    #[cfg(unix)]
    #[test]
    fn worker_follows_the_latest_activity() {
        use std::os::unix::net::UnixStream;
        use std::sync::Mutex as M;
        let (mine, mut theirs) = UnixStream::pair().unwrap();
        let slot = M::new(Some(Box::new(mine) as Box<dyn Stream>));
        let (tx, rx) = channel();
        let worker = std::thread::spawn(move || run(rx, "9", || slot.lock().unwrap().take()));
        let a = |d: &str| Activity { details: d.into(), state: String::new(), start: None };
        // Both go out before the worker wakes up, so only the second one is sent.
        tx.send(Some(a("Editing a.rs"))).unwrap();
        tx.send(Some(a("Editing b.rs"))).unwrap();
        let (_, hello) = read_frame(&mut theirs).unwrap();
        assert_eq!(hello["client_id"], "9");
        write_frame(&mut theirs, FRAME, &json!({ "evt": "READY" })).unwrap();
        let (_, cmd) = read_frame(&mut theirs).unwrap();
        assert!(cmd["args"]["activity"]["details"].as_str().unwrap().starts_with("Editing"));
        write_frame(&mut theirs, FRAME, &json!({ "nonce": cmd["nonce"] })).unwrap();
        drop(tx);
        worker.join().unwrap();
    }

    /// Against the Discord app running on this computer, with this app's id: the handshake and
    /// an activity must be accepted. Run by hand: `cargo test real_discord -- --ignored`.
    #[test]
    #[ignore]
    fn real_discord_accepts_the_app() {
        let s = open().expect("Discord is not running");
        let mut c = Conn::start(s, CLIENT_ID).expect("handshake refused: wrong application id?");
        let a = Activity { details: "Editing main.cpp".into(), state: "in a self test".into(), start: None };
        c.activity(Some(&a)).expect("activity refused");
        std::thread::sleep(Duration::from_secs(8));
        c.activity(None).expect("clearing refused");
    }

    /// A fake Discord on the other end of a socket pair.
    #[cfg(unix)]
    #[test]
    fn talks_to_discord() {
        use std::os::unix::net::UnixStream;
        let (mine, mut theirs) = UnixStream::pair().unwrap();
        let fake = std::thread::spawn(move || {
            let (op, hello) = read_frame(&mut theirs).unwrap();
            assert_eq!((op, hello["client_id"].as_str()), (HANDSHAKE, Some("123")));
            write_frame(&mut theirs, FRAME, &json!({ "cmd": "DISPATCH", "evt": "READY" })).unwrap();
            // A ping in the middle must be answered, and the reply still comes after it.
            let (_, cmd) = read_frame(&mut theirs).unwrap();
            assert_eq!(cmd["cmd"], "SET_ACTIVITY");
            assert_eq!(cmd["args"]["activity"]["details"], "Editing main.cpp");
            write_frame(&mut theirs, PING, &json!({ "n": 1 })).unwrap();
            let (op, _) = read_frame(&mut theirs).unwrap();
            assert_eq!(op, PONG);
            write_frame(&mut theirs, FRAME, &json!({ "cmd": "SET_ACTIVITY", "nonce": cmd["nonce"] })).unwrap();
            // Clearing sends no activity.
            let (_, cmd) = read_frame(&mut theirs).unwrap();
            assert!(cmd["args"]["activity"].is_null());
            write_frame(&mut theirs, FRAME, &json!({ "evt": "ERROR", "nonce": cmd["nonce"] })).unwrap();
        });
        let mut c = Conn::start(mine, "123").unwrap();
        let a = Activity { details: "Editing main.cpp".into(), state: "in demo".into(), start: None };
        c.activity(Some(&a)).unwrap();
        assert!(c.activity(None).is_err());
        fake.join().unwrap();
    }
}
