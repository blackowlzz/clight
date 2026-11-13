use serde::Serialize;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

const MAX_SCROLLBACK: usize = 3000;

const BOLD: u16 = 1;
const DIM: u16 = 2;
const ITALIC: u16 = 4;
const UNDER: u16 = 8;
const INVERSE: u16 = 16;
const STRONG: u16 = 32;

#[derive(Clone, Copy, PartialEq, Debug)]
struct Cell {
    c: char,
    a: u16,
}
const BLANK: Cell = Cell { c: ' ', a: 0 };
const SPACER: char = '\0';

#[derive(Serialize, Default, Clone)]
pub struct TermInfo {
    pub cwd: String,
    pub branch: Option<String>,
    pub busy: bool,
}

#[derive(Serialize, Default)]
pub struct TermView {
    pub rows: Vec<(usize, String)>,
    pub x: usize,
    pub y: usize,
    pub cursor: bool,
    pub app_cursor: bool,
    pub bracketed: bool,
    pub scrollback: usize,
    pub alt: bool,
}

pub struct Grid {
    cols: usize,
    rows: usize,
    screen: Vec<Vec<Cell>>,
    alt: Option<Vec<Vec<Cell>>>,
    scrollback: VecDeque<Vec<Cell>>,
    dirty: Vec<bool>,
    x: usize,
    y: usize,
    top: usize,
    bottom: usize,
    attr: u16,
    saved: (usize, usize, u16),
    wrap_pending: bool,
    app_cursor: bool,
    bracketed: bool,
    cursor_on: bool,
    dec_graphics: bool,
    last: char,
    reply: Vec<u8>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Grid {
        Grid {
            cols,
            rows,
            screen: vec![vec![BLANK; cols]; rows],
            alt: None,
            scrollback: VecDeque::new(),
            dirty: vec![true; rows],
            x: 0,
            y: 0,
            top: 0,
            bottom: rows - 1,
            attr: 0,
            saved: (0, 0, 0),
            wrap_pending: false,
            app_cursor: false,
            bracketed: false,
            cursor_on: true,
            dec_graphics: false,
            last: ' ',
            reply: vec![],
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        while self.screen.len() > rows {
            if self.screen.len() - 1 > self.y {
                self.screen.pop();
            } else {
                let line = self.screen.remove(0);
                self.y = self.y.saturating_sub(1);
                if self.alt.is_none() {
                    self.push_scrollback(line);
                }
            }
        }
        while self.screen.len() < rows {
            self.screen.push(vec![BLANK; cols]);
        }
        for r in &mut self.screen {
            r.resize(cols, BLANK);
        }
        if let Some(a) = &mut self.alt {
            a.resize(rows, vec![BLANK; cols]);
            for r in a {
                r.resize(cols, BLANK);
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.x = self.x.min(cols - 1);
        self.y = self.y.min(rows - 1);
        self.wrap_pending = false;
        self.dirty = vec![true; rows];
    }

    fn push_scrollback(&mut self, mut line: Vec<Cell>) {
        while line.last() == Some(&BLANK) {
            line.pop();
        }
        line.shrink_to_fit();
        self.scrollback.push_back(line);
        if self.scrollback.len() > MAX_SCROLLBACK {
            self.scrollback.pop_front();
        }
    }

    fn mark(&mut self, from: usize, to: usize) {
        for d in &mut self.dirty[from..=to.min(self.rows - 1)] {
            *d = true;
        }
    }

    fn scroll_up(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom - self.top + 1) {
            let line = self.screen.remove(self.top);
            if self.top == 0 && self.alt.is_none() {
                self.push_scrollback(line);
            }
            self.screen.insert(self.bottom, vec![BLANK; self.cols]);
        }
        self.mark(self.top, self.bottom);
    }

    fn scroll_down(&mut self, n: usize) {
        for _ in 0..n.min(self.bottom - self.top + 1) {
            self.screen.remove(self.bottom);
            self.screen.insert(self.top, vec![BLANK; self.cols]);
        }
        self.mark(self.top, self.bottom);
    }

    fn linefeed(&mut self) {
        self.wrap_pending = false;
        if self.y == self.bottom {
            self.scroll_up(1);
        } else if self.y < self.rows - 1 {
            self.y += 1;
        }
    }

    fn goto(&mut self, x: usize, y: usize) {
        self.x = x.min(self.cols - 1);
        self.y = y.min(self.rows - 1);
        self.wrap_pending = false;
    }

    fn clear(&mut self, y: usize, from: usize, to: usize) {
        for c in &mut self.screen[y][from.min(self.cols)..to.min(self.cols)] {
            *c = BLANK;
        }
        self.dirty[y] = true;
    }

    fn set_alt(&mut self, on: bool) {
        if on && self.alt.is_none() {
            self.saved = (self.x, self.y, self.attr);
            let blank = vec![vec![BLANK; self.cols]; self.rows];
            self.alt = Some(std::mem::replace(&mut self.screen, blank));
        } else if !on {
            if let Some(main) = self.alt.take() {
                self.screen = main;
                // The window may have been resized while the program (vim, less, htop) ran, so
                // the place saved when it started can be outside the screen now.
                let (x, y, a) = self.saved;
                self.attr = a;
                self.goto(x, y);
            }
        }
        self.mark(0, self.rows - 1);
    }

    fn set_fg(&mut self, color: u16) {
        self.attr = (self.attr & 0xff) | (color << 8);
    }

    fn sgr(&mut self, p: &[u16]) {
        let by_luma = |l: u32| {
            if l < 85 {
                DIM
            } else if l > 200 {
                BOLD
            } else {
                0
            }
        };
        let mut i = 0;
        if p.is_empty() {
            self.attr = 0;
        }
        while i < p.len() {
            match p[i] {
                0 => self.attr = 0,
                1 => self.attr |= BOLD,
                2 => self.attr |= DIM,
                3 => self.attr |= ITALIC,
                4 => self.attr |= UNDER,
                7 => self.attr |= INVERSE,
                22 => self.attr &= !(BOLD | DIM),
                23 => self.attr &= !ITALIC,
                24 => self.attr &= !UNDER,
                27 => self.attr &= !INVERSE,
                30..=37 => self.set_fg(p[i] - 29),
                90..=97 => self.set_fg(p[i] - 89),
                39 => {
                    self.set_fg(0);
                    self.attr &= !(STRONG | DIM);
                }
                38 | 48 => {
                    let fg = p[i] == 38;
                    let mut luma = None;
                    match p.get(i + 1) {
                        Some(5) => {
                            let n = *p.get(i + 2).unwrap_or(&7) as u32;
                            luma = Some(match n {
                                0..=15 => {
                                    if n == 0 || n == 8 {
                                        40
                                    } else {
                                        150
                                    }
                                }
                                232..=255 => 8 + (n - 232) * 10,
                                _ => 150,
                            });
                            if fg && n < 16 {
                                self.set_fg(n as u16 % 8 + 1);
                                luma = None;
                            }
                            i += 2;
                        }
                        Some(2) => {
                            let c = |k: usize| *p.get(i + k).unwrap_or(&0) as u32;
                            luma = Some((c(2) * 3 + c(3) * 6 + c(4)) / 10);
                            i += 4;
                        }
                        _ => {}
                    }
                    if let Some(l) = luma {
                        if fg {
                            self.attr = (self.attr & !(DIM | STRONG)) | (by_luma(l) & DIM);
                        } else if l > 120 {
                            self.attr |= INVERSE;
                        }
                    }
                }
                49 => self.attr &= !INVERSE,
                _ => {}
            }
            i += 1;
        }
    }

    fn row_html(line: &[Cell], out: &mut String) {
        let end = line.iter().rposition(|c| *c != BLANK).map_or(0, |i| i + 1);
        let mut cur = 0u16;
        let close = |a: u16, out: &mut String| {
            if a != 0 {
                out.push_str("</span>");
            }
        };
        for c in &line[..end] {
            if c.c == SPACER {
                continue;
            }
            if c.a != cur {
                close(cur, out);
                cur = c.a;
                if cur != 0 {
                    out.push_str("<span class=\"");
                    for (bit, name) in [
                        (BOLD, "b "),
                        (DIM, "d "),
                        (ITALIC, "i "),
                        (UNDER, "u "),
                        (INVERSE, "v "),
                        (STRONG, "s "),
                    ] {
                        if cur & bit != 0 {
                            out.push_str(name);
                        }
                    }
                    if cur >> 8 != 0 {
                        out.push_str(&format!("c{} ", cur >> 8));
                    }
                    out.push_str("\">");
                }
            }
            match c.c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                ch => out.push(ch),
            }
        }
        close(cur, out);
    }

    pub fn view(&mut self, offset: usize, full: bool) -> TermView {
        let offset = offset.min(self.scrollback.len());
        let mut rows = vec![];
        for y in 0..self.rows {
            if offset == 0 && !full && !self.dirty[y] {
                continue;
            }
            let mut html = String::new();
            if y < offset {
                if let Some(line) = self.scrollback.get(self.scrollback.len() - offset + y) {
                    Grid::row_html(line, &mut html);
                }
            } else {
                Grid::row_html(&self.screen[y - offset], &mut html);
            }
            rows.push((y, html));
        }
        if offset == 0 {
            self.dirty.iter_mut().for_each(|d| *d = false);
        }
        TermView {
            rows,
            x: self.x,
            y: self.y + offset,
            cursor: self.cursor_on && offset + self.y < self.rows,
            app_cursor: self.app_cursor,
            bracketed: self.bracketed,
            scrollback: self.scrollback.len(),
            alt: self.alt.is_some(),
        }
    }

    pub fn take_reply(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.reply)
    }
}

