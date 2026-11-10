use crate::runner::clean_env;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

fn git(cwd: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    clean_env(&mut cmd);
    let out = cmd
        .args(args)
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.trim().to_string())
    }
}

fn git_any(cwd: &str, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    clean_env(&mut cmd);
    cmd.args(args)
        .current_dir(cwd)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[derive(Serialize, Default)]
pub struct Status {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<Change>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    pub x: String,
    pub y: String,
}

fn parse_status(raw: &str) -> Status {
    let mut st = Status::default();
    let mut it = raw.split('\0');
    while let Some(e) = it.next() {
        if let Some(h) = e.strip_prefix("## ") {
            let (name, rest) = h.split_once(' ').unwrap_or((h, ""));
            st.branch = name.split("...").next().unwrap_or(name).to_string();
            let num = |k: &str| {
                rest.split(k)
                    .nth(1)
                    .and_then(|s| s.trim_start().split([',', ']']).next())
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0)
            };
            st.ahead = num("ahead");
            st.behind = num("behind");
        } else if e.len() > 3 {
            let (x, y) = (e[..1].to_string(), e[1..2].to_string());
            if x == "R" || x == "C" {
                it.next();
            }
            st.files.push(Change {
                path: e[3..].to_string(),
                x,
                y,
            });
        }
    }
    st
}

pub fn status(root: &str) -> Result<Status, String> {
    git(root, &["status", "--porcelain=v1", "-b", "-z", "-unormal"]).map(|s| parse_status(&s))
}

pub fn stage(root: &str, paths: &[String]) -> Result<(), String> {
    let mut a = vec!["add", "-A", "--"];
    a.extend(paths.iter().map(String::as_str));
    git(root, &a).map(|_| ())
}

pub fn unstage(root: &str, paths: &[String]) -> Result<(), String> {
    let mut a = vec!["reset", "-q", "HEAD", "--"];
    a.extend(paths.iter().map(String::as_str));
    git(root, &a).map(|_| ())
}

pub fn discard(root: &str, path: &str, untracked: bool) -> Result<(), String> {
    if untracked {
        git(root, &["clean", "-fdq", "--", path]).map(|_| ())
    } else {
        git(root, &["checkout", "HEAD", "--", path]).map(|_| ())
    }
}

pub fn commit(root: &str, message: &str, amend: bool) -> Result<String, String> {
    let mut a = vec!["commit", "-m", message];
    if amend {
        a.push("--amend");
    }
    git(root, &a)
}

pub fn diff(root: &str, path: &str, untracked: bool) -> String {
    if untracked {
        git_any(
            root,
            &["diff", "--no-color", "--no-index", "--", "/dev/null", path],
        )
    } else {
        git_any(root, &["diff", "--no-color", "HEAD", "--", path])
    }
}

#[derive(Serialize)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub when: String,
}

pub fn log(root: &str) -> Result<Vec<Commit>, String> {
    let out = git(
        root,
        &["log", "-n", "100", "--format=%h%x00%s%x00%an%x00%cr"],
    )?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut p = l.split('\0');
            Some(Commit {
                hash: p.next()?.into(),
                subject: p.next()?.into(),
                author: p.next()?.into(),
                when: p.next()?.into(),
            })
        })
        .collect())
}

pub fn show(root: &str, hash: &str) -> Result<String, String> {
    git(root, &["show", "--no-color", "--stat", "-p", hash])
}

pub fn branches(root: &str) -> Result<Vec<String>, String> {
    Ok(git(root, &["branch", "--format=%(refname:short)"])?
        .lines()
        .map(String::from)
        .collect())
}

pub fn checkout(root: &str, name: &str, create: bool) -> Result<(), String> {
    if create {
        git(root, &["switch", "-c", name]).map(|_| ())
    } else {
        git(root, &["switch", name]).map(|_| ())
    }
}

#[cfg(unix)]
pub fn branch(cwd: &str) -> Option<String> {
    git(cwd, &["branch", "--show-current"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn clone(url: &str, parent: &str) -> Result<String, String> {
    let name = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".git")
        .to_string();
    if name.is_empty() || name.starts_with('.') {
        return Err("Couldn't work out a folder name from that URL".into());
    }
    let dest = Path::new(parent).join(&name);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    let mut cmd = Command::new("git");
    clean_env(&mut cmd);
    let out = cmd
        .args(["clone", "--", url, &name])
        .current_dir(parent)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(dest.to_string_lossy().into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn line_marks(file: &Path) -> Vec<(usize, usize, u8)> {
    let (Some(dir), Some(name)) = (
        file.parent().and_then(|d| d.to_str()),
        file.file_name().and_then(|n| n.to_str()),
    ) else {
        return vec![];
    };
    parse_hunks(&git_any(
        dir,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "-U0",
            "HEAD",
            "--",
            name,
        ],
    ))
}

fn parse_hunks(diff: &str) -> Vec<(usize, usize, u8)> {
    let span = |s: &str| -> (usize, usize) {
        let mut p = s[1..].split(',');
        let a = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        (a, p.next().and_then(|v| v.parse().ok()).unwrap_or(1))
    };
    diff.lines()
        .filter(|l| l.starts_with("@@ "))
        .filter_map(|l| {
            let mut w = l.split_whitespace().skip(1);
            let (_, old) = span(w.next()?);
            let (start, new) = span(w.next()?);
            Some(match (old, new) {
                (_, 0) => (start, start, 3),
                (0, n) => (start - 1, start + n - 2, 1),
                (_, n) => (start - 1, start + n - 2, 2),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reads_branch_and_renames() {
        let st = parse_status(
            "## main...origin/main [ahead 2, behind 1]\0 M a.c\0R  new.c\0old.c\0?? x\0",
        );
        assert_eq!(st.branch, "main");
        assert_eq!((st.ahead, st.behind), (2, 1));
        assert_eq!(st.files.len(), 3);
        assert_eq!(st.files[1].path, "new.c");
        assert_eq!(st.files[2].x, "?");
    }

    #[test]
    fn branch_without_upstream() {
        assert_eq!(parse_status("## feature\0").branch, "feature");
    }

    #[test]
    fn hunks_classify_changes() {
        let d = "@@ -3 +3 @@ x\n-a\n+b\n@@ -0,0 +5,2 @@\n+c\n+d\n@@ -9,2 +11,0 @@\n-e\n-f\n";
        assert_eq!(parse_hunks(d), vec![(2, 2, 2), (4, 5, 1), (11, 11, 3)]);
    }
}
