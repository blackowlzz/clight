//! Throws random commands at documents, with positions that are often out of range, the way a
//! stale reply from clangd or a remembered caret in a file that changed on disk would. Any panic
//! here would close the whole app in a release build, where panics abort.

use crate::doc::{Cmd, Doc};
use serde_json::{json, Value};

thread_local! {
    static WHERE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Panics are caught and reported with the place in our code where they happened, past the
/// frames of ropey and the std, instead of being printed as they happen.
fn quiet_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|_| {
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            let ours: Vec<String> = bt.lines().filter(|l| l.contains("./src/") && !l.contains("fuzz.rs")).take(3).map(|l| l.trim().to_string()).collect();
            WHERE.with(|w| *w.borrow_mut() = ours.join(" <- "));
        }));
    });
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
    /// Mostly sensible numbers, sometimes far too large.
    fn num(&mut self, around: usize) -> usize {
        match self.below(10) {
            0 => 1 << 40,
            1 => around + self.below(50),
            _ => self.below(around + 3),
        }
    }
}

const TEXTS: &[&str] = &[
    "",
    "/* a\nb */ c\n\"s\" /* d\n*/ e\nR\"x(raw\n)x\" f\n#if 0\ng\n#endif\n",
    "a /* b\nc\nd */ e\nf\ng /*\nh\n",
    "\n",
    "a",
    "int main() {\n\treturn 0;\n}\n",
    "😀x\r\nyé\u{301}z\r\n",
    "/* open comment\nstill\n",
    "\"open string\n'c'\n",
    "{{{\n((\n[[",
    "}}}\n))\n]]",
    "    if (x) {\n        y();\n    }\n\n\n",
    "#include <a.h>\n#define X(a) a\n// x\nstruct S { int a; };\n",
    "\t\t\t\n \t \n",
    "日本語のテキスト\n한국어\n",
];

const STRINGS: &[&str] = &["", "a", "/*", "*/", "\"", "R\"x(", ")x\"", "#if 0\n", "#endif\n", "\n", "\t", "}", "{", "(", "\"", "😀", "\r\n", "abc\ndef", "é\u{301}", "  ", "//", "/*"];

const MOTIONS: &[&str] = &["left", "right", "up", "down", "home", "end", "word_left", "word_right", "page_up", "page_down", "doc_start", "doc_end"];

