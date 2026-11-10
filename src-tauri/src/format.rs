use crate::runner::clean_env;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub const NOT_INSTALLED: &str = "clang-format is not installed";

fn style(name: &str, use_tabs: bool, indent: usize) -> String {
    let tabs = if use_tabs { "Always" } else { "Never" };
    match name {
        "google" => format!(
            "{{BasedOnStyle: Google, IndentWidth: {indent}, TabWidth: {indent}, UseTab: {tabs}}}"
        ),
        "llvm" => format!(
            "{{BasedOnStyle: LLVM, IndentWidth: {indent}, TabWidth: {indent}, UseTab: {tabs}}}"
        ),
        other => {
            let braces = if other == "allman" {
                "Allman"
            } else {
                "Attach"
            };
            format!(
                "{{BasedOnStyle: LLVM, IndentWidth: {indent}, TabWidth: {indent}, UseTab: {tabs}, ColumnLimit: 100, \
                 BreakBeforeBraces: {braces}, AllowShortFunctionsOnASingleLine: None, \
                 AllowShortIfStatementsOnASingleLine: Never, AllowShortLoopsOnASingleLine: false, \
                 AllowShortBlocksOnASingleLine: Never, AllowShortCaseLabelsOnASingleLine: false, \
                 IndentCaseLabels: true, MaxEmptyLinesToKeep: 1, KeepEmptyLinesAtTheStartOfBlocks: false, \
                 SortIncludes: Never, AlignTrailingComments: true}}"
            )
        }
    }
}

fn has_project_style(file: &Path) -> bool {
    file.ancestors()
        .skip(1)
        .any(|d| d.join(".clang-format").exists() || d.join("_clang-format").exists())
}

pub struct Request<'a> {
    pub path: &'a Path,
    pub text: String,
    pub style: &'a str,
    pub use_tabs: bool,
    pub indent: usize,
    pub lines: Option<(usize, usize)>,
}

pub fn replacements(req: Request) -> Result<Vec<(usize, usize, String)>, String> {
    let style_arg = if has_project_style(req.path) {
        "--style=file".to_string()
    } else {
        format!("--style={}", style(req.style, req.use_tabs, req.indent))
    };
    let mut cmd = Command::new("clang-format");
    clean_env(&mut cmd);
    cmd.arg(style_arg)
        .arg("--output-replacements-xml")
        .arg(format!("--assume-filename={}", req.path.display()));
    if let Some((first, last)) = req.lines {
        cmd.arg(format!("--lines={}:{}", first + 1, last + 1));
    }
    if let Some(dir) = req.path.parent() {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                NOT_INSTALLED.to_string()
            } else {
                format!("clang-format: {e}")
            }
        })?;
    let mut stdin = child.stdin.take().ok_or("clang-format: no stdin")?;
    let text = req.text;
    let writer = std::thread::spawn(move || stdin.write_all(text.as_bytes()));
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = writer.join();
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(parse_xml(&String::from_utf8_lossy(&out.stdout)))
}

fn attr(tag: &str, name: &str) -> Option<usize> {
    let start = tag.find(&format!("{name}='"))? + name.len() + 2;
    tag[start..].split('\'').next()?.parse().ok()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        match entity {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "apos" => out.push('\''),
            "quot" => out.push('"'),
            _ => {
                if let Some(c) = entity
                    .strip_prefix('#')
                    .and_then(|n| n.parse().ok())
                    .and_then(char::from_u32)
                {
                    out.push(c);
                }
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn parse_xml(xml: &str) -> Vec<(usize, usize, String)> {
    let mut out = vec![];
    let mut rest = xml;
    while let Some(i) = rest.find("<replacement ") {
        rest = &rest[i..];
        let Some(open_end) = rest.find('>') else {
            break;
        };
        let tag = &rest[..open_end];
        let (Some(offset), Some(length)) = (attr(tag, "offset"), attr(tag, "length")) else {
            break;
        };
        if tag.ends_with('/') {
            out.push((offset, length, String::new()));
            rest = &rest[open_end + 1..];
            continue;
        }
        let body = &rest[open_end + 1..];
        let Some(close) = body.find("</replacement>") else {
            break;
        };
        out.push((offset, length, unescape(&body[..close])));
        rest = &body[close..];
        rest = &rest["</replacement>".len()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacements_decode_entities() {
        let xml = "<?xml version='1.0'?>\n<replacements xml:space='preserve' incomplete_format='false'>\n\
                   <replacement offset='3' length='1'>&#10;  &lt;x&gt;</replacement>\n\
                   <replacement offset='9' length='2'></replacement>\n\
                   <replacement offset='12' length='1'/>\n</replacements>\n";
        assert_eq!(
            parse_xml(xml),
            vec![
                (3, 1, "\n  <x>".to_string()),
                (9, 2, String::new()),
                (12, 1, String::new())
            ]
        );
    }

    #[test]
    fn clean_style_uses_the_file_indent() {
        let s = style("clean", false, 4);
        assert!(
            s.contains("IndentWidth: 4") && s.contains("Attach") && s.contains("UseTab: Never")
        );
        assert!(style("allman", true, 2).contains("Allman"));
    }
}