fn dec_map(c: char) -> char {
    match c {
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'q' => '─',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        '~' => '·',
        c => c,
    }
}

impl Perform for Grid {
    fn print(&mut self, c: char) {
        let c = if self.dec_graphics { dec_map(c) } else { c };
        let w = c.width().unwrap_or(0);
        if w == 0 {
            return;
        }
        self.last = c;
        if self.wrap_pending || self.x + w > self.cols {
            self.x = 0;
            self.linefeed();
        }
        self.screen[self.y][self.x] = Cell { c, a: self.attr };
        if w == 2 && self.x + 1 < self.cols {
            self.screen[self.y][self.x + 1] = Cell {
                c: SPACER,
                a: self.attr,
            };
        }
        self.dirty[self.y] = true;
        self.x += w;
        if self.x >= self.cols {
            self.x = self.cols - 1;
            self.wrap_pending = true;
        }
    }

    fn execute(&mut self, b: u8) {
        match b {
            b'\n' | 0x0b | 0x0c => self.linefeed(),
            b'\r' => {
                self.x = 0;
                self.wrap_pending = false;
            }
            0x08 => {
                self.x = self.x.saturating_sub(1);
                self.wrap_pending = false;
            }
            b'\t' => self.x = ((self.x / 8 + 1) * 8).min(self.cols - 1),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, inter: &[u8], _ignore: bool, action: char) {
        let raw: Vec<u16> = params.iter().map(|p| p[0]).collect();
        let p = |i: usize, d: usize| {
            raw.get(i)
                .map_or(d, |v| if *v == 0 { d } else { *v as usize })
        };
        let private = inter.first() == Some(&b'?');

        if private {
            let on = action == 'h';
            if on || action == 'l' {
                for &m in &raw {
                    match m {
                        1 => self.app_cursor = on,
                        25 => self.cursor_on = on,
                        47 | 1047 | 1049 => self.set_alt(on),
                        2004 => self.bracketed = on,
                        _ => {}
                    }
                }
            }
            return;
        }
        if inter.first() == Some(&b'>') {
            if action == 'c' {
                self.reply.extend_from_slice(b"\x1b[>0;0;0c");
            }
            return;
        }
        if !inter.is_empty() {
            return;
        }
        let (x, y) = (self.x, self.y);
        match action {
            'A' => self.goto(
                x,
                y.saturating_sub(p(0, 1))
                    .max(if y >= self.top { self.top } else { 0 }),
            ),
            'B' => self.goto(
                x,
                (y + p(0, 1)).min(if y <= self.bottom {
                    self.bottom
                } else {
                    self.rows - 1
                }),
            ),
            'C' => self.goto(x + p(0, 1), y),
            'D' => self.goto(x.saturating_sub(p(0, 1)), y),
            'E' => self.goto(0, y + p(0, 1)),
            'F' => self.goto(0, y.saturating_sub(p(0, 1))),
            'G' | '`' => self.goto(p(0, 1) - 1, y),
            'H' | 'f' => self.goto(p(1, 1) - 1, p(0, 1) - 1),
            'd' => self.goto(x, p(0, 1) - 1),
            'J' => {
                let cols = self.cols;
                match raw.first().copied().unwrap_or(0) {
                    0 => {
                        self.clear(y, x, cols);
                        for r in y + 1..self.rows {
                            self.clear(r, 0, cols);
                        }
                    }
                    1 => {
                        for r in 0..y {
                            self.clear(r, 0, cols);
                        }
                        self.clear(y, 0, x + 1);
                    }
                    2 | 3 => {
                        for r in 0..self.rows {
                            self.clear(r, 0, cols);
                        }
                        if raw.first() == Some(&3) {
                            self.scrollback.clear();
                        }
                    }
                    _ => {}
                }
            }
            'K' => {
                let cols = self.cols;
                match raw.first().copied().unwrap_or(0) {
                    0 => self.clear(y, x, cols),
                    1 => self.clear(y, 0, x + 1),
                    2 => self.clear(y, 0, cols),
                    _ => {}
                }
            }
            '@' => {
                for _ in 0..p(0, 1).min(self.cols - x) {
                    self.screen[y].pop();
                    self.screen[y].insert(x, BLANK);
                }
                self.dirty[y] = true;
            }
            'P' => {
                for _ in 0..p(0, 1).min(self.cols - x) {
                    self.screen[y].remove(x);
                    self.screen[y].push(BLANK);
                }
                self.dirty[y] = true;
            }
            'X' => self.clear(y, x, x + p(0, 1)),
            'L' if (self.top..=self.bottom).contains(&y) => {
                for _ in 0..p(0, 1).min(self.bottom - y + 1) {
                    self.screen.remove(self.bottom);
                    self.screen.insert(y, vec![BLANK; self.cols]);
                }
                self.mark(y, self.bottom);
            }
            'M' if (self.top..=self.bottom).contains(&y) => {
                for _ in 0..p(0, 1).min(self.bottom - y + 1) {
                    self.screen.remove(y);
                    self.screen.insert(self.bottom, vec![BLANK; self.cols]);
                }
                self.mark(y, self.bottom);
            }
            'S' => self.scroll_up(p(0, 1)),
            'T' => self.scroll_down(p(0, 1)),
            'b' => {
                for _ in 0..p(0, 1).min(self.cols) {
                    let c = self.last;
                    self.print(c);
                }
            }
            'm' => self.sgr(&raw),
            'r' => {
                let (t, b) = (p(0, 1) - 1, p(1, self.rows) - 1);
                if t < b && b < self.rows {
                    self.top = t;
                    self.bottom = b;
                    self.goto(0, 0);
                }
            }
            's' => self.saved = (x, y, self.attr),
            'u' => {
                let (sx, sy, a) = self.saved;
                self.attr = a;
                self.goto(sx, sy);
            }
            'n' => match raw.first() {
                Some(6) => self
                    .reply
                    .extend_from_slice(format!("\x1b[{};{}R", y + 1, x + 1).as_bytes()),
                Some(5) => self.reply.extend_from_slice(b"\x1b[0n"),
                _ => {}
            },
            'c' => self.reply.extend_from_slice(b"\x1b[?62;22c"),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, inter: &[u8], _ignore: bool, b: u8) {
        if inter.first() == Some(&b'(') {
            self.dec_graphics = b == b'0';
            return;
        }
        if !inter.is_empty() {
            return;
        }
        match b {
            b'7' => self.saved = (self.x, self.y, self.attr),
            b'8' => {
                let (x, y, a) = self.saved;
                self.attr = a;
                self.goto(x, y);
            }
            b'D' => self.linefeed(),
            b'E' => {
                self.x = 0;
                self.linefeed();
            }
            b'M' => {
                if self.y == self.top {
                    self.scroll_down(1);
                } else {
                    self.y = self.y.saturating_sub(1);
                }
            }
            b'c' => {
                let (c, r) = (self.cols, self.rows);
                let sb = std::mem::take(&mut self.scrollback);
                *self = Grid::new(c, r);
                self.scrollback = sb;
            }
            _ => {}
        }
    }
}

pub struct Inner {
    pub parser: Parser,
    pub grid: Grid,
}

impl Inner {
    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let Inner { parser, grid } = self;
        parser.advance(grid, bytes);
        grid.take_reply()
    }
}

pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Copy)]
enum OnExit {
    CloseTab,
    ReportRun,
}