fn random_cmd(r: &mut Rng, lines: usize) -> Value {
    let s = |r: &mut Rng| {
        let mut out = String::new();
        for _ in 0..r.below(3) + 1 {
            out.push_str(r.pick(STRINGS));
        }
        out
    };
    match r.below(40) {
        0 => json!({ "t": "type", "s": s(r) }),
        1 => json!({ "t": "insert", "s": s(r) }),
        2 => json!({ "t": "newline" }),
        3 => json!({ "t": "backspace" }),
        4 => json!({ "t": "delete" }),
        5 => json!({ "t": "delete_word_left" }),
        6 => json!({ "t": "delete_word_right" }),
        7 => json!({ "t": "tab" }),
        8 => json!({ "t": "outdent" }),
        9 => json!({ "t": "move", "to": r.pick(MOTIONS), "ext": r.below(2) == 0, "rows": if r.below(2) == 0 { Value::Null } else { json!(r.num(10)) } }),
        10 => json!({ "t": "click", "line": r.num(lines), "vcol": r.num(20), "ext": r.below(2) == 0 }),
        11 => json!({ "t": "word", "line": r.num(lines), "vcol": r.num(20) }),
        12 => json!({ "t": "select_line", "line": r.num(lines) }),
        13 => json!({ "t": "select_all" }),
        14 => json!({ "t": "move_selection", "line": r.num(lines), "vcol": r.num(20), "copy": r.below(2) == 0 }),
        15 => json!({ "t": "new_line_below" }),
        16 => json!({ "t": "new_line_above" }),
        17 => json!({ "t": "complete_statement" }),
        18 => json!({ "t": "toggle_case" }),
        19 => json!({ "t": "goto_bracket" }),
        20 => json!({ "t": "undo" }),
        21 => json!({ "t": "redo" }),
        22 => json!({ "t": "toggle_comment" }),
        23 => json!({ "t": "tidy", "trim": r.below(2) == 0, "newline": r.below(2) == 0 }),
        24 => json!({ "t": "duplicate_line" }),
        25 => json!({ "t": "delete_line" }),
        26 => json!({ "t": "cut_selection" }),
        27 => json!({ "t": "complete", "s": s(r) }),
        28 => json!({ "t": "goto", "line": r.num(lines), "col": r.num(20) }),
        29 => json!({ "t": "find_next", "back": r.below(2) == 0 }),
        30 => json!({ "t": "replace_one", "s": s(r) }),
        31 => json!({ "t": "replace_all", "s": s(r) }),
        32 => json!({ "t": "move_lines", "up": r.below(2) == 0 }),
        33 => json!({ "t": "join_lines" }),
        34 => json!({ "t": if r.below(2) == 0 { "expand_selection" } else { "shrink_selection" } }),
        35 => json!({ "t": "toggle_fold", "row": r.num(lines) }),
        36 => json!({ "t": "fold_caret", "fold": r.below(2) == 0 }),
        37 => json!({ "t": "fold_all", "fold": r.below(2) == 0 }),
        38 => json!({ "t": "toggle_breakpoint", "row": if r.below(2) == 0 { Value::Null } else { json!(r.num(lines)) } }),
        _ => match r.below(3) {
            0 => json!({ "t": "set_breakpoints", "lines": [r.num(lines), r.num(lines)] }),
            1 => json!({ "t": "set_exec", "line": if r.below(2) == 0 { Value::Null } else { json!(r.num(lines)) } }),
            _ => json!({ "t": "fresh" }),
        },
    }
}

/// The calls made from outside `run`: replies from clangd and clang-format, searches, the view.
fn random_call(r: &mut Rng, d: &mut Doc, log: &mut Vec<String>) {
    let lines = d.rope.len_lines();
    match r.below(13) {
        0 => {
            let (top, rows, reveal) = (r.num(lines), r.num(40), r.below(2) == 0);
            log.push(format!("view({top}, {rows}, {reveal})"));
            d.view(top, rows, reveal);
        }
        1 => {
            let q = r.pick(STRINGS).to_string();
            log.push(format!("set_search({q:?})"));
            d.set_search(&q, r.below(2) == 0);
        }
        2 => {
            let e: Vec<_> = (0..r.below(3) + 1).map(|_| (r.num(lines), r.num(10), r.num(lines), r.num(10), r.pick(STRINGS).to_string())).collect();
            log.push(format!("apply_utf16_edits({e:?})"));
            d.apply_utf16_edits(e);
        }
        3 => {
            let n = d.rope.len_bytes();
            let e: Vec<_> = (0..r.below(3) + 1).map(|_| (r.num(n), r.num(4), r.pick(STRINGS).to_string())).collect();
            log.push(format!("apply_byte_replacements({e:?})"));
            d.apply_byte_replacements(e);
        }
        4 => {
            let t: Vec<_> = (0..r.below(5)).map(|_| (r.num(lines), r.num(10), r.num(10), r.below(12) as u8)).collect();
            log.push(format!("set_semantic({t:?})"));
            d.set_semantic(t);
        }
        5 => {
            let (row, vcol) = (r.num(lines), r.num(20));
            log.push(format!("word_at/diag_at({row}, {vcol})"));
            d.word_at(row, vcol);
            d.diag_at(row, vcol);
        }
        6 => {
            let (l, c) = (r.num(lines), r.num(20));
            log.push(format!("goto_chars({l}, {c})"));
            d.goto_chars(l, c);
        }
        7 => {
            log.push("context".into());
            d.line_context();
            d.windows(r.num(100), r.num(100));
            d.prefix();
            d.before_prefix();
            d.after_comment_or_string();
            d.next_problem();
            d.selection_lines();
            let _ = d.lines_for_suggest().count();
        }
        8 => {
            let (l, u) = (r.num(lines), r.num(10));
            log.push(format!("col_from_utf16({l}, {u})"));
            d.col_from_utf16(l, u);
        }
        9 => {
            let (l, c) = d.cursor_pos();
            log.push(format!("utf16_col({l}, {c})"));
            d.utf16_col(l, c);
        }
        11 => {
            // Diagnostics and git marks are computed for an older text and arrive late.
            let diags = (0..r.below(4))
                .map(|_| {
                    let (sl, el) = (r.num(lines), r.num(lines));
                    crate::doc::Diag { sl, sc: r.num(20), el, ec: r.num(20), sev: r.below(4) + 1, msg: "m".into(), raw: Value::Null }
                })
                .collect();
            let marks = (0..r.below(4)).map(|_| (r.num(lines), r.num(lines), r.below(4) as u8)).collect();
            log.push(format!("stale diags/marks {:?}", (lines,)));
            d.diags = diags;
            d.git_marks = marks;
        }
        10 => {
            let text = r.pick(TEXTS).to_string();
            log.push(format!("reload({text:?})"));
            d.reload(&text, None);
        }
        _ => {
            log.push("contents".into());
            d.contents_for_disk();
        }
    }
}

