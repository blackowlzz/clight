use crate::doc::{Diag, Doc};
use crate::runner::{clean_env, lower_priority};
use serde_json::Value;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

#[derive(Debug, PartialEq)]
pub struct Issue {
    line: usize,
    col: usize,
    sev: usize,
    msg: String,
}

fn spawn(program: &str, path: &Path, cpp: bool) -> std::io::Result<Child> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut cmd = Command::new(program);
    clean_env(&mut cmd);
    cmd.args([
        "-fsyntax-only",
        "-fdiagnostics-color=never",
        "-fno-diagnostics-show-caret",
        "-ftabstop=1",
        "-Wall",
        "-Wextra",
        "-iquote",
    ])
    .arg(dir)
    .args(["-x", if cpp { "c++" } else { "c" }, "-"])
    .env("LC_ALL", "C")
    .env("LANGUAGE", "en")
    .current_dir(dir)
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    lower_priority(&mut cmd);
    cmd.spawn()
}

/// Asks gcc or clang about an unsaved buffer, for when clangd isn't available.
pub fn check(path: &Path, text: String, cpp: bool) -> Result<Vec<Issue>, String> {
    let names = if cpp {
        ["g++", "clang++"]
    } else {
        ["gcc", "clang"]
    };
    let mut child = None;
    for name in names {
        match spawn(name, path, cpp) {
            Ok(c) => {
                child = Some(c);
                break;
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{name}: {e}")),
        }
    }
    let mut child = child.ok_or("No C or C++ compiler found")?;
    let mut stdin = child.stdin.take().ok_or("compiler: no stdin")?;
    std::thread::spawn(move || {
        let _ = stdin.write_all(text.as_bytes());
    });
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok(parse(&String::from_utf8_lossy(&out.stderr)))
}

fn parse(stderr: &str) -> Vec<Issue> {
    stderr.lines().filter_map(parse_line).collect()
}

fn parse_line(l: &str) -> Option<Issue> {
    let mut it = l.strip_prefix("<stdin>:")?.splitn(3, ':');
    let line = it.next()?.parse::<usize>().ok()?.checked_sub(1)?;
    let col = it.next()?.parse::<usize>().ok()?.saturating_sub(1);
    let tail = it.next()?.trim_start();
    let (sev, msg) = if let Some(m) = tail.strip_prefix("fatal error: ") {
        (1, m)
    } else if let Some(m) = tail.strip_prefix("error: ") {
        (1, m)
    } else {
        (2, tail.strip_prefix("warning: ")?)
    };
    // Without the project's include paths a missing header proves nothing.
    if msg.contains("No such file or directory") || msg.contains("file not found") {
        return None;
    }
    Some(Issue {
        line,
        col,
        sev,
        msg: msg.to_string(),
    })
}

pub fn to_diags(doc: &Doc, issues: &[Issue]) -> Vec<Diag> {
    issues
        .iter()
        .filter(|i| i.line < doc.rope.len_lines())
        .map(|i| {
            let word = doc
                .line_text(i.line)
                .chars()
                .skip(i.col)
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .count();
            Diag {
                sl: i.line,
                sc: i.col,
                el: i.line,
                ec: i.col + word.max(1),
                sev: i.sev,
                msg: i.msg.clone(),
                raw: Value::Null,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_output_becomes_issues() {
        let out = "<stdin>: In function 'main':\n\
                   <stdin>:3:5: error: 'x' undeclared (first use in this function)\n\
                   <stdin>:2:9: warning: unused variable 'y' [-Wunused-variable]\n\
                   <stdin>:3:5: note: each undeclared identifier is reported only once\n\
                   /usr/include/a.h:1:1: error: not in this buffer\n";
        assert_eq!(
            parse(out),
            vec![
                Issue {
                    line: 2,
                    col: 4,
                    sev: 1,
                    msg: "'x' undeclared (first use in this function)".into()
                },
                Issue {
                    line: 1,
                    col: 8,
                    sev: 2,
                    msg: "unused variable 'y' [-Wunused-variable]".into()
                },
            ]
        );
    }

    #[test]
    fn missing_headers_are_not_reported() {
        assert!(parse("<stdin>:1:10: fatal error: a.h: No such file or directory\n").is_empty());
        assert!(parse("<stdin>:1:10: fatal error: 'a.h' file not found\n").is_empty());
    }

    #[test]
    fn a_real_compiler_flags_a_broken_buffer() {
        let path = std::env::temp_dir().join("clight-syntax-test.c");
        let Ok(issues) = check(&path, "int main(void) { return x; }\n".into(), false) else {
            return;
        };
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!((issues[0].line, issues[0].col, issues[0].sev), (0, 24, 1));
    }
}