#[derive(Serialize, Clone)]
struct RunExit {
    code: Option<i32>,
    signal: Option<i32>,
}

fn announce_exit(
    app: &AppHandle,
    id: u32,
    on_exit: OnExit,
    code: Option<i32>,
    signal: Option<i32>,
) {
    let _ = match on_exit {
        OnExit::CloseTab => app.emit("term-exit", id),
        OnExit::ReportRun => app.emit("run-exit", RunExit { code, signal }),
    };
}

fn pump(t: &Term, app: &AppHandle, id: u32, mut reader: impl Read) {
    let mut buf = [0u8; 16 * 1024];
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        let reply = t.inner.lock().unwrap().feed(&buf[..n]);
        if !reply.is_empty() {
            let _ = t.write(&reply);
        }
        if !t.notified.swap(true, Ordering::AcqRel) {
            let _ = app.emit("term-dirty", id);
        }
    }
}

impl Term {
    pub fn note(&self, text: &str) {
        self.inner.lock().unwrap().feed(text.as_bytes());
    }
}

pub use pty::*;

#[cfg(unix)]
mod pty {
    use super::*;
    use crate::runner::RawFd;
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};

    pub struct Term {
        pub inner: Mutex<Inner>,
        writer: Mutex<File>,
        pid: u32,
        fd: RawFd,
        pub notified: AtomicBool,
        running: AtomicBool,
        last_dir: Mutex<(String, Option<String>)>,
    }

    fn winsize(cols: usize, rows: usize) -> libc::winsize {
        libc::winsize {
            ws_row: rows as u16,
            ws_col: cols as u16,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }

    pub fn open_pty(cols: usize, rows: usize) -> Result<(File, File), String> {
        let (mut m, mut s) = (0, 0);
        let ws = winsize(cols, rows);
        let rc =
            unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null(), &ws) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        unsafe {
            libc::fcntl(m, libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(s, libc::F_SETFD, libc::FD_CLOEXEC);
            Ok((
                File::from(OwnedFd::from_raw_fd(m)),
                File::from(OwnedFd::from_raw_fd(s)),
            ))
        }
    }

    pub fn tty_path(slave: &File) -> Result<String, String> {
        let mut buf = [0 as libc::c_char; 128];
        unsafe {
            if libc::ttyname_r(slave.as_raw_fd(), buf.as_mut_ptr(), buf.len()) != 0 {
                return Err("no tty name".into());
            }
            Ok(std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned())
        }
    }

    impl Term {
        pub fn spawn(
            app: AppHandle,
            id: u32,
            cwd: &str,
            cols: usize,
            rows: usize,
        ) -> Result<Arc<Term>, String> {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            let mut cmd = Command::new(&shell);
            cmd.current_dir(cwd);
            Self::start(app, id, cmd, cols, rows, "", OnExit::CloseTab)
        }

        pub fn run(
            app: AppHandle,
            id: u32,
            launch: Launch,
            cols: usize,
            rows: usize,
            banner: &str,
        ) -> Result<Arc<Term>, String> {
            let mut cmd = Command::new(&launch.program);
            cmd.args(&launch.args)
                .envs(launch.env.iter().map(|(k, v)| (k, v)))
                .current_dir(&launch.cwd);
            Self::start(app, id, cmd, cols, rows, banner, OnExit::ReportRun)
        }

        fn start(
            app: AppHandle,
            id: u32,
            mut cmd: Command,
            cols: usize,
            rows: usize,
            banner: &str,
            on_exit: OnExit,
        ) -> Result<Arc<Term>, String> {
            let (master, slave) = open_pty(cols, rows)?;
            crate::runner::clean_env(&mut cmd);
            cmd.env("TERM", "xterm-256color")
                .env("COLORTERM", "")
                .stdin(Stdio::from(slave.try_clone().map_err(|e| e.to_string())?))
                .stdout(Stdio::from(slave.try_clone().map_err(|e| e.to_string())?))
                .stderr(Stdio::from(slave));
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                    Ok(())
                });
            }
            let mut child = cmd
                .spawn()
                .map_err(|e| format!("{}: {e}", cmd.get_program().to_string_lossy()))?;
            let reader = master.try_clone().map_err(|e| e.to_string())?;
            let term = Arc::new(Term {
                inner: Mutex::new(Inner {
                    parser: Parser::new(),
                    grid: Grid::new(cols, rows),
                }),
                fd: master.as_raw_fd(),
                writer: Mutex::new(master),
                pid: child.id(),
                notified: AtomicBool::new(false),
                running: AtomicBool::new(true),
                last_dir: Mutex::new((String::new(), None)),
            });
            term.note(banner);
            let t = term.clone();
            std::thread::spawn(move || {
                pump(&t, &app, id, reader);
                let status = child.wait().ok();
                t.running.store(false, Ordering::SeqCst);
                announce_exit(
                    &app,
                    id,
                    on_exit,
                    status.and_then(|s| s.code()),
                    status.and_then(|s| s.signal()),
                );
            });
            Ok(term)
        }

        pub fn write(&self, data: &[u8]) -> Result<(), String> {
            self.writer
                .lock()
                .unwrap()
                .write_all(data)
                .map_err(|e| e.to_string())
        }

        pub fn resize(&self, cols: usize, rows: usize) {
            let ws = winsize(cols, rows);
            let _guard = self.writer.lock().unwrap();
            unsafe {
                libc::ioctl(self.fd, libc::TIOCSWINSZ as _, &ws);
            }
            self.inner.lock().unwrap().grid.resize(cols, rows);
        }

        pub fn info(&self) -> TermInfo {
            let cwd = std::fs::read_link(format!("/proc/{}/cwd", self.pid))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut last = self.last_dir.lock().unwrap();
            if last.0 != cwd {
                *last = (cwd.clone(), crate::git::branch(&cwd));
            }
            let foreground = unsafe { libc::tcgetpgrp(self.fd) };
            TermInfo {
                cwd,
                branch: last.1.clone(),
                busy: foreground != self.pid as i32,
            }
        }

        fn signal(&self, signal: i32) {
            if self.running.load(Ordering::SeqCst) {
                unsafe {
                    libc::kill(-(self.pid as i32), signal);
                }
            }
        }

        pub fn kill(&self) {
            self.signal(libc::SIGHUP);
        }

        pub fn stop(&self) {
            self.signal(libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
mod pty {
    use super::*;
    use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize, SlavePty};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    pub struct Term {
        pub inner: Mutex<Inner>,
        writer: Mutex<Option<Box<dyn Write + Send>>>,
        master: Mutex<Option<Box<dyn MasterPty + Send>>>,
        pid: Mutex<Option<u32>>,
        pub notified: AtomicBool,
        running: AtomicBool,
    }

    fn command(program: &str, cwd: &str) -> CommandBuilder {
        let mut cmd = CommandBuilder::new(program);
        // CommandBuilder starts from the registry, which lacks the tools clight added to PATH.
        for (k, v) in std::env::vars_os() {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.cwd(cwd);
        cmd
    }

    /// A console starts in the OEM code page, which garbles UTF-8 text like "è".
    fn use_utf8(slave: &dyn SlavePty, cwd: &str) {
        let mut cmd = command("cmd.exe", cwd);
        cmd.args(["/c", "chcp 65001 >nul"]);
        let Ok(mut child) = slave.spawn_command(cmd) else {
            return;
        };
        for _ in 0..200 {
            if !matches!(child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
    }

    fn with_exe_suffix(program: String) -> String {
        let exe = format!("{program}.exe");
        if !program.to_lowercase().ends_with(".exe") && std::path::Path::new(&exe).is_file() {
            exe
        } else {
            program
        }
    }

    impl Term {
        pub fn spawn(
            app: AppHandle,
            id: u32,
            cwd: &str,
            cols: usize,
            rows: usize,
        ) -> Result<Arc<Term>, String> {
            Self::start(
                app,
                id,
                command("powershell.exe", cwd),
                cwd,
                cols,
                rows,
                "",
                OnExit::CloseTab,
            )
        }

        pub fn run(
            app: AppHandle,
            id: u32,
            launch: Launch,
            cols: usize,
            rows: usize,
            banner: &str,
        ) -> Result<Arc<Term>, String> {
            let program = with_exe_suffix(launch.program);
            let mut cmd = command(&program, &launch.cwd);
            cmd.args(&launch.args);
            for (k, v) in &launch.env {
                cmd.env(k, v);
            }
            Self::start(
                app,
                id,
                cmd,
                &launch.cwd,
                cols,
                rows,
                banner,
                OnExit::ReportRun,
            )
        }

        #[allow(clippy::too_many_arguments)]
        fn start(
            app: AppHandle,
            id: u32,
            cmd: CommandBuilder,
            cwd: &str,
            cols: usize,
            rows: usize,
            banner: &str,
            on_exit: OnExit,
        ) -> Result<Arc<Term>, String> {
            let text = |e: anyhow::Error| e.to_string();
            let pair = native_pty_system()
                .openpty(PtySize {
                    rows: rows as u16,
                    cols: cols as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(text)?;
            let reader = pair.master.try_clone_reader().map_err(text)?;
            let writer = pair.master.take_writer().map_err(text)?;
            let term = Arc::new(Term {
                inner: Mutex::new(Inner {
                    parser: Parser::new(),
                    grid: Grid::new(cols, rows),
                }),
                writer: Mutex::new(Some(writer)),
                master: Mutex::new(Some(pair.master)),
                pid: Mutex::new(None),
                notified: AtomicBool::new(false),
                running: AtomicBool::new(true),
            });
            // ConPTY starts from the cursor it is told about and counts its rows from there,
            // so anything written first would push its bottom rows one line out of place.
            let _ = banner;

            // ConPTY asks for the cursor position at startup and waits for the
            // answer, so the reader has to be running before anything is spawned.
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let (t, a) = (term.clone(), app.clone());
            std::thread::spawn(move || {
                pump(&t, &a, id, reader);
                let _ = done_tx.send(());
            });

            use_utf8(&*pair.slave, cwd);
            let spawned = pair.slave.spawn_command(cmd);
            drop(pair.slave);
            let mut child: Box<dyn Child + Send + Sync> = match spawned {
                Ok(child) => child,
                Err(e) => {
                    term.close();
                    return Err(e.to_string());
                }
            };
            *term.pid.lock().unwrap() = child.process_id();

            let t = term.clone();
            std::thread::spawn(move || {
                let status = child.wait().ok();
                t.running.store(false, Ordering::SeqCst);
                // The output pipe only ends once the pseudo console is closed.
                t.close();
                let _ = done_rx.recv_timeout(Duration::from_secs(3));
                let code = status.map(|s| s.exit_code() as i32);
                announce_exit(&app, id, on_exit, code, None);
            });
            Ok(term)
        }

        fn close(&self) {
            let master = self.master.lock().unwrap().take();
            let writer = self.writer.lock().unwrap().take();
            drop(writer);
            drop(master);
        }

        pub fn write(&self, data: &[u8]) -> Result<(), String> {
            let mut writer = self.writer.lock().unwrap();
            let w = writer.as_mut().ok_or("The program has ended")?;
            w.write_all(data)
                .and_then(|_| w.flush())
                .map_err(|e| e.to_string())
        }

        pub fn resize(&self, cols: usize, rows: usize) {
            if let Some(m) = self.master.lock().unwrap().as_ref() {
                let _ = m.resize(PtySize {
                    rows: rows as u16,
                    cols: cols as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
            self.inner.lock().unwrap().grid.resize(cols, rows);
        }

        pub fn info(&self) -> TermInfo {
            TermInfo::default()
        }

        pub fn kill(&self) {
            self.stop();
        }

        pub fn stop(&self) {
            let pid = *self.pid.lock().unwrap();
            let (true, Some(pid)) = (self.running.load(Ordering::SeqCst), pid) else {
                return;
            };
            let mut taskkill = Command::new("taskkill");
            crate::runner::clean_env(&mut taskkill);
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

    fn feed(g: &mut Grid, s: &str) {
        Parser::new().advance(g, s.as_bytes());
    }
    fn row(g: &Grid, y: usize) -> String {
        g.screen[y]
            .iter()
            .filter(|c| c.c != SPACER)
            .map(|c| c.c)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn leaving_a_full_screen_program_after_shrinking_the_window() {
        let mut g = Grid::new(80, 30);
        feed(&mut g, "\x1b[28;70Hprompt\x1b[?1049h");
        g.resize(20, 5);
        feed(&mut g, "\x1b[?1049lok");
        assert!(g.x < 20 && g.y < 5);
    }

    #[test]
    fn text_wraps_only_when_the_next_character_arrives() {
        let mut g = Grid::new(5, 3);
        feed(&mut g, "abcde");
        assert_eq!((g.x, g.y), (4, 0));
        feed(&mut g, "f");
        assert_eq!(row(&g, 0), "abcde");
        assert_eq!(row(&g, 1), "f");
    }

    #[test]
    fn scrolling_fills_the_scrollback() {
        let mut g = Grid::new(4, 2);
        feed(&mut g, "a\r\nb\r\nc");
        assert_eq!(g.scrollback.len(), 1);
        assert_eq!(row(&g, 0), "b");
        let v = g.view(1, true);
        assert_eq!(v.rows[0].1, "a");
        assert_eq!(v.rows[1].1, "b");
    }

    #[test]
    fn cursor_addressing_and_erase() {
        let mut g = Grid::new(10, 3);
        feed(&mut g, "hello\x1b[1;3H\x1b[K");
        assert_eq!(row(&g, 0), "he");
        feed(&mut g, "\x1b[2J");
        assert_eq!(row(&g, 0), "");
    }

    #[test]
    fn alternate_screen_restores_the_shell() {
        let mut g = Grid::new(10, 3);
        feed(&mut g, "shell\x1b[?1049h\x1b[2;2Hvim");
        assert_eq!(row(&g, 1), " vim");
        assert_eq!(row(&g, 0), "");
        feed(&mut g, "\x1b[?1049l");
        assert_eq!(row(&g, 0), "shell");
        assert_eq!((g.x, g.y), (5, 0));
    }

    #[test]
    fn only_changed_rows_are_sent() {
        let mut g = Grid::new(10, 3);
        g.view(0, true);
        feed(&mut g, "x");
        assert_eq!(g.view(0, false).rows.len(), 1);
        assert_eq!(g.view(0, false).rows.len(), 0);
    }

    #[test]
    fn ansi_colors_reach_the_html_and_reset() {
        let mut g = Grid::new(10, 1);
        feed(&mut g, "\x1b[31mr\x1b[92mg\x1b[38;5;4mb\x1b[0mx");
        assert_eq!(
            g.view(0, true).rows[0].1,
            "<span class=\"c2 \">r</span><span class=\"c3 \">g</span><span class=\"c5 \">b</span>x"
        );
    }

    #[test]
    fn the_cursor_position_query_is_answered() {
        let mut g = Grid::new(10, 3);
        feed(&mut g, "ab\r\nc\x1b[6n");
        assert_eq!(g.take_reply(), b"\x1b[2;2R");
    }

    #[test]
    fn html_is_escaped_and_styles_close() {
        let mut g = Grid::new(10, 1);
        feed(&mut g, "\x1b[1m<b>\x1b[0m&");
        assert_eq!(
            g.view(0, true).rows[0].1,
            "<span class=\"b \">&lt;b&gt;</span>&amp;"
        );
    }

    #[test]
    fn scroll_region_keeps_outside_rows() {
        let mut g = Grid::new(5, 4);
        feed(&mut g, "top\x1b[2;3r\x1b[2;1Ha\r\nb\r\nc");
        assert_eq!(row(&g, 0), "top");
        assert_eq!(row(&g, 1), "b");
        assert_eq!(row(&g, 2), "c");
        assert_eq!(g.scrollback.len(), 0);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let mut g = Grid::new(6, 1);
        feed(&mut g, "日本x");
        assert_eq!(g.x, 5);
        assert_eq!(g.view(0, true).rows[0].1, "日本x");
    }

    #[test]
    fn cursor_position_report_is_answered() {
        let mut g = Grid::new(10, 3);
        feed(&mut g, "\x1b[2;4H\x1b[6n");
        assert_eq!(g.take_reply(), b"\x1b[2;4R");
    }

    #[test]
    fn resize_keeps_the_cursor_on_screen() {
        let mut g = Grid::new(10, 4);
        feed(&mut g, "a\r\nb\r\nc\r\nd");
        g.resize(5, 2);
        assert_eq!(row(&g, 1), "d");
        assert_eq!(g.y, 1);
        assert_eq!(g.scrollback.len(), 2);
    }
}