fn session(seed: u64, steps: usize) -> Result<(), String> {
    let mut log = vec![];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut r = Rng(seed * 2654435761 + 1);
        let text = *r.pick(TEXTS);
        let name = r.pick(&["a.cpp", "a.c", "a.h", "a.txt", "CMakeLists.txt", "a.py"]).to_string();
        log.push(format!("new {name} {text:?}"));
        let mut d = Doc::new(format!("/x/{name}").into(), text, None);
        for _ in 0..steps {
            if let Err(e) = d.check() {
                panic!("broken after the last step: {e}");
            }
            // Now and then, so that several changes can pile up before the colors are worked out.
            if r.below(5) == 0 {
                if let Err(e) = d.states_from_scratch() {
                    panic!("line states differ from a fresh start: {e}");
                }
            }
            if r.below(4) == 0 {
                random_call(&mut r, &mut d, &mut log);
            } else {
                let c = random_cmd(&mut r, d.rope.len_lines());
                log.push(c.to_string());
                let cmd: Cmd = serde_json::from_value(c).unwrap();
                let _ = d.run(cmd);
            }
        }
    }));
    result.map_err(|e| {
        let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
        let tail = log.iter().rev().take(8).rev().cloned().collect::<Vec<_>>().join("\n  ");
        let at = WHERE.with(|w| w.borrow().clone());
        format!("seed {seed}: {msg}\n  at {at}\n  {tail}")
    })
}

