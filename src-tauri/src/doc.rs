use crate::lexer::{self, Lang, Tok};
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

pub const MAX_RENDER: usize = 4000;
const MAX_UNDO_BYTES: usize = 8 << 20;
const OVERSCAN: usize = 6;
const COALESCE_MS: u128 = 900;

#[derive(Deserialize, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Cmd {
    Type {
        s: String,
    },
    Insert {
        s: String,
    },
    Newline,
    Backspace,
    Delete,
    DeleteWordLeft,
    DeleteWordRight,
    Tab,
    Outdent,
    Move {
        to: Motion,
        ext: bool,
        rows: Option<usize>,
    },
    Click {
        line: usize,
        vcol: usize,
        ext: bool,
    },
    Word {
        line: usize,
        vcol: usize,
    },
    SelectLine {
        line: usize,
    },
    SelectAll,
    MoveSelection {
        line: usize,
        vcol: usize,
        copy: bool,
    },
    NewLineBelow,
    NewLineAbove,
    CompleteStatement,
    ToggleCase,
    GotoBracket,
    Undo,
    Redo,
    ToggleComment,
    DuplicateLine,
    DeleteLine,
    CutSelection,
    Complete {
        s: String,
    },
    Goto {
        line: usize,
        col: usize,
    },
    FindNext {
        back: bool,
    },
    ReplaceOne {
        s: String,
    },
    ReplaceAll {
        s: String,
    },
    Fresh,
    MoveLines {
        up: bool,
    },
    JoinLines,
    ExpandSelection,
    ShrinkSelection,
    ToggleFold {
        row: usize,
    },
    FoldCaret {
        fold: bool,
    },
    FoldAll {
        fold: bool,
    },
    ToggleBreakpoint {
        row: Option<usize>,
    },
    SetBreakpoints {
        lines: Vec<usize>,
    },
    SetExec {
        line: Option<usize>,
    },
}