#[test]
fn random_commands_never_panic() {
    let seeds: u64 = std::env::var("FUZZ_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    quiet_panics();
    let mut failures: Vec<String> = (0..seeds).filter_map(|s| session(s, 200).err()).collect();
    failures.truncate(5);
    eprintln!("{}", failures.join("\n\n"));
    assert!(failures.is_empty(), "{} failures, printed above", failures.len());
}


/// Random output from a program: escape sequences with odd or huge parameters, wide and
/// combining characters, invalid UTF-8, all mixed with resizes and scrolling the view.
#[test]
fn random_terminal_output_never_panics() {
    use crate::term::Grid;
    use vte::Parser;
    const PIECES: &[&str] = &[
        "a", "😀", "日", "e\u{301}", "\r", "\n", "\t", "\x08", "\x07", "\x1b[", "\x1b]", "\x1b(0", "\x1b(B", "\x1bM", "\x1b7", "\x1b8", "\x1bc",
        "\x1b[H", "\x1b[2J", "\x1b[K", "\x1b[1K", "\x1b[2K", "\x1b[J", "\x1b[1J", "\x1b[3J", "\x1b[?1049h", "\x1b[?1049l", "\x1b[?25l", "\x1b[?7l", "\x1b[?7h",
        "\x1b[?2004h", "\x1b[6n", "\x1b[c", "\x1b[s", "\x1b[u", "\x1b]0;title\x07", "\x1b]8;;http://x\x1b\\", "\u{fffd}", "\x00", "\x7f",
    ];
    const OPS: &[char] = &['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'f', 'd', 'J', 'K', 'L', 'M', 'P', '@', 'X', 'S', 'T', 'r', 'm', 'h', 'l', 'b', 'Z', 'I', 'n', 'c', 'q', 't'];
    quiet_panics();
    let seeds: u64 = std::env::var("FUZZ_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let mut failures = vec![];
    for seed in 0..seeds {
        let mut log: Vec<String> = vec![];
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut r = Rng(seed * 2654435761 + 7);
            let (c, h) = (r.below(100) + 1, r.below(40) + 1);
            log.push(format!("new {c}x{h}"));
            let mut g = Grid::new(c, h);
            let mut p = Parser::new();
            for _ in 0..300 {
                match r.below(20) {
                    0 => {
                        let (c, h) = (r.below(120) + 1, r.below(50) + 1);
                        log.push(format!("resize {c}x{h}"));
                        g.resize(c, h);
                    }
                    1 => {
                        let (off, full) = (r.num(100), r.below(2) == 0);
                        log.push(format!("view {off} {full}"));
                        g.view(off, full);
                        g.take_reply();
                    }
                    2 => {
                        let bytes: Vec<u8> = (0..r.below(8)).map(|_| r.next() as u8).collect();
                        log.push(format!("bytes {bytes:?}"));
                        p.advance(&mut g, &bytes);
                    }
                    3..=9 => {
                        let mut s = String::from("\x1b[");
                        if r.below(4) == 0 {
                            s.push('?');
                        }
                        for i in 0..r.below(4) {
                            if i > 0 {
                                s.push(';');
                            }
                            s.push_str(&match r.below(6) {
                                0 => "0".to_string(),
                                1 => "65535".to_string(),
                                2 => "4294967295".to_string(),
                                _ => r.below(200).to_string(),
                            });
                        }
                        s.push(*r.pick(OPS));
                        log.push(format!("{s:?}"));
                        p.advance(&mut g, s.as_bytes());
                    }
                    _ => {
                        let s = *r.pick(PIECES);
                        log.push(format!("{s:?}"));
                        p.advance(&mut g, s.as_bytes());
                    }
                }
            }
        }));
        if let Err(e) = res {
            let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            let at = WHERE.with(|w| w.borrow().clone());
            let tail = log.iter().rev().take(8).rev().cloned().collect::<Vec<_>>().join("\n  ");
            failures.push(format!("seed {seed}: {msg}\n  at {at}\n  {tail}"));
        }
        if failures.len() >= 5 {
            break;
        }
    }
    eprintln!("{}", failures.join("\n\n"));
    assert!(failures.is_empty(), "{} failures, printed above", failures.len());
}

/// How long the usual commands take on files that are hard for an editor. Run it with
/// `cargo test --profile local bench_big_files -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_big_files() {
    use std::time::Instant;
    let time = |label: &str, d: &mut Doc, n: usize, f: &mut dyn FnMut(&mut Doc, usize)| {
        let t = Instant::now();
        for i in 0..n {
            f(d, i);
        }
        eprintln!("{label:<44} {:>9.3} ms each", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
    };
    let run = |d: &mut Doc, c: Value| {
        let _ = d.run(serde_json::from_value(c).unwrap());
    };

    let long = format!("{{\"k\":[{}]}}\n", (0..400_000).map(|i| format!("\"v{i}\"")).collect::<Vec<_>>().join(","));
    eprintln!("one line of {} MB", long.len() >> 20);
    let mut d = Doc::new("/x/a.json".into(), &long, None);
    time("view at the top", &mut d, 20, &mut |d, _| {
        d.view(0, 50, false);
    });
    run(&mut d, json!({ "t": "move", "to": "end", "ext": false, "rows": null }));
    time("type at the end of the line + view", &mut d, 50, &mut |d, _| {
        run(d, json!({ "t": "type", "s": "x" }));
        d.view(0, 50, true);
    });
    time("arrow left + view", &mut d, 50, &mut |d, _| {
        run(d, json!({ "t": "move", "to": "left", "ext": false, "rows": null }));
        d.view(0, 50, true);
    });

    let many: String = (0..300_000).map(|i| format!("int f{i}(int a) {{ return a * {i}; }} // line {i}\n")).collect();
    eprintln!("{} lines, {} MB", 300_000, many.len() >> 20);
    let mut d = Doc::new("/x/a.cpp".into(), &many, None);
    time("view at the top", &mut d, 20, &mut |d, _| {
        d.view(0, 50, false);
    });
    time("view at the end", &mut d, 5, &mut |d, _| {
        d.view(299_950, 50, false);
    });
    time("view at the end again", &mut d, 20, &mut |d, _| {
        d.view(299_950, 50, false);
    });
    run(&mut d, json!({ "t": "goto", "line": 299_960, "col": 0 }));
    time("type near the end + view", &mut d, 50, &mut |d, _| {
        run(d, json!({ "t": "type", "s": "x" }));
        d.view(299_940, 50, true);
    });
    run(&mut d, json!({ "t": "goto", "line": 0, "col": 0 }));
    time("type at the top + view", &mut d, 50, &mut |d, _| {
        run(d, json!({ "t": "type", "s": "/" }));
        d.view(0, 50, true);
    });
    time("type at the top + view at the end", &mut d, 10, &mut |d, _| {
        run(d, json!({ "t": "type", "s": "x" }));
        d.view(299_940, 50, false);
    });
    time("set_search(\"return\")", &mut d, 5, &mut |d, _| {
        d.set_search("return", false);
    });
    time("find_next", &mut d, 50, &mut |d, _| {
        run(d, json!({ "t": "find_next", "back": false }));
    });
    time("select_all + toggle_comment", &mut d, 2, &mut |d, _| {
        run(d, json!({ "t": "select_all" }));
        run(d, json!({ "t": "toggle_comment" }));
    });
    time("fold_all", &mut d, 3, &mut |d, i| {
        run(d, json!({ "t": "fold_all", "fold": i % 2 == 0 }));
    });
    time("undo", &mut d, 20, &mut |d, _| {
        run(d, json!({ "t": "undo" }));
    });
    time("contents_for_disk", &mut d, 5, &mut |d, _| {
        d.contents_for_disk();
    });
}

/// How fast the terminal takes a flood of output and how long the project search takes.
/// `cargo test --profile local bench_terminal_and_search -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_terminal_and_search() {
    use crate::term::Grid;
    use std::time::Instant;
    use vte::Parser;
    let log: String = (0..200_000)
        .map(|i| format!("\x1b[32m[{:>3}%]\x1b[0m Building CXX object src/CMakeFiles/app.dir/file_{i}.cpp.o \x1b[1;33mwarning:\x1b[0m unused 'x' é😀\r\n", i % 100))
        .collect();
    eprintln!("terminal: {} MB of colored output", log.len() >> 20);
    let mut g = Grid::new(160, 40);
    let mut p = Parser::new();
    let t = Instant::now();
    let mut views = 0;
    for chunk in log.as_bytes().chunks(16 << 10) {
        p.advance(&mut g, chunk);
        // The UI asks for a view about every frame while output streams in.
        if views < 2000 {
            g.view(0, false);
            views += 1;
        }
    }
    let secs = t.elapsed().as_secs_f64();
    eprintln!("  parse + {views} views: {:.0} ms ({:.0} MB/s)", secs * 1000.0, (log.len() >> 20) as f64 / secs);
    let t = Instant::now();
    for i in 0..200 {
        g.view(i * 10, true);
    }
    eprintln!("  scrolled full view: {:.3} ms each", t.elapsed().as_secs_f64() * 1000.0 / 200.0);

    if let Ok(root) = std::env::var("GREP_ROOT") {
        let t = Instant::now();
        let n = crate::project::list_files(&root).len();
        eprintln!("list_files {root}: {n} files, {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        for q in ["fn_nothing_here", "impl"] {
            let t = Instant::now();
            let n = crate::project::grep(&root, q).len();
            eprintln!("grep {q:?}: {n} hits, {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