impl Cmd {
    fn mutates(&self) -> bool {
        use Cmd::*;
        matches!(
            self,
            Type { .. }
                | Insert { .. }
                | Newline
                | Backspace
                | Delete
                | DeleteWordLeft
                | DeleteWordRight
                | Tab
                | Outdent
                | Undo
                | Redo
                | ToggleComment
                | DuplicateLine
                | DeleteLine
                | CutSelection
                | Complete { .. }
                | ReplaceOne { .. }
                | ReplaceAll { .. }
                | MoveLines { .. }
                | JoinLines
                | MoveSelection { .. }
                | NewLineBelow
                | NewLineAbove
                | CompleteStatement
                | ToggleCase
        )
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    WordLeft,
    WordRight,
    PageUp,
    PageDown,
    DocStart,
    DocEnd,
}

#[derive(Serialize, Default)]
pub struct View {
    pub rev: u64,
    pub n: usize,
    pub top: usize,
    pub first: usize,
    pub lines: Vec<String>,
    pub nums: Vec<usize>,
    pub cur: [usize; 2],
    pub line: usize,
    pub col: usize,
    pub total: usize,
    pub crlf: bool,
    pub use_tabs: bool,
    pub tab: usize,
    pub readonly: bool,
    pub folds: Vec<[usize; 2]>,
    pub bps: Vec<usize>,
    pub git: Vec<[usize; 2]>,
    pub exec: Option<usize>,
    pub stripes: Vec<[usize; 2]>,
    pub occ: Vec<[usize; 3]>,
    pub pair: Vec<[usize; 2]>,
    pub all_bps: Option<Vec<usize>>,
    pub dirty: bool,
    pub sel: Vec<[usize; 3]>,
    pub marks: Vec<[usize; 3]>,
    pub diags: Vec<[usize; 4]>,
    pub msg: Option<String>,
    pub maxw: usize,
    pub clip: Option<String>,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub sl: usize,
    pub sc: usize,
    pub el: usize,
    pub ec: usize,
    pub sev: usize,
    pub msg: String,
    pub raw: serde_json::Value,
}

type PairCache = ((usize, u64), Option<(usize, usize)>);

#[derive(Default, Clone)]
struct SemLine {
    toks: Vec<(u32, u32, u8)>,
    fresh: bool,
}

#[derive(Clone)]
struct Op {
    at: usize,
    del: String,
    ins: String,
}

struct Txn {
    ops: Vec<Op>,
    before: (usize, usize),
    after: (usize, usize),
    typing: bool,
    at: Instant,
}

pub struct Doc {
    pub path: PathBuf,
    pub rope: Rope,
    pub lang: Lang,
    pub rev: u64,
    pub saved_rev: u64,
    pub mtime: Option<SystemTime>,
    pub crlf: bool,
    pub diags: Vec<Diag>,
    pub lsp_rev: Option<u64>,
    pub lsp_ver: i32,
    pub tab: usize,
    pub use_tabs: bool,
    cur: usize,
    anchor: usize,
    want: Option<usize>,
    undo: Vec<Txn>,
    redo: Vec<Txn>,
    undo_bytes: usize,
    open_ops: Vec<Op>,
    before: (usize, usize),
    states: Vec<u32>,
    raws: Vec<String>,
    query: String,
    case: bool,
    sent_clip: Option<(usize, usize, u64)>,
    maxw: usize,
    pub readonly: bool,
    folds: Vec<(usize, usize)>,
    bps: Vec<usize>,
    bps_changed: bool,
    pub git_marks: Vec<(usize, usize, u8)>,
    exec: Option<usize>,
    sel_stack: Vec<(usize, usize)>,
    sem: Vec<SemLine>,
    pair_cache: Option<PairCache>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn closer(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

impl Doc {
    pub fn new(path: PathBuf, text: &str, mtime: Option<SystemTime>) -> Doc {
        let crlf = text.contains("\r\n");
        let normalized: Cow<str> = if crlf {
            Cow::Owned(text.replace("\r\n", "\n"))
        } else {
            Cow::Borrowed(text)
        };
        let rope = Rope::from_str(&normalized);
        let lang = Lang::from_path(&path);
        let use_tabs = (0..rope.len_lines().min(400))
            .filter(|&i| rope.line(i).len_chars() > 1)
            .fold((0, 0), |(t, s), i| match rope.line(i).char(0) {
                '\t' => (t + 1, s),
                ' ' => (t, s + 1),
                _ => (t, s),
            });
        Doc {
            path,
            rope,
            lang,
            rev: 0,
            saved_rev: 0,
            mtime,
            crlf,
            diags: vec![],
            lsp_rev: None,
            lsp_ver: 0,
            tab: 4,
            use_tabs: use_tabs.0 > use_tabs.1,
            cur: 0,
            anchor: 0,
            want: None,
            undo: vec![],
            redo: vec![],
            undo_bytes: 0,
            open_ops: vec![],
            before: (0, 0),
            states: vec![0],
            raws: vec![],
            query: String::new(),
            case: false,
            sent_clip: None,
            maxw: 0,
            readonly: false,
            folds: vec![],
            bps: vec![],
            bps_changed: false,
            git_marks: vec![],
            exec: None,
            sel_stack: vec![],
            sem: vec![],
            pair_cache: None,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.rev != self.saved_rev
    }

    pub fn contents_for_disk(&self) -> String {
        let s = self.rope.to_string();
        if self.crlf {
            s.replace('\n', "\r\n")
        } else {
            s
        }
    }

    pub fn reload(&mut self, text: &str, mtime: Option<SystemTime>) {
        let (line, col) = self.pos(self.cur);
        self.crlf = text.contains("\r\n");
        self.rope = if self.crlf {
            Rope::from_str(&text.replace("\r\n", "\n"))
        } else {
            Rope::from_str(text)
        };
        self.mtime = mtime;
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.folds.clear();
        self.sem.clear();
        self.states.truncate(1);
        self.rev += 1;
        self.saved_rev = self.rev;
        self.sent_clip = None;
        self.cur = self.at(line, col);
        self.anchor = self.cur;
    }

    fn line_to_row(&self, line: usize) -> usize {
        let mut hidden = 0;
        for &(s, e) in &self.folds {
            if line > e {
                hidden += e - s;
            } else if line > s {
                return s - hidden;
            } else {
                break;
            }
        }
        line - hidden
    }

    fn row_to_line(&self, row: usize) -> usize {
        let mut line = row;
        for &(s, e) in &self.folds {
            if line > s {
                line += e - s;
            } else {
                break;
            }
        }
        line.min(self.rope.len_lines() - 1)
    }

    fn row_count(&self) -> usize {
        self.rope.len_lines() - self.folds.iter().map(|(s, e)| e - s).sum::<usize>()
    }

    pub fn set_semantic(&mut self, tokens: Vec<(usize, usize, usize, u8)>) {
        let total = self.rope.len_lines();
        let mut sem = vec![
            SemLine {
                toks: vec![],
                fresh: true
            };
            total
        ];
        for (line, col, len, tag) in tokens {
            if line >= total {
                continue;
            }
            let ascii = {
                let l = self.rope.line(line);
                l.len_bytes() == l.len_chars()
            };
            let (start, end) = if ascii {
                (col, col + len)
            } else {
                (
                    self.col_from_utf16(line, col),
                    self.col_from_utf16(line, col + len),
                )
            };
            sem[line].toks.push((start as u32, end as u32, tag));
        }
        self.sem = sem;
    }

    fn shift_sem(
        &mut self,
        line: usize,
        col: usize,
        del: usize,
        ins: usize,
        removed: usize,
        added: usize,
    ) {
        if line >= self.sem.len() {
            return;
        }
        let (c, d, i) = (col as u32, del as u32, ins as u32);
        let sl = &mut self.sem[line];
        sl.fresh = false;
        if removed == 0 && added == 0 {
            sl.toks.retain_mut(|t| {
                if t.1 <= c {
                    true
                } else if t.0 >= c + d {
                    t.0 = t.0 + i - d;
                    t.1 = t.1 + i - d;
                    true
                } else {
                    false
                }
            });
        } else {
            sl.toks.retain(|t| t.1 <= c);
            let end = (line + removed).min(self.sem.len() - 1);
            self.sem.splice(
                line + 1..=end,
                std::iter::repeat_n(SemLine::default(), added),
            );
        }
    }

    fn overlay_semantic(&self, line: usize, text: &str, toks: &mut Vec<Tok>) {
        let Some(sl) = self.sem.get(line) else { return };
        let guess = |tag: u8| {
            matches!(
                tag,
                lexer::FUNCTION
                    | lexer::USER_TYPE
                    | lexer::NAMESPACE
                    | lexer::MACRO
                    | lexer::MEMBER
                    | lexer::VARIABLE
            )
        };
        if sl.fresh {
            toks.retain(|t| !guess(t.tag));
        }
        if sl.toks.is_empty() {
            return;
        }
        let ascii = text.is_ascii();
        let offsets: Vec<usize> = if ascii {
            vec![]
        } else {
            text.char_indices()
                .map(|(b, _)| b)
                .chain([text.len()])
                .collect()
        };
        let byte = |c: u32| {
            if ascii {
                (c as usize).min(text.len())
            } else {
                *offsets.get(c as usize).unwrap_or(&text.len())
            }
        };
        for &(cs, ce, tag) in &sl.toks {
            let (s, e) = (byte(cs) as u32, byte(ce) as u32);
            if s >= e {
                continue;
            }
            let hits = |t: &Tok| t.start < e && s < t.end;
            if tag != lexer::COMMENT
                && toks
                    .iter()
                    .any(|t| hits(t) && !guess(t.tag) && t.tag != lexer::TYPE)
            {
                continue;
            }
            toks.retain(|t| !hits(t));
            if tag != 0 {
                toks.push(Tok {
                    start: s,
                    end: e,
                    tag,
                });
            }
        }
        toks.sort_unstable_by_key(|t| t.start);
    }

    fn shift_marks(&mut self, line: usize, col0: bool, removed: usize, added: usize) {
        if removed == 0 && added == 0 {
            return;
        }
        let delta = added as isize - removed as isize;
        let move_line = |l: usize| {
            if l > line + removed || (l == line && col0 && removed == 0) {
                (l as isize + delta) as usize
            } else if l > line {
                line
            } else {
                l
            }
        };
        self.folds.retain_mut(|f| {
            if line + removed < f.0 {
                *f = (
                    (f.0 as isize + delta) as usize,
                    (f.1 as isize + delta) as usize,
                );
                true
            } else {
                line > f.1
            }
        });
        let before = self.bps.clone();
        self.bps.iter_mut().for_each(|l| *l = move_line(*l));
        self.bps.dedup();
        if before != self.bps {
            self.bps_changed = true;
        }
        for m in &mut self.git_marks {
            m.0 = move_line(m.0);
            m.1 = move_line(m.1);
        }
    }

    fn fold_end(&mut self, line: usize) -> Option<usize> {
        let total = self.rope.len_lines();
        self.ensure_states(line);
        let mut toks = Vec::new();
        let opening_comment = {
            let st = self.states[line];
            let text = self.line_text(line).into_owned();
            toks.clear();
            st == 0 && lexer::lex(&text, st, self.lang, &mut self.raws, &mut toks) == lexer::BLOCK
        };
        if opening_comment {
            for l in line + 1..total.min(line + 20_000) {
                self.ensure_states(l + 1);
                if self.states[l + 1] != lexer::BLOCK {
                    return Some(l);
                }
            }
            return None;
        }
        let mut depth = 0i32;
        for l in line..total.min(line + 100_000) {
            self.ensure_states(l);
            let text = self.line_text(l).into_owned();
            toks.clear();
            lexer::lex(&text, self.states[l], self.lang, &mut self.raws, &mut toks);
            let mut t = toks.iter().peekable();
            for (i, b) in text.bytes().enumerate() {
                while t.peek().is_some_and(|k| k.end as usize <= i) {
                    t.next();
                }
                if t.peek().is_some_and(|k| {
                    k.start as usize <= i
                        && matches!(k.tag, lexer::COMMENT | lexer::STRING | lexer::ESCAPE)
                }) {
                    continue;
                }
                match b {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return (l > line).then_some(l);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn set_fold(&mut self, line: usize, fold: bool) {
        if let Some(i) = self.folds.iter().position(|f| f.0 == line) {
            if !fold {
                self.folds.remove(i);
            }
            return;
        }
        if !fold {
            return;
        }
        if let Some(end) = self.fold_end(line) {
            self.folds.retain(|f| !(f.0 >= line && f.1 <= end));
            if self.folds.iter().any(|f| f.0 < line && f.1 >= end) {
                return;
            }
            self.folds.push((line, end));
            self.folds.sort_unstable();
        }
    }

    fn fold_caret(&mut self, fold: bool) {
        let (line, _) = self.pos(self.cur);
        if !fold {
            self.set_fold(line, false);
            return;
        }
        let start = (0..=line).rev().take(5000).find(|&l| {
            self.fold_candidate(l) && self.brace_block_end(l).is_some_and(|e| e >= line)
        });
        if let Some(l) = start {
            self.set_fold(l, true);
            let at = self.at(l, 0);
            self.set(at);
        }
    }

    fn brace_block_end(&self, line: usize) -> Option<usize> {
        let mut depth = 0i32;
        for l in line..self.rope.len_lines().min(line + 20_000) {
            for c in self.rope.line(l).chars() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return (l > line).then_some(l);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn fold_candidate(&self, line: usize) -> bool {
        self.line_text(line).trim_end().ends_with('{')
    }

    fn fold_all(&mut self, fold: bool) {
        self.folds.clear();
        if fold {
            let mut l = 0;
            while l < self.rope.len_lines() {
                if self.fold_candidate(l) {
                    if let Some(e) = self.fold_end(l) {
                        self.folds.push((l, e));
                        l = e;
                    }
                }
                l += 1;
            }
        }
    }

    fn toggle_breakpoint(&mut self, line: usize) {
        match self.bps.binary_search(&line) {
            Ok(i) => {
                self.bps.remove(i);
            }
            Err(i) => self.bps.insert(i, line),
        }
        self.bps_changed = true;
    }

    fn word_range(&self, at: usize) -> Option<(usize, usize)> {
        let n = self.rope.len_chars();
        let on = |i: usize| i < n && is_word(self.rope.char(i));
        if !on(at) && !(at > 0 && on(at - 1)) {
            return None;
        }
        let mut a = at;
        while a > 0 && on(a - 1) {
            a -= 1;
        }
        let mut b = at;
        while on(b) {
            b += 1;
        }
        Some((a, b))
    }

    pub fn word_at(&self, row: usize, vcol: usize) -> Option<String> {
        let line = self.row_to_line(row);
        let at = self.at(line, self.col_from_vcol(line, vcol));
        let (a, b) = self.word_range(at)?;
        Some(self.rope.slice(a..b).to_string())
    }

    pub fn diag_at(&self, row: usize, vcol: usize) -> Option<String> {
        let line = self.row_to_line(row);
        let col = self.col_from_vcol(line, vcol);
        let hit =
            |d: &&Diag| (d.sl, d.sc) <= (line, col) && (line, col) <= (d.el, d.ec.max(d.sc + 1));
        self.diags
            .iter()
            .filter(hit)
            .min_by_key(|d| d.sev)
            .map(|d| d.msg.clone())
    }

    pub fn next_problem(&self) -> Option<(usize, usize)> {
        let here = self.pos(self.cur);
        let mut all: Vec<(usize, usize)> = self
            .diags
            .iter()
            .filter(|d| d.sev <= 2)
            .map(|d| (d.sl, d.sc))
            .collect();
        all.sort_unstable();
        all.iter().find(|p| **p > here).or(all.first()).copied()
    }

    pub fn goto_chars(&mut self, line: usize, col: usize) {
        let at = self.at(line, col);
        self.set(at);
    }

    pub fn pos(&self, ch: usize) -> (usize, usize) {
        let line = self.rope.char_to_line(ch);
        (line, ch - self.rope.line_to_char(line))
    }

    pub fn cursor_pos(&self) -> (usize, usize) {
        self.pos(self.cur)
    }

    fn line_len(&self, line: usize) -> usize {
        let s = self.rope.line(line);
        let n = s.len_chars();
        if n > 0 && s.char(n - 1) == '\n' {
            n - 1
        } else {
            n
        }
    }

    pub fn at(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.rope.len_lines() - 1);
        self.rope.line_to_char(line) + col.min(self.line_len(line))
    }

    pub fn line_text(&self, line: usize) -> Cow<'_, str> {
        let s: Cow<str> = self.rope.line(line).into();
        match s {
            Cow::Borrowed(b) => Cow::Borrowed(b.strip_suffix('\n').unwrap_or(b)),
            Cow::Owned(mut o) => {
                if o.ends_with('\n') {
                    o.pop();
                }
                Cow::Owned(o)
            }
        }
    }

    fn step(&self, vcol: usize, c: char) -> usize {
        if c == '\t' {
            vcol + self.tab - vcol % self.tab
        } else {
            vcol + 1
        }
    }

    fn vcol(&self, line: usize, col: usize) -> usize {
        self.rope
            .line(line)
            .chars()
            .take(col)
            .fold(0, |v, c| self.step(v, c))
    }

    fn col_from_vcol(&self, line: usize, target: usize) -> usize {
        let mut v = 0;
        for (i, c) in self.rope.line(line).chars().enumerate() {
            if c == '\n' {
                return i;
            }
            let next = self.step(v, c);
            if target < next {
                return if target - v > (next - v) / 2 {
                    i + 1
                } else {
                    i
                };
            }
            v = next;
        }
        self.line_len(line)
    }

    pub fn utf16_col(&self, line: usize, col: usize) -> usize {
        self.rope
            .line(line)
            .chars()
            .take(col)
            .map(char::len_utf16)
            .sum()
    }

    pub fn col_from_utf16(&self, line: usize, units: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let mut u = 0;
        for (i, c) in self.rope.line(line).chars().enumerate() {
            if u >= units || c == '\n' {
                return i;
            }
            u += c.len_utf16();
        }
        self.line_len(line)
    }

    pub fn prefix(&self) -> String {
        let mut start = self.cur;
        while start > 0 && is_word(self.rope.char(start - 1)) {
            start -= 1;
        }
        self.rope.slice(start..self.cur).to_string()
    }

    /// Whether the character just before the caret is in a comment or a string.
    pub fn after_comment_or_string(&mut self) -> bool {
        let (line, col) = self.pos(self.cur);
        if col == 0 {
            return false;
        }
        self.ensure_states(line);
        let text = self.line_text(line).into_owned();
        let byte = text.char_indices().nth(col - 1).map_or(0, |(b, _)| b) as u32;
        let mut toks = vec![];
        lexer::lex(
            &text,
            self.states[line],
            self.lang,
            &mut self.raws,
            &mut toks,
        );
        toks.iter().any(|t| {
            (t.start..t.end).contains(&byte)
                && matches!(t.tag, lexer::COMMENT | lexer::STRING | lexer::ESCAPE)
        })
    }

    pub fn before_prefix(&self) -> String {
        let start = self.cur - self.prefix().chars().count();
        self.rope.slice(start.saturating_sub(2)..start).to_string()
    }

    fn sel_range(&self) -> (usize, usize) {
        (self.cur.min(self.anchor), self.cur.max(self.anchor))
    }

    fn has_sel(&self) -> bool {
        self.cur != self.anchor
    }

    fn peek(&self, at: usize) -> Option<char> {
        (at < self.rope.len_chars()).then(|| self.rope.char(at))
    }

    fn set(&mut self, cur: usize) {
        self.cur = cur;
        self.anchor = cur;
    }

    fn begin(&mut self) {
        self.before = (self.cur, self.anchor);
    }

    fn edit(&mut self, a: usize, b: usize, ins: &str) {
        if a == b && ins.is_empty() {
            return;
        }
        let del: String = self.rope.slice(a..b).into();
        let line = self.rope.char_to_line(a);
        let col0 = a == self.rope.line_to_char(line);
        if a != b {
            self.rope.remove(a..b);
        }
        if !ins.is_empty() {
            self.rope.insert(a, ins);
        }
        let col = a - self.rope.line_to_char(line);
        let (removed, added) = (del.matches('\n').count(), ins.matches('\n').count());
        self.shift_sem(
            line,
            col,
            del.chars().count(),
            ins.chars().count(),
            removed,
            added,
        );
        self.shift_marks(line, col0, removed, added);
        self.states.truncate(line + 1);
        self.rev += 1;
        self.open_ops.push(Op {
            at: a,
            del,
            ins: ins.to_string(),
        });
    }

    fn commit(&mut self, typing: bool) {
        if self.open_ops.is_empty() {
            return;
        }
        let ops = std::mem::take(&mut self.open_ops);
        self.undo_bytes += ops
            .iter()
            .map(|o| o.del.len() + o.ins.len() + 32)
            .sum::<usize>();
        self.redo.clear();
        let after = (self.cur, self.anchor);
        let word_break = ops.iter().any(|o| o.ins.chars().any(char::is_whitespace));
        if let Some(last) = self.undo.last_mut() {
            if typing
                && last.typing
                && !word_break
                && last.after == self.before
                && last.at.elapsed().as_millis() < COALESCE_MS
            {
                last.ops.extend(ops);
                last.after = after;
                last.at = Instant::now();
                return;
            }
        }
        self.undo.push(Txn {
            ops,
            before: self.before,
            after,
            typing,
            at: Instant::now(),
        });
        while self.undo_bytes > MAX_UNDO_BYTES && self.undo.len() > 1 {
            let old = self.undo.remove(0);
            self.undo_bytes -= old
                .ops
                .iter()
                .map(|o| o.del.len() + o.ins.len() + 32)
                .sum::<usize>()
                .min(self.undo_bytes);
        }
    }

    fn splice(&mut self, op_at: usize, remove: usize, text: &str) {
        let line = self.rope.char_to_line(op_at);
        let col0 = op_at == self.rope.line_to_char(line);
        let removed = self
            .rope
            .slice(op_at..op_at + remove)
            .chars()
            .filter(|c| *c == '\n')
            .count();
        let col = op_at - self.rope.line_to_char(line);
        self.shift_sem(
            line,
            col,
            remove,
            text.chars().count(),
            removed,
            text.matches('\n').count(),
        );
        self.shift_marks(line, col0, removed, text.matches('\n').count());
        if remove > 0 {
            self.rope.remove(op_at..op_at + remove);
        }
        if !text.is_empty() {
            self.rope.insert(op_at, text);
        }
        self.states.truncate(line + 1);
        self.rev += 1;
    }

    fn undo(&mut self) {
        if let Some(t) = self.undo.pop() {
            for op in t.ops.iter().rev() {
                self.splice(op.at, op.ins.chars().count(), &op.del);
            }
            self.cur = t.before.0;
            self.anchor = t.before.1;
            self.redo.push(t);
        }
    }

    fn redo(&mut self) {
        if let Some(t) = self.redo.pop() {
            for op in &t.ops {
                self.splice(op.at, op.del.chars().count(), &op.ins);
            }
            self.cur = t.after.0;
            self.anchor = t.after.1;
            self.undo.push(t);
        }
    }

    fn replace_sel(&mut self, s: &str) {
        let (a, b) = self.sel_range();
        self.edit(a, b, s);
        self.set(a + s.chars().count());
    }

    fn indent_unit(&self) -> String {
        if self.use_tabs {
            "\t".into()
        } else {
            " ".repeat(self.tab)
        }
    }

    fn leading_ws(&self, line: usize) -> String {
        self.rope
            .line(line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    fn type_text(&mut self, s: &str) {
        let mut it = s.chars();
        let (Some(c), None) = (it.next(), it.next()) else {
            self.replace_sel(s);
            return;
        };
        let next = self.peek(self.cur);
        let prev = (self.cur > 0).then(|| self.rope.char(self.cur - 1));

        if self.lang != Lang::Plain {
            if self.has_sel() && (closer(c).is_some() || c == '"' || c == '\'') {
                let (a, b) = self.sel_range();
                let close = closer(c).unwrap_or(c);
                self.edit(b, b, &close.to_string());
                self.edit(a, a, &c.to_string());
                self.anchor = a + 1;
                self.cur = b + 1;
                return;
            }
            if !self.has_sel() {
                if next == Some(c) && matches!(c, ')' | ']' | '}' | '"' | '\'') {
                    self.set(self.cur + 1);
                    return;
                }
                let free_after = next
                    .is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | ';'));
                if let Some(close) = closer(c) {
                    if free_after {
                        let at = self.cur;
                        self.edit(at, at, &format!("{c}{close}"));
                        self.set(at + 1);
                        return;
                    }
                }
                if (c == '"' || c == '\'') && free_after && !prev.is_some_and(is_word) {
                    let at = self.cur;
                    self.edit(at, at, &format!("{c}{c}"));
                    self.set(at + 1);
                    return;
                }
                if c == '}' {
                    let (line, col) = self.pos(self.cur);
                    let start = self.rope.line_to_char(line);
                    let only_ws = self
                        .rope
                        .slice(start..self.cur)
                        .chars()
                        .all(|c| c == ' ' || c == '\t');
                    if only_ws && col > 0 {
                        let drop = if self.rope.char(self.cur - 1) == '\t' {
                            1
                        } else {
                            col.min(self.tab)
                        };
                        let at = self.cur - drop;
                        self.edit(at, self.cur, "}");
                        self.set(at + 1);
                        return;
                    }
                }
            }
        }
        self.replace_sel(s);
    }

    fn newline(&mut self) {
        let (a, b) = self.sel_range();
        let (line, col) = self.pos(a);
        let mut indent = self.leading_ws(line);
        indent.truncate(indent.chars().take(col).map(char::len_utf8).sum());
        let unit = self.indent_unit();
        let prev = (a > 0).then(|| self.rope.char(a - 1));
        let next = self.peek(b);
        if self.lang != Lang::Plain && prev == Some('{') {
            if next == Some('}') {
                let text = format!("\n{indent}{unit}\n{indent}");
                self.edit(a, b, &text);
                self.set(a + 1 + indent.chars().count() + unit.chars().count());
            } else {
                self.replace_sel(&format!("\n{indent}{unit}"));
            }
        } else {
            self.replace_sel(&format!("\n{indent}"));
        }
    }

    fn move_selection(&mut self, line: usize, vcol: usize, copy: bool) {
        if !self.has_sel() {
            return;
        }
        let (a, b) = self.sel_range();
        let line = self.row_to_line(line);
        let at = self.at(line, self.col_from_vcol(line, vcol));
        if at > a && at < b || (!copy && (at == a || at == b)) {
            return;
        }
        let text: String = self.rope.slice(a..b).into();
        let len = b - a;
        let start = if copy {
            self.edit(at, at, &text);
            at
        } else if at >= b {
            self.edit(at, at, &text);
            self.edit(a, b, "");
            at - len
        } else {
            self.edit(a, b, "");
            self.edit(at, at, &text);
            at
        };
        self.anchor = start;
        self.cur = start + len;
    }

    fn line_end(&self, line: usize) -> usize {
        self.rope.line_to_char(line) + self.line_len(line)
    }

    fn new_line_below(&mut self) {
        let (line, _) = self.pos(self.cur);
        self.set(self.line_end(line));
        self.newline();
    }

    fn new_line_above(&mut self) {
        let (line, _) = self.pos(self.cur);
        let start = self.rope.line_to_char(line);
        let indent = self.leading_ws(line);
        self.edit(start, start, &format!("{indent}\n"));
        self.set(start + indent.chars().count());
    }

    fn complete_statement(&mut self) {
        let (line, _) = self.pos(self.cur);
        let text = self.line_text(line).into_owned();
        let code = text.trim();
        let end = self.rope.line_to_char(line) + text.trim_end().chars().count();
        self.set(end);
        let finished = code.is_empty()
            || code.starts_with('#')
            || code.contains("//")
            || code.ends_with(['}', '{', ';', ',', '(', ':', '\\']);
        if self.lang == Lang::Plain || finished {
            self.new_line_below();
            return;
        }
        let keyword = code
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap_or("");
        if matches!(keyword, "if" | "for" | "while" | "switch") && code.ends_with(')') {
            let indent = self.leading_ws(line);
            let unit = self.indent_unit();
            self.edit(end, end, &format!(" {{\n{indent}{unit}\n{indent}}}"));
            self.set(end + 3 + indent.chars().count() + unit.chars().count());
            return;
        }
        self.edit(end, end, ";");
        self.set(end + 1);
        self.new_line_below();
    }

    fn toggle_case(&mut self) {
        let keep_selection = self.has_sel();
        let (a, b) = if keep_selection {
            self.sel_range()
        } else {
            match self.word_range(self.cur) {
                Some(range) => range,
                None => return,
            }
        };
        let text: String = self.rope.slice(a..b).into();
        let next = if text.chars().any(char::is_lowercase) {
            text.to_uppercase()
        } else {
            text.to_lowercase()
        };
        if next == text {
            return;
        }
        let cur = self.cur;
        self.edit(a, b, &next);
        let end = a + next.chars().count();
        if keep_selection {
            self.anchor = a;
            self.cur = end;
        } else {
            self.set(cur.min(end));
        }
    }

    fn goto_bracket(&mut self) {
        let Some((a, b)) = self.matching_bracket() else {
            return;
        };
        let target = if self.cur == a || self.cur == a + 1 {
            b
        } else {
            a
        };
        self.set(target);
    }

    fn backspace(&mut self) {
        if self.has_sel() {
            self.replace_sel("");
            return;
        }
        if self.cur == 0 {
            return;
        }
        let prev = self.rope.char(self.cur - 1);
        let next = self.peek(self.cur);
        if self.lang != Lang::Plain
            && next.is_some()
            && (closer(prev) == next || (matches!(prev, '"' | '\'') && next == Some(prev)))
        {
            let a = self.cur - 1;
            self.edit(a, a + 2, "");
            self.set(a);
            return;
        }
        let (line, col) = self.pos(self.cur);
        let start = self.rope.line_to_char(line);
        if prev == ' '
            && !self.use_tabs
            && self.rope.slice(start..self.cur).chars().all(|c| c == ' ')
        {
            let drop = match col % self.tab {
                0 => self.tab,
                r => r,
            };
            let a = self.cur - drop;
            self.edit(a, self.cur, "");
            self.set(a);
            return;
        }
        let a = self.cur - 1;
        self.edit(a, self.cur, "");
        self.set(a);
    }

    fn sel_lines(&self) -> (usize, usize) {
        let (a, b) = self.sel_range();
        let (first, _) = self.pos(a);
        let (mut last, col) = self.pos(b);
        if last > first && col == 0 {
            last -= 1;
        }
        (first, last)
    }

    fn indent_lines(&mut self, remove: bool) {
        let (first, last) = self.sel_lines();
        let unit = self.indent_unit();
        let (mut cur, mut anchor) = (self.cur, self.anchor);
        for line in (first..=last).rev() {
            let start = self.rope.line_to_char(line);
            let (a, b, text): (usize, usize, &str) = if remove {
                let chars: Vec<char> = self.rope.line(line).chars().take(self.tab).collect();
                let n = if chars.first() == Some(&'\t') {
                    1
                } else {
                    chars.iter().take_while(|c| **c == ' ').count()
                };
                (start, start + n, "")
            } else {
                if self.line_len(line) == 0 {
                    continue;
                }
                (start, start, &unit)
            };
            self.edit(a, b, text);
            let delta = text.chars().count() as isize - (b - a) as isize;
            let shift = |p: usize| {
                if p >= b {
                    (p as isize + delta) as usize
                } else if p > a {
                    a
                } else {
                    p
                }
            };
            cur = shift(cur);
            anchor = shift(anchor);
        }
        self.cur = cur;
        self.anchor = anchor;
    }

    fn toggle_comment(&mut self) {
        let Some(mark) = self.lang.line_comment() else {
            return;
        };
        let (first, last) = self.sel_lines();
        let lines: Vec<usize> = (first..=last)
            .filter(|&l| !self.line_text(l).trim().is_empty())
            .collect();
        if lines.is_empty() {
            return;
        }
        let all = lines
            .iter()
            .all(|&l| self.line_text(l).trim_start().starts_with(mark));
        let col = lines
            .iter()
            .map(|&l| self.leading_ws(l).chars().count())
            .min()
            .unwrap_or(0);
        let (mut cur, mut anchor) = (self.cur, self.anchor);
        for &line in lines.iter().rev() {
            let start = self.rope.line_to_char(line);
            let (a, b, ins) = if all {
                let text = self.line_text(line);
                let at = text.chars().take_while(|c| c.is_whitespace()).count();
                let spaced = text.trim_start()[mark.len()..].starts_with(' ');
                (
                    start + at,
                    start + at + mark.len() + spaced as usize,
                    String::new(),
                )
            } else {
                (start + col, start + col, format!("{mark} "))
            };
            self.edit(a, b, &ins);
            let delta = ins.chars().count() as isize - (b - a) as isize;
            let shift = |p: usize| {
                if p >= b {
                    (p as isize + delta) as usize
                } else if p > a {
                    a
                } else {
                    p
                }
            };
            cur = shift(cur);
            anchor = shift(anchor);
        }
        self.cur = cur;
        self.anchor = anchor;
    }

    fn duplicate(&mut self) {
        if self.has_sel() {
            let (a, b) = self.sel_range();
            let text: String = self.rope.slice(a..b).into();
            self.edit(b, b, &text);
            self.anchor = b;
            self.cur = b + text.chars().count();
            return;
        }
        let (line, col) = self.pos(self.cur);
        let text = self.line_text(line).into_owned();
        let end = self.rope.line_to_char(line) + self.line_len(line);
        self.edit(end, end, &format!("\n{text}"));
        let next = self.at(line + 1, col);
        self.set(next);
    }

    fn delete_lines(&mut self) {
        let (first, last) = self.sel_lines();
        let a = self.rope.line_to_char(first);
        let total = self.rope.len_lines();
        let b = if last + 1 < total {
            self.rope.line_to_char(last + 1)
        } else {
            self.rope.len_chars()
        };
        let (a, keep) = if last + 1 >= total && a > 0 {
            (a - 1, first - 1)
        } else {
            (a, first)
        };
        self.edit(a, b, "");
        let to = self.at(keep, 0);
        self.set(to);
    }

    fn move_lines(&mut self, up: bool) {
        let (first, last) = self.sel_lines();
        let total = self.rope.len_lines();
        if (up && first == 0) || (!up && last + 1 >= total) {
            return;
        }
        let (a_line, b_line) = if up {
            (first - 1, last)
        } else {
            (first, last + 1)
        };
        let start = self.rope.line_to_char(a_line);
        let end = self.rope.line_to_char(b_line) + self.line_len(b_line);
        let lines: Vec<String> = (a_line..=b_line)
            .map(|l| self.line_text(l).into_owned())
            .collect();
        let mut out = lines.clone();
        out.rotate_left(if up { 1 } else { lines.len() - 1 });
        let hop = if up {
            lines[0].chars().count() as isize + 1
        } else {
            lines[lines.len() - 1].chars().count() as isize + 1
        };
        let delta = if up { -hop } else { hop };
        let (cur, anchor) = (self.cur, self.anchor);
        self.edit(start, end, &out.join("\n"));
        self.cur = (cur as isize + delta) as usize;
        self.anchor = (anchor as isize + delta) as usize;
    }

    fn join_lines(&mut self) {
        let (line, _) = self.pos(self.cur);
        if line + 1 >= self.rope.len_lines() {
            return;
        }
        let end = self.rope.line_to_char(line) + self.line_len(line);
        let ws = self.leading_ws(line + 1).chars().count();
        self.edit(end, end + 1 + ws, " ");
        self.set(end + 1);
    }

    fn expand_selection(&mut self) {
        let (a, b) = self.sel_range();
        self.sel_stack.push((self.cur, self.anchor));
        if a == b {
            if let Some((wa, wb)) = self.word_range(a) {
                self.anchor = wa;
                self.cur = wb;
                return;
            }
        }
        let n = self.rope.len_chars();
        let mut depth = 0;
        let mut i = a;
        let mut open = None;
        while i > 0 && a - i < 200_000 {
            i -= 1;
            match self.rope.char(i) {
                ')' | ']' | '}' => depth += 1,
                '(' | '[' | '{' => {
                    if depth == 0 {
                        open = Some(i);
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        let close = open.and_then(|o| {
            let mut depth = 0;
            for i in b..n.min(b + 200_000) {
                match self.rope.char(i) {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => {
                        if depth == 0 {
                            return Some((o, i));
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
            }
            None
        });
        let (na, nb) = match close {
            Some((o, c)) if (o + 1, c) != (a, b) => (o + 1, c),
            Some((o, c)) => (o, c + 1),
            None => (0, n),
        };
        self.anchor = na;
        self.cur = nb;
    }

    fn matching_bracket(&mut self) -> Option<(usize, usize)> {
        let key = (self.cur, self.rev);
        if let Some((k, v)) = self.pair_cache {
            if k == key {
                return v;
            }
        }
        const LIMIT: usize = 20_000;
        let n = self.rope.len_chars();
        let at = |i: usize| (i < n).then(|| self.rope.char(i));
        let opposite = |c: char| match c {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        };
        let mut found = None;
        let candidates = [Some(self.cur), self.cur.checked_sub(1)];
        for start in candidates.into_iter().flatten() {
            let Some(c) = at(start) else { continue };
            if let Some(close) = closer(c) {
                let mut depth = 0;
                for i in start + 1..n.min(start + LIMIT) {
                    let d = self.rope.char(i);
                    if d == c {
                        depth += 1;
                    } else if d == close {
                        if depth == 0 {
                            found = Some((start, i));
                            break;
                        }
                        depth -= 1;
                    }
                }
                break;
            }
            if let Some(open) = opposite(c) {
                let mut depth = 0;
                for i in (start.saturating_sub(LIMIT)..start).rev() {
                    let d = self.rope.char(i);
                    if d == c {
                        depth += 1;
                    } else if d == open {
                        if depth == 0 {
                            found = Some((i, start));
                            break;
                        }
                        depth -= 1;
                    }
                }
                break;
            }
        }
        self.pair_cache = Some((key, found));
        found
    }

    pub fn selection_lines(&self) -> Option<(usize, usize)> {
        self.has_sel().then(|| self.sel_lines())
    }

    pub fn apply_byte_replacements(&mut self, mut edits: Vec<(usize, usize, String)>) -> usize {
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        let total = self.rope.len_bytes();
        let count = edits.len();
        let mut cur = self.cur;
        self.begin();
        for (offset, length, text) in edits {
            if offset + length > total {
                continue;
            }
            let a = self.rope.byte_to_char(offset);
            let b = self.rope.byte_to_char(offset + length);
            self.edit(a, b, &text);
            let inserted = text.chars().count();
            if cur >= b {
                cur = cur + inserted - (b - a);
            } else if cur > a {
                cur = a + inserted.min(cur - a);
            }
        }
        self.set(cur);
        self.commit(false);
        count
    }

    pub fn apply_utf16_edits(&mut self, mut edits: Vec<(usize, usize, usize, usize, String)>) {
        edits.sort_by_key(|e| std::cmp::Reverse((e.0, e.1)));
        let (line, col) = self.pos(self.cur);
        self.begin();
        for (sl, sc, el, ec, text) in edits {
            let a = self.at(sl, self.col_from_utf16(sl, sc));
            let b = self.at(el, self.col_from_utf16(el, ec));
            self.edit(a, b, &text);
        }
        let to = self.at(line, col);
        self.set(to);
        self.commit(false);
    }

    fn word_left(&self) -> usize {
        let mut i = self.cur;
        while i > 0 && self.rope.char(i - 1).is_whitespace() && self.rope.char(i - 1) != '\n' {
            i -= 1;
        }
        if i == self.cur && i > 0 && self.rope.char(i - 1) == '\n' {
            return i - 1;
        }
        if i > 0 {
            let w = is_word(self.rope.char(i - 1));
            while i > 0 {
                let c = self.rope.char(i - 1);
                if c.is_whitespace() || is_word(c) != w {
                    break;
                }
                i -= 1;
            }
        }
        i
    }

    fn word_right(&self) -> usize {
        let n = self.rope.len_chars();
        let mut i = self.cur;
        if i < n && self.rope.char(i) == '\n' {
            return i + 1;
        }
        if i < n && !self.rope.char(i).is_whitespace() {
            let w = is_word(self.rope.char(i));
            while i < n {
                let c = self.rope.char(i);
                if c.is_whitespace() || is_word(c) != w {
                    break;
                }
                i += 1;
            }
        }
        while i < n && self.rope.char(i).is_whitespace() && self.rope.char(i) != '\n' {
            i += 1;
        }
        i
    }

    fn motion(&mut self, to: Motion, ext: bool, rows: usize) {
        use Motion::*;
        let (line, col) = self.pos(self.cur);
        let last = self.rope.len_lines() - 1;
        let vertical = matches!(to, Up | Down | PageUp | PageDown);
        let want = self.want.unwrap_or_else(|| self.vcol(line, col));
        let target = match to {
            Left if self.has_sel() && !ext => self.sel_range().0,
            Right if self.has_sel() && !ext => self.sel_range().1,
            Left => self.cur.saturating_sub(1),
            Right => (self.cur + 1).min(self.rope.len_chars()),
            Up if line == 0 => 0,
            Down if line == last => self.rope.len_chars(),
            Up | PageUp => {
                let r = self
                    .line_to_row(line)
                    .saturating_sub(if to == Up { 1 } else { rows });
                let l = self.row_to_line(r);
                self.at(l, self.col_from_vcol(l, want))
            }
            Down | PageDown => {
                let r = self.line_to_row(line) + if to == Down { 1 } else { rows };
                let l = self.row_to_line(r.min(self.row_count() - 1));
                self.at(l, self.col_from_vcol(l, want))
            }
            Home => {
                let first = self.leading_ws(line).chars().count();
                self.at(line, if col == first { 0 } else { first })
            }
            End => self.at(line, usize::MAX),
            WordLeft => self.word_left(),
            WordRight => self.word_right(),
            DocStart => 0,
            DocEnd => self.rope.len_chars(),
        };
        self.cur = target;
        if !ext {
            self.anchor = target;
        }
        self.want = vertical.then_some(want);
    }

    fn select_word(&mut self, line: usize, vcol: usize) {
        let line = line.min(self.rope.len_lines() - 1);
        let at = self.at(line, self.col_from_vcol(line, vcol));
        let n = self.rope.len_chars();
        let on = |i: usize| i < n && is_word(self.rope.char(i));
        if !on(at) && !(at > 0 && on(at - 1)) {
            self.set(at);
            return;
        }
        let mut a = at;
        while a > 0 && on(a - 1) {
            a -= 1;
        }
        let mut b = at;
        while on(b) {
            b += 1;
        }
        self.anchor = a;
        self.cur = b;
    }

    fn matches_in(&self, text: &str) -> Vec<usize> {
        let q = &self.query;
        if q.is_empty() || q.len() > text.len() {
            return vec![];
        }
        let (hay, needle) = (text.as_bytes(), q.as_bytes());
        let eq = |a: u8, b: u8| {
            if self.case {
                a == b
            } else {
                a.eq_ignore_ascii_case(&b)
            }
        };
        let mut out = vec![];
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            if hay[i..i + needle.len()]
                .iter()
                .zip(needle)
                .all(|(a, b)| eq(*a, *b))
                && text.is_char_boundary(i)
            {
                out.push(i);
                i += needle.len();
            } else {
                i += 1;
            }
        }
        out
    }

    pub fn set_search(&mut self, q: &str, case: bool) -> usize {
        self.query = q.lines().next().unwrap_or("").to_string();
        self.case = case;
        (0..self.rope.len_lines())
            .map(|l| self.matches_in(&self.line_text(l)).len())
            .sum()
    }

    fn select_match(&mut self, line: usize, byte: usize, text: &str) {
        let a = self.rope.line_to_char(line) + text[..byte].chars().count();
        self.anchor = a;
        self.cur = a + self.query.chars().count();
    }

    fn find_next(&mut self, back: bool) -> bool {
        let total = self.rope.len_lines();
        let (a, b) = self.sel_range();
        let from = if back { a } else { b };
        let (line, col) = self.pos(from);
        for step in 0..=total {
            let l = if back {
                (line + total * 2 - step) % total
            } else {
                (line + step) % total
            };
            let text = self.line_text(l).into_owned();
            let at_col = |m: usize| text[..m].chars().count();
            let found = self.matches_in(&text);
            let pick = if back {
                found.iter().rev().find(|&&m| step > 0 || at_col(m) < col)
            } else {
                found.iter().find(|&&m| step > 0 || at_col(m) >= col)
            };
            if let Some(&m) = pick {
                self.select_match(l, m, &text);
                return true;
            }
        }
        false
    }

    fn sel_is_match(&self) -> bool {
        if !self.has_sel() {
            return false;
        }
        let (a, b) = self.sel_range();
        let s: String = self.rope.slice(a..b).into();
        if self.case {
            s == self.query
        } else {
            s.eq_ignore_ascii_case(&self.query)
        }
    }

    fn replace_all(&mut self, with: &str) {
        for l in (0..self.rope.len_lines()).rev() {
            let text = self.line_text(l).into_owned();
            let qlen = self.query.chars().count();
            for &m in self.matches_in(&text).iter().rev() {
                let a = self.rope.line_to_char(l) + text[..m].chars().count();
                self.edit(a, a + qlen, with);
            }
        }
        let c = self.cur.min(self.rope.len_chars());
        self.set(c);
    }

    pub fn run(&mut self, cmd: Cmd) -> Result<(), String> {
        use Cmd::*;
        if self.readonly && cmd.mutates() {
            return Ok(());
        }
        let keeps_want = matches!(cmd, Move { .. });
        if !matches!(cmd, ExpandSelection | ShrinkSelection) {
            self.sel_stack.clear();
        }
        let mut typing = false;
        self.begin();
        match cmd {
            Type { s } => {
                self.type_text(&s);
                typing = true;
            }
            Insert { s } => {
                let s = s.replace("\r\n", "\n");
                self.replace_sel(&s);
            }
            Newline => self.newline(),
            Backspace => {
                self.backspace();
                typing = true;
            }
            Delete => {
                if self.has_sel() {
                    self.replace_sel("");
                } else if self.cur < self.rope.len_chars() {
                    self.edit(self.cur, self.cur + 1, "");
                }
                typing = true;
            }
            DeleteWordLeft => {
                if !self.has_sel() {
                    self.anchor = self.word_left();
                }
                self.replace_sel("");
            }
            DeleteWordRight => {
                if !self.has_sel() {
                    self.anchor = self.word_right();
                }
                self.replace_sel("");
            }
            Tab => {
                if self.has_sel() && self.pos(self.cur).0 != self.pos(self.anchor).0 {
                    self.indent_lines(false);
                } else if self.use_tabs {
                    self.replace_sel("\t");
                } else {
                    let (a, _) = self.sel_range();
                    let (line, col) = self.pos(a);
                    let v = self.vcol(line, col);
                    self.replace_sel(&" ".repeat(self.tab - v % self.tab));
                }
            }
            Outdent => self.indent_lines(true),
            Move { to, ext, rows } => self.motion(to, ext, rows.unwrap_or(30)),
            Click { line, vcol, ext } => {
                let line = self.row_to_line(line);
                self.cur = self.at(line, self.col_from_vcol(line, vcol));
                if !ext {
                    self.anchor = self.cur;
                }
            }
            Word { line, vcol } => {
                let line = self.row_to_line(line);
                self.select_word(line, vcol);
            }
            SelectLine { line } => {
                let line = self.row_to_line(line);
                self.anchor = self.rope.line_to_char(line);
                self.cur = if line + 1 < self.rope.len_lines() {
                    self.rope.line_to_char(line + 1)
                } else {
                    self.rope.len_chars()
                };
            }
            SelectAll => {
                self.anchor = 0;
                self.cur = self.rope.len_chars();
            }
            MoveSelection { line, vcol, copy } => self.move_selection(line, vcol, copy),
            NewLineBelow => self.new_line_below(),
            NewLineAbove => self.new_line_above(),
            CompleteStatement => self.complete_statement(),
            ToggleCase => self.toggle_case(),
            GotoBracket => self.goto_bracket(),
            Undo => self.undo(),
            Redo => self.redo(),
            ToggleComment => self.toggle_comment(),
            DuplicateLine => self.duplicate(),
            DeleteLine => self.delete_lines(),
            CutSelection => {
                if self.has_sel() {
                    self.replace_sel("");
                } else {
                    self.delete_lines();
                }
            }
            Complete { s } => {
                let n = self.prefix().chars().count();
                let (a, b) = self.sel_range();
                self.edit(a - n, b, &s);
                self.set(a - n + s.chars().count());
            }
            Goto { line, col } => {
                let c = self.col_from_utf16(line, col);
                let at = self.at(line, c);
                self.set(at);
            }
            FindNext { back } => {
                self.find_next(back);
            }
            ReplaceOne { s } => {
                if self.sel_is_match() {
                    self.replace_sel(&s);
                }
                self.find_next(false);
            }
            ReplaceAll { s } => self.replace_all(&s),
            Fresh => self.sent_clip = None,
            MoveLines { up } => self.move_lines(up),
            JoinLines => self.join_lines(),
            ExpandSelection => self.expand_selection(),
            ShrinkSelection => {
                if let Some((c, a)) = self.sel_stack.pop() {
                    self.cur = c;
                    self.anchor = a;
                }
            }
            ToggleFold { row } => {
                let line = self.row_to_line(row);
                let folded = self.folds.iter().any(|f| f.0 == line);
                self.set_fold(line, !folded);
            }
            FoldCaret { fold } => self.fold_caret(fold),
            FoldAll { fold } => self.fold_all(fold),
            ToggleBreakpoint { row } => {
                let line = row.map_or_else(|| self.pos(self.cur).0, |r| self.row_to_line(r));
                self.toggle_breakpoint(line);
            }
            SetBreakpoints { lines } => {
                self.bps = lines;
                self.bps.sort_unstable();
                self.bps.dedup();
            }
            SetExec { line } => self.exec = line,
        }
        self.commit(typing);
        if !keeps_want {
            self.want = None;
        }
        Ok(())
    }

    fn ensure_states(&mut self, upto: usize) {
        let mut scratch = Vec::new();
        while self.states.len() <= upto {
            let i = self.states.len() - 1;
            let st = self.states[i];
            scratch.clear();
            let text: Cow<str> = self.rope.line(i).into();
            let next = lexer::lex(
                text.trim_end_matches('\n'),
                st,
                self.lang,
                &mut self.raws,
                &mut scratch,
            );
            self.states.push(next);
        }
    }

    fn emit(
        &self,
        s: &str,
        tag: Option<u8>,
        vcol: &mut usize,
        out: &mut String,
        budget: &mut usize,
    ) {
        if s.is_empty() || *budget == 0 {
            return;
        }
        if let Some(t) = tag {
            out.push('<');
            out.push(t as char);
            out.push('>');
        }
        for c in s.chars() {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '\t' => {
                    let next = self.step(*vcol, c);
                    out.extend(std::iter::repeat_n(' ', next - *vcol));
                    *vcol = next;
                    continue;
                }
                c if (c as u32) < 0x20 => out.push('\u{fffd}'),
                c => out.push(c),
            }
            *vcol += 1;
        }
        if let Some(t) = tag {
            out.push_str("</");
            out.push(t as char);
            out.push('>');
        }
    }

    fn neighbour_indent(&self, line: usize, up: bool) -> usize {
        let total = self.rope.len_lines();
        let mut l = line;
        for _ in 0..16 {
            if up {
                if l == 0 {
                    return 0;
                }
                l -= 1;
            } else {
                l += 1;
                if l >= total {
                    return 0;
                }
            }
            let text = self.line_text(l);
            if !text.trim().is_empty() {
                return text
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .fold(0, |v, c| self.step(v, c));
            }
        }
        0
    }

    fn push_guides(&self, out: &mut String, vcols: usize) {
        for unit in 0..vcols / self.tab {
            if unit > 0 {
                out.push_str("<g>");
            }
            out.extend(std::iter::repeat_n(' ', self.tab));
            if unit > 0 {
                out.push_str("</g>");
            }
        }
        out.extend(std::iter::repeat_n(' ', vcols % self.tab));
    }

    fn render_line(&mut self, i: usize, toks: &mut Vec<Tok>) -> (String, bool) {
        self.ensure_states(i);
        toks.clear();
        let text = self.line_text(i).into_owned();
        let st = self.states[i];
        let next = lexer::lex(&text, st, self.lang, &mut self.raws, toks);
        if self.states.len() == i + 1 {
            self.states.push(next);
        }
        self.overlay_semantic(i, &text, toks);
        let code_lang = matches!(self.lang, Lang::C | Lang::Cpp | Lang::CMake);
        let mut out = String::with_capacity(text.len() + 16 * toks.len());
        let (mut pos, mut vcol) = (0usize, 0usize);
        let mut budget = MAX_RENDER;
        if code_lang {
            if text.is_empty() {
                let indent = self
                    .neighbour_indent(i, true)
                    .min(self.neighbour_indent(i, false));
                self.push_guides(&mut out, indent);
            } else {
                let first_tok = toks.first().map_or(text.len(), |t| t.start as usize);
                let lead = text
                    .bytes()
                    .take(first_tok)
                    .take_while(|b| *b == b' ' || *b == b'\t')
                    .count();
                let v = text[..lead].chars().fold(0, |v, c| self.step(v, c));
                self.push_guides(&mut out, v);
                pos = lead;
                vcol = v;
            }
        }
        for t in toks.iter() {
            let (s, e) = (t.start as usize, t.end.min(text.len() as u32) as usize);
            self.emit(&text[pos..s], None, &mut vcol, &mut out, &mut budget);
            self.emit(&text[s..e], Some(t.tag), &mut vcol, &mut out, &mut budget);
            pos = e;
        }
        self.emit(&text[pos..], None, &mut vcol, &mut out, &mut budget);
        self.maxw = self.maxw.max(vcol);

        let last_code = text.bytes().enumerate().rev().find(|(i, b)| {
            !b.is_ascii_whitespace()
                && !toks.iter().any(|t| {
                    (t.start as usize..t.end as usize).contains(i) && t.tag == lexer::COMMENT
                })
        });
        let foldable = code_lang
            && (last_code.is_some_and(|(_, b)| b == b'{') || (st == 0 && next == lexer::BLOCK));
        (out, foldable)
    }

    fn vcol_of_byte(&self, text: &str, byte: usize) -> usize {
        text[..byte].chars().fold(0, |v, c| self.step(v, c))
    }

    pub fn view(&mut self, top: usize, rows: usize, reveal: bool) -> View {
        let (cl, cc) = self.pos(self.cur);
        self.folds.retain(|f| !(cl > f.0 && cl <= f.1));
        let n = self.row_count();
        let total = self.rope.len_lines();
        let cr = self.line_to_row(cl);
        let mut top = top.min(n.saturating_sub(1));
        if reveal {
            if cr < top {
                top = cr;
            } else if rows > 0 && cr >= top + rows {
                top = cr + 1 - rows;
            }
        }
        let first = top.saturating_sub(OVERSCAN);
        let end = (top + rows + OVERSCAN).min(n);

        let mut toks = Vec::new();
        let mut lines = Vec::with_capacity(end - first);
        let mut nums = Vec::with_capacity(end - first);
        let mut folds = vec![];
        for row in first..end {
            let line = self.row_to_line(row);
            let (mut html, foldable) = self.render_line(line, &mut toks);
            let folded = self.folds.iter().any(|f| f.0 == line);
            if folded {
                html.push_str("<e> \u{2026} </e>");
            }
            if folded || foldable {
                folds.push([row, if folded { 2 } else { 1 }]);
            }
            lines.push(html);
            nums.push(line);
        }
        let in_view = |nums: &[usize], l: usize| nums.binary_search(&l).ok().map(|i| first + i);

        let (sa, sb) = self.sel_range();
        let mut sel = vec![];
        if sa != sb {
            let (al, ac) = self.pos(sa);
            let (bl, bc) = self.pos(sb);
            for (i, &l) in nums.iter().enumerate() {
                if l < al || l > bl {
                    continue;
                }
                let vs = if l == al { self.vcol(l, ac) } else { 0 };
                let ve = if l == bl {
                    self.vcol(l, bc)
                } else {
                    self.vcol(l, self.line_len(l)) + 1
                };
                sel.push([first + i, vs, ve]);
            }
        }

        let mut marks = vec![];
        if !self.query.is_empty() {
            let qw = self.query.chars().count();
            for (i, &l) in nums.iter().enumerate() {
                let text = self.line_text(l).into_owned();
                for m in self.matches_in(&text) {
                    let v = self.vcol_of_byte(&text, m);
                    marks.push([first + i, v, v + qw]);
                }
            }
        }

        let mut occ = vec![];
        if sa == sb && self.lang != Lang::Diff {
            if let Some((a, b)) = self.word_range(self.cur).filter(|(a, b)| b - a >= 2) {
                let word: String = self.rope.slice(a..b).into();
                let chars = word.chars().count();
                for (i, &l) in nums.iter().enumerate() {
                    let text = self.line_text(l).into_owned();
                    for (m, _) in text.match_indices(&word) {
                        let before = text[..m].chars().next_back().is_some_and(is_word);
                        let after = text[m + word.len()..].chars().next().is_some_and(is_word);
                        if !before && !after {
                            let v = self.vcol_of_byte(&text, m);
                            occ.push([first + i, v, v + chars]);
                        }
                    }
                }
            }
        }

        let mut pair = vec![];
        if let Some((a, b)) = self.matching_bracket() {
            for p in [a, b] {
                let (l, c) = self.pos(p);
                if let Some(r) = in_view(&nums, l) {
                    pair.push([r, self.vcol(l, c)]);
                }
            }
        }

        let mut diags = vec![];
        let (mut errors, mut warnings) = (0, 0);
        let mut msg = None;
        for d in &self.diags {
            match d.sev {
                1 => errors += 1,
                2 => warnings += 1,
                _ => {}
            }
            let here = (d.sl, d.sc) <= (cl, cc) && (cl, cc) <= (d.el, d.ec);
            if here && msg.is_none() {
                msg = Some(d.msg.clone());
            }
            if d.sl >= total {
                continue;
            }
            for (i, &l) in nums.iter().enumerate() {
                if l < d.sl || l > d.el {
                    continue;
                }
                let len = self.line_len(l);
                let vs = if l == d.sl {
                    self.vcol(l, d.sc.min(len))
                } else {
                    0
                };
                let ve = if l == d.el {
                    self.vcol(l, d.ec.min(len))
                } else {
                    self.vcol(l, len)
                };
                diags.push([first + i, vs, ve.max(vs + 1), d.sev]);
            }
        }

        let bps = self.bps.iter().filter_map(|&l| in_view(&nums, l)).collect();
        let mut git = vec![];
        for &(a, b, kind) in &self.git_marks {
            for (i, &l) in nums.iter().enumerate() {
                if l >= a && l <= b {
                    git.push([first + i, kind as usize]);
                }
            }
        }
        let stripes = self
            .diags
            .iter()
            .filter(|d| d.sev <= 2 && d.sl < total)
            .take(400)
            .map(|d| [self.line_to_row(d.sl), d.sev])
            .collect();
        let exec = self.exec.and_then(|l| in_view(&nums, l));
        let all_bps = std::mem::take(&mut self.bps_changed).then(|| self.bps.clone());

        let key = (self.cur, self.anchor, self.rev);
        let clip = if self.sent_clip != Some(key) {
            self.sent_clip = Some(key);
            Some(if sa != sb {
                self.rope.slice(sa..sb).to_string()
            } else {
                format!("{}\n", self.line_text(cl))
            })
        } else {
            None
        };

        View {
            rev: self.rev,
            n,
            top,
            first,
            lines,
            nums,
            cur: [cr, self.vcol(cl, cc)],
            line: cl,
            col: cc + 1,
            total,
            crlf: self.crlf,
            use_tabs: self.use_tabs,
            tab: self.tab,
            readonly: self.readonly,
            folds,
            bps,
            git,
            exec,
            stripes,
            occ,
            pair,
            all_bps,
            dirty: self.is_dirty(),
            sel,
            marks,
            diags,
            msg,
            maxw: self.maxw,
            clip,
            errors,
            warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpp(text: &str) -> Doc {
        Doc::new("/x/a.cpp".into(), text, None)
    }
    fn txt(d: &Doc) -> String {
        d.rope.to_string()
    }
    fn typed(d: &mut Doc, s: &str) {
        for c in s.chars() {
            if c == '\n' {
                d.run(Cmd::Newline).unwrap();
            } else {
                d.run(Cmd::Type { s: c.to_string() }).unwrap();
            }
        }
    }
    fn mv(d: &mut Doc, to: Motion, ext: bool) {
        d.run(Cmd::Move {
            to,
            ext,
            rows: None,
        })
        .unwrap();
    }

    fn select(d: &mut Doc, from: usize, to: usize) {
        d.anchor = from;
        d.cur = to;
    }

    fn drop_at(d: &mut Doc, vcol: usize, copy: bool) {
        d.run(Cmd::MoveSelection {
            line: 0,
            vcol,
            copy,
        })
        .unwrap();
    }

    #[test]
    fn dragging_a_selection_moves_it_in_one_undo_step() {
        let mut d = cpp("one two three");
        select(&mut d, 4, 8);
        drop_at(&mut d, 13, false);
        assert_eq!(txt(&d), "one threetwo ");
        assert_eq!(d.sel_range(), (9, 13));
        d.run(Cmd::Undo).unwrap();
        assert_eq!(txt(&d), "one two three");

        select(&mut d, 4, 8);
        drop_at(&mut d, 0, false);
        assert_eq!(txt(&d), "two one three");
        assert_eq!(d.sel_range(), (0, 4));
    }

    #[test]
    fn dropping_inside_the_selection_changes_nothing() {
        let mut d = cpp("one two");
        select(&mut d, 0, 4);
        drop_at(&mut d, 2, false);
        drop_at(&mut d, 4, false);
        drop_at(&mut d, 2, true);
        assert_eq!(txt(&d), "one two");
    }

    #[test]
    fn dragging_with_copy_keeps_the_original() {
        let mut d = cpp("ab cd");
        select(&mut d, 0, 2);
        drop_at(&mut d, 5, true);
        assert_eq!(txt(&d), "ab cdab");
        assert_eq!(d.sel_range(), (5, 7));
    }

    #[test]
    fn new_lines_above_and_below_keep_the_indent() {
        let mut d = cpp("    int a;");
        d.run(Cmd::Goto { line: 0, col: 6 }).unwrap();
        d.run(Cmd::NewLineBelow).unwrap();
        assert_eq!(txt(&d), "    int a;\n    ");
        assert_eq!(d.cursor_pos(), (1, 4));
        d.run(Cmd::NewLineAbove).unwrap();
        assert_eq!(txt(&d), "    int a;\n    \n    ");
        assert_eq!(d.cursor_pos(), (1, 4));
    }

    #[test]
    fn complete_statement_adds_what_is_missing() {
        let mut d = cpp("    foo(1)");
        d.run(Cmd::Goto { line: 0, col: 5 }).unwrap();
        d.run(Cmd::CompleteStatement).unwrap();
        assert_eq!(txt(&d), "    foo(1);\n    ");

        let mut d = cpp("if (x)");
        d.run(Cmd::CompleteStatement).unwrap();
        assert_eq!(txt(&d), "if (x) {\n    \n}");
        assert_eq!(d.cursor_pos(), (1, 4));

        let mut d = cpp("int a;");
        d.run(Cmd::CompleteStatement).unwrap();
        assert_eq!(txt(&d), "int a;\n");
    }

    #[test]
    fn toggle_case_flips_the_word_or_selection() {
        let mut d = cpp("foo Bar");
        d.run(Cmd::ToggleCase).unwrap();
        assert_eq!(txt(&d), "FOO Bar");
        d.run(Cmd::ToggleCase).unwrap();
        assert_eq!(txt(&d), "foo Bar");
        select(&mut d, 4, 7);
        d.run(Cmd::ToggleCase).unwrap();
        assert_eq!(txt(&d), "foo BAR");
        assert_eq!(d.sel_range(), (4, 7));
    }

    #[test]
    fn goto_bracket_jumps_back_and_forth() {
        let mut d = cpp("f(a, b)");
        d.run(Cmd::Goto { line: 0, col: 1 }).unwrap();
        d.run(Cmd::GotoBracket).unwrap();
        assert_eq!(d.cursor_pos(), (0, 6));
        d.run(Cmd::GotoBracket).unwrap();
        assert_eq!(d.cursor_pos(), (0, 1));
    }

    #[test]
    fn comments_and_strings_are_detected_at_the_caret() {
        let mut d = cpp("int a; // note");
        d.run(Cmd::Goto { line: 0, col: 5 }).unwrap();
        assert!(!d.after_comment_or_string());
        d.run(Cmd::Goto { line: 0, col: 12 }).unwrap();
        assert!(d.after_comment_or_string());

        let mut d = cpp("f(\"abc\");");
        d.run(Cmd::Goto { line: 0, col: 5 }).unwrap();
        assert!(d.after_comment_or_string());
    }

    #[test]
    fn auto_pairs_and_type_over() {
        let mut d = cpp("");
        typed(&mut d, "f(");
        assert_eq!(txt(&d), "f()");
        typed(&mut d, ")");
        assert_eq!(txt(&d), "f()");
        assert_eq!(d.cur, 3);
    }

    #[test]
    fn enter_between_braces_opens_a_block() {
        let mut d = cpp("");
        typed(&mut d, "int f() {\n");
        assert_eq!(txt(&d), "int f() {\n    \n}");
        assert_eq!(d.cursor_pos(), (1, 4));
        typed(&mut d, "x;");
        assert_eq!(txt(&d), "int f() {\n    x;\n}");
    }

    #[test]
    fn typing_coalesces_into_one_undo_step_until_a_space() {
        let mut d = cpp("");
        typed(&mut d, "foo bar");
        assert_eq!(txt(&d), "foo bar");
        d.run(Cmd::Undo).unwrap();
        assert_eq!(txt(&d), "foo");
        d.run(Cmd::Undo).unwrap();
        assert_eq!(txt(&d), "");
        d.run(Cmd::Redo).unwrap();
        d.run(Cmd::Redo).unwrap();
        assert_eq!(txt(&d), "foo bar");
    }

    #[test]
    fn undo_restores_multi_op_edits_and_selection() {
        let mut d = cpp("a\nb\nc\n");
        d.run(Cmd::SelectAll).unwrap();
        d.run(Cmd::ToggleComment).unwrap();
        assert_eq!(txt(&d), "// a\n// b\n// c\n");
        d.run(Cmd::ToggleComment).unwrap();
        assert_eq!(txt(&d), "a\nb\nc\n");
        d.run(Cmd::Undo).unwrap();
        d.run(Cmd::Undo).unwrap();
        assert_eq!(txt(&d), "a\nb\nc\n");
        assert!(d.has_sel());
    }

    #[test]
    fn closing_brace_on_a_blank_line_dedents() {
        let mut d = cpp("{\n    ");
        d.run(Cmd::Move {
            to: Motion::DocEnd,
            ext: false,
            rows: None,
        })
        .unwrap();
        typed(&mut d, "}");
        assert_eq!(txt(&d), "{\n}");
    }

    #[test]
    fn dirty_tracks_save_point() {
        let mut d = cpp("x");
        assert!(!d.is_dirty());
        typed(&mut d, "y");
        assert!(d.is_dirty());
        d.saved_rev = d.rev;
        assert!(!d.is_dirty());
    }

    #[test]
    fn vertical_motion_keeps_the_column_through_short_lines() {
        let mut d = cpp("abcdef\nx\nabcdef");
        d.run(Cmd::Goto { line: 0, col: 5 }).unwrap();
        mv(&mut d, Motion::Down, false);
        assert_eq!(d.cursor_pos(), (1, 1));
        mv(&mut d, Motion::Down, false);
        assert_eq!(d.cursor_pos(), (2, 5));
    }

    #[test]
    fn tabs_map_to_visual_columns() {
        let mut d = cpp("\tx");
        d.run(Cmd::Click {
            line: 0,
            vcol: 4,
            ext: false,
        })
        .unwrap();
        assert_eq!(d.cursor_pos(), (0, 1));
        let v = d.view(0, 10, false);
        assert_eq!(v.cur, [0, 4]);
        assert_eq!(v.lines[0], "    <l>x</l>");
    }

    #[test]
    fn crlf_round_trips() {
        let d = Doc::new("/x/a.c".into(), "a\r\nb\r\n", None);
        assert_eq!(txt(&d), "a\nb\n");
        assert_eq!(d.contents_for_disk(), "a\r\nb\r\n");
    }

    #[test]
    fn html_is_escaped() {
        let mut d = cpp("a<b>&c");
        let v = d.view(0, 5, false);
        assert_eq!(v.lines[0], "<l>a</l>&lt;<l>b</l>&gt;&amp;<l>c</l>");
    }

    #[test]
    fn multiline_state_invalidates_after_an_edit() {
        let mut d = cpp("int a;\nint b;");
        d.view(0, 5, false);
        d.run(Cmd::Goto { line: 0, col: 0 }).unwrap();
        d.run(Cmd::Insert { s: "/*".into() }).unwrap();
        let v = d.view(0, 5, false);
        assert_eq!(v.lines[1], "<c>int b;</c>");
    }

    #[test]
    fn search_wraps_and_replaces() {
        let mut d = cpp("foo x\nFOO foo\n");
        assert_eq!(d.set_search("foo", false), 3);
        d.run(Cmd::Goto { line: 1, col: 4 }).unwrap();
        d.run(Cmd::FindNext { back: false }).unwrap();
        assert_eq!(d.cursor_pos(), (1, 7));
        d.run(Cmd::FindNext { back: false }).unwrap();
        assert_eq!(d.cursor_pos(), (0, 3));
        d.run(Cmd::FindNext { back: true }).unwrap();
        assert_eq!(d.cursor_pos(), (1, 7));
        d.run(Cmd::ReplaceAll { s: "z".into() }).unwrap();
        assert_eq!(txt(&d), "z x\nz z\n");
        d.run(Cmd::Undo).unwrap();
        assert_eq!(txt(&d), "foo x\nFOO foo\n");
    }

    #[test]
    fn utf16_columns_handle_astral_characters() {
        let d = cpp("😀x");
        assert_eq!(d.col_from_utf16(0, 2), 1);
        assert_eq!(d.utf16_col(0, 1), 2);
    }

    #[test]
    fn delete_line_at_end_of_file_takes_the_previous_newline() {
        let mut d = cpp("a\nb");
        d.run(Cmd::Goto { line: 1, col: 0 }).unwrap();
        d.run(Cmd::DeleteLine).unwrap();
        assert_eq!(txt(&d), "a");
    }

    #[test]
    fn complete_replaces_the_prefix() {
        let mut d = cpp("std::vec");
        d.run(Cmd::Move {
            to: Motion::DocEnd,
            ext: false,
            rows: None,
        })
        .unwrap();
        assert_eq!(d.prefix(), "vec");
        assert_eq!(d.before_prefix(), "::");
        d.run(Cmd::Complete { s: "vector".into() }).unwrap();
        assert_eq!(txt(&d), "std::vector");
    }

    #[test]
    fn lexing_is_lazy_on_huge_files() {
        let text = "int x;\n".repeat(200_000);
        let mut d = cpp(&text);
        let v = d.view(0, 40, false);
        assert_eq!(v.lines.len(), 46);
        assert!(d.states.len() < 100);
    }

    #[test]
    fn long_lines_are_capped() {
        let mut d = cpp(&"+".repeat(100_000));
        let v = d.view(0, 1, false);
        assert_eq!(v.lines[0].len(), MAX_RENDER);
    }

    #[test]
    fn folding_hides_rows_and_keeps_numbers() {
        let mut d = cpp("int f() {\n  a;\n  b;\n}\nint x;\n");
        assert_eq!(d.view(0, 10, false).folds, vec![[0, 1]]);
        d.run(Cmd::ToggleFold { row: 0 }).unwrap();
        let v = d.view(0, 10, false);
        assert_eq!(v.n, 3);
        assert_eq!(v.nums, vec![0, 4, 5]);
        assert!(v.lines[0].ends_with("<e> \u{2026} </e>"));
        d.run(Cmd::Goto { line: 4, col: 0 }).unwrap();
        d.run(Cmd::Move {
            to: Motion::Up,
            ext: false,
            rows: None,
        })
        .unwrap();
        assert_eq!(d.cursor_pos().0, 0);
    }

    #[test]
    fn caret_inside_a_fold_opens_it() {
        let mut d = cpp("f() {\n a;\n}\n");
        d.run(Cmd::ToggleFold { row: 0 }).unwrap();
        d.run(Cmd::Goto { line: 1, col: 0 }).unwrap();
        assert_eq!(d.view(0, 10, false).n, 4);
    }

    #[test]
    fn braces_in_strings_do_not_end_a_fold() {
        let mut d = cpp("f() {\n s = \"}\";\n // }\n}\nnext;\n");
        d.run(Cmd::ToggleFold { row: 0 }).unwrap();
        assert_eq!(d.view(0, 10, false).nums, vec![0, 4, 5]);
    }

    #[test]
    fn editing_above_shifts_folds_and_breakpoints() {
        let mut d = cpp("a\nf() {\n x;\n}\nend\n");
        d.run(Cmd::ToggleFold { row: 1 }).unwrap();
        d.run(Cmd::SetBreakpoints { lines: vec![4] }).unwrap();
        d.run(Cmd::Goto { line: 0, col: 0 }).unwrap();
        d.run(Cmd::Newline).unwrap();
        let v = d.view(0, 10, false);
        assert_eq!(v.nums, vec![0, 1, 2, 5, 6]);
        assert_eq!(v.bps, vec![3]);
        assert_eq!(v.all_bps, Some(vec![5]));
    }

    #[test]
    fn moving_lines_carries_the_caret() {
        let mut d = cpp("one\ntwo\nthree");
        d.run(Cmd::Goto { line: 1, col: 1 }).unwrap();
        d.run(Cmd::MoveLines { up: true }).unwrap();
        assert_eq!(txt(&d), "two\none\nthree");
        assert_eq!(d.cursor_pos(), (0, 1));
        d.run(Cmd::MoveLines { up: false }).unwrap();
        d.run(Cmd::MoveLines { up: false }).unwrap();
        assert_eq!(txt(&d), "one\nthree\ntwo");
        assert_eq!(d.cursor_pos(), (2, 1));
        d.run(Cmd::Undo).unwrap();
        assert_eq!(txt(&d), "one\ntwo\nthree");
    }

    #[test]
    fn expand_selection_walks_outwards_and_back() {
        let mut d = cpp("f(a, g(bcd))");
        d.run(Cmd::Goto { line: 0, col: 8 }).unwrap();
        let sel = |d: &Doc| d.rope.slice(d.sel_range().0..d.sel_range().1).to_string();
        d.run(Cmd::ExpandSelection).unwrap();
        assert_eq!(sel(&d), "bcd");
        d.run(Cmd::ExpandSelection).unwrap();
        assert_eq!(sel(&d), "(bcd)");
        d.run(Cmd::ExpandSelection).unwrap();
        assert_eq!(sel(&d), "a, g(bcd)");
        d.run(Cmd::ShrinkSelection).unwrap();
        assert_eq!(sel(&d), "(bcd)");
    }

    #[test]
    fn read_only_documents_ignore_edits_but_move() {
        let mut d = cpp("abc");
        d.readonly = true;
        d.run(Cmd::Type { s: "x".into() }).unwrap();
        assert_eq!(txt(&d), "abc");
        d.run(Cmd::Move {
            to: Motion::DocEnd,
            ext: false,
            rows: None,
        })
        .unwrap();
        assert_eq!(d.cursor_pos(), (0, 3));
    }

    #[test]
    fn matching_bracket_and_occurrences() {
        let mut d = cpp("foo(foo, xfoo)");
        d.run(Cmd::Goto { line: 0, col: 3 }).unwrap();
        let v = d.view(0, 5, false);
        assert_eq!(v.pair, vec![[0, 3], [0, 13]]);
        assert_eq!(v.occ, vec![[0, 0, 3], [0, 4, 7]]);
    }

    #[test]
    fn indent_guides_skip_the_first_column_and_keep_widths() {
        let mut d = cpp("        x");
        let v = d.view(0, 5, false);
        assert_eq!(v.lines[0], "    <g>    </g><l>x</l>");
        assert_eq!(v.cur, [0, 0]);
    }

    #[test]
    fn clangd_tokens_replace_guesses_but_not_keywords() {
        let mut d = cpp("Foo bar; return Baz;\n");
        d.set_semantic(vec![(0, 4, 3, lexer::MEMBER), (0, 9, 6, lexer::MEMBER)]);
        assert_eq!(
            d.view(0, 5, false).lines[0],
            "Foo <v>bar</v>; <k>return</k> Baz;"
        );
    }

    #[test]
    fn clangd_tokens_follow_edits_on_their_line() {
        let mut d = cpp("int value;\nint other;\n");
        d.set_semantic(vec![(0, 4, 5, lexer::MEMBER), (1, 4, 5, lexer::MEMBER)]);
        d.run(Cmd::Goto { line: 0, col: 0 }).unwrap();
        d.run(Cmd::Insert { s: "x ".into() }).unwrap();
        assert_eq!(
            d.view(0, 5, false).lines[0],
            "<l>x</l> <t>int</t> <v>value</v>;"
        );
        d.run(Cmd::Newline).unwrap();
        assert_eq!(d.view(0, 5, false).lines[2], "<t>int</t> <v>other</v>;");
    }

    #[test]
    fn semantic_columns_convert_from_utf16() {
        let mut d = cpp("/*😀*/ abc;\n");
        d.set_semantic(vec![(0, 7, 3, lexer::MEMBER)]);
        assert_eq!(d.view(0, 5, false).lines[0], "<c>/*😀*/</c> <v>abc</v>;");
    }

    #[test]
    fn byte_replacements_are_one_undo_and_keep_the_caret() {
        let mut d = cpp("int  a;\nint b;\n");
        d.run(Cmd::Goto { line: 1, col: 4 }).unwrap();
        d.run(Cmd::ToggleBreakpoint { row: Some(1) }).unwrap();
        assert_eq!(d.apply_byte_replacements(vec![(3, 2, " ".into())]), 1);
        assert_eq!(txt(&d), "int a;\nint b;\n");
        assert_eq!(d.cursor_pos(), (1, 4));
        assert_eq!(d.view(0, 5, false).bps, vec![1]);
        d.run(Cmd::Undo).unwrap();
        assert_eq!(txt(&d), "int  a;\nint b;\n");
    }

    #[test]
    fn clipboard_is_only_resent_when_it_changes() {
        let mut d = cpp("hello\n");
        assert_eq!(d.view(0, 5, false).clip.as_deref(), Some("hello\n"));
        assert!(d.view(0, 5, false).clip.is_none());
        d.run(Cmd::SelectAll).unwrap();
        assert_eq!(d.view(0, 5, false).clip.as_deref(), Some("hello\n"));
    }
}
