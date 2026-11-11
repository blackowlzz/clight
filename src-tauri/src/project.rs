use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".cache",
    ".idea",
    ".vscode",
    ".clight",
    "node_modules",
    "target",
];
const MAX_WALK: usize = 30_000;
const MAX_GREP_FILE: u64 = 2 << 20;
const MAX_GREP_HITS: usize = 400;

fn skip(name: &str) -> bool {
    SKIP_DIRS.contains(&name) || name.starts_with("cmake-build-")
}

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
}

pub fn list_dir(path: &str, hidden: bool) -> Result<Vec<Entry>, String> {
    let mut out: Vec<Entry> = fs::read_dir(path)
        .map_err(|e| format!("{path}: {e}"))?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            let dir = e.path().is_dir();
            Some(Entry {
                path: e.path().to_string_lossy().into_owned(),
                name,
                dir,
            })
        })
        .collect();
    out.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
    Ok(out)
}

fn walk(root: &Path, visit: &mut dyn FnMut(&Path, &fs::Metadata) -> bool) {
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_dir() {
                if !skip(&name) {
                    stack.push(e.path());
                }
            } else if ty.is_file() {
                seen += 1;
                let Ok(meta) = e.metadata() else { continue };
                if seen > MAX_WALK || !visit(&e.path(), &meta) {
                    return;
                }
            }
        }
    }
}

/// Texts of the project's C and C++ files, capped so the memory stays small.
pub fn source_texts(root: &str) -> Vec<(PathBuf, String)> {
    const MAX_FILES: usize = 400;
    const MAX_BYTES: usize = 3 << 20;
    let mut out = vec![];
    let mut total = 0usize;
    walk(Path::new(root), &mut |p, meta| {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx")
            || meta.len() > 256 << 10
        {
            return true;
        }
        if out.len() >= MAX_FILES || total + meta.len() as usize > MAX_BYTES {
            return false;
        }
        if let Ok(text) = fs::read_to_string(p) {
            total += text.len();
            out.push((p.to_path_buf(), text));
        }
        true
    });
    out
}

pub fn list_files(root: &str) -> Vec<String> {
    let mut out = vec![];
    let base = Path::new(root);
    walk(base, &mut |p, _| {
        if let Ok(rel) = p.strip_prefix(base) {
            out.push(rel.to_string_lossy().into_owned());
        }
        true
    });
    out.sort();
    out
}

#[derive(Serialize)]
pub struct Hit {
    pub path: String,
    pub rel: String,
    pub line: usize,
    pub col: usize,
    pub text: String,
}

pub fn grep(root: &str, query: &str) -> Vec<Hit> {
    let mut hits = vec![];
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return hits;
    }
    let base = Path::new(root);
    walk(base, &mut |p, meta| {
        if meta.len() > MAX_GREP_FILE {
            return true;
        }
        let Ok(bytes) = fs::read(p) else { return true };
        if bytes.iter().take(4096).any(|b| *b == 0) {
            return true;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return true;
        };
        for (i, l) in text.lines().enumerate() {
            let lower = l.to_lowercase();
            if let Some(b) = lower.find(&needle) {
                hits.push(Hit {
                    path: p.to_string_lossy().into_owned(),
                    rel: p
                        .strip_prefix(base)
                        .unwrap_or(p)
                        .to_string_lossy()
                        .into_owned(),
                    line: i,
                    col: lower[..b].encode_utf16().count(),
                    text: l.trim().chars().take(160).collect(),
                });
                if hits.len() >= MAX_GREP_HITS {
                    return false;
                }
            }
        }
        true
    });
    hits.sort_by(|a, b| (&a.rel, a.line).cmp(&(&b.rel, b.line)));
    hits
}

pub fn create_project(parent: &str, name: &str, cpp: bool) -> Result<String, String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("Pick a simple folder name".into());
    }
    let dir = Path::new(parent).join(name);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    let target: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let (lang, std_line, file, body) = if cpp {
        (
            "CXX",
            "set(CMAKE_CXX_STANDARD 20)\nset(CMAKE_CXX_STANDARD_REQUIRED ON)",
            "main.cpp",
            "#include <iostream>\n\nint main() {\n    std::cout << \"Hello, world!\\n\";\n    return 0;\n}\n",
        )
    } else {
        (
            "C",
            "set(CMAKE_C_STANDARD 17)\nset(CMAKE_C_STANDARD_REQUIRED ON)",
            "main.c",
            "#include <stdio.h>\n\nint main(void) {\n    printf(\"Hello, world!\\n\");\n    return 0;\n}\n",
        )
    };
    let cmake = format!("cmake_minimum_required(VERSION 3.16)\nproject({target} LANGUAGES {lang})\n\n{std_line}\n\nadd_executable({target} src/{file})\n");
    fs::write(dir.join("CMakeLists.txt"), cmake).map_err(|e| e.to_string())?;
    fs::write(dir.join("src").join(file), body).map_err(|e| e.to_string())?;
    fs::write(dir.join(".gitignore"), "cmake-build-*/\n.clight/\n").map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().into_owned())
}

const UNBUFFERED_C: &str = "#include <stdio.h>\n\
__attribute__((constructor)) static void clight_unbuffered(void) { setvbuf(stdout, NULL, _IONBF, 0); }\n";

/// A pipe makes stdout fully buffered, so a crash would lose everything printed so far.
pub fn write_unbuffered_helper(dir: &Path) -> Result<String, String> {
    let path = dir.join("clight_unbuffered.c");
    fs::write(&path, UNBUFFERED_C).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[derive(Serialize)]
pub struct Target {
    pub name: String,
    pub path: String,
}

pub fn request_codemodel(build: &Path) -> Result<(), String> {
    let q = build.join(".cmake/api/v1/query");
    fs::create_dir_all(&q).map_err(|e| e.to_string())?;
    fs::write(q.join("codemodel-v2"), "").map_err(|e| e.to_string())
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(p).ok()?).ok()
}

pub fn cmake_targets(build: &Path) -> Vec<Target> {
    let reply = build.join(".cmake/api/v1/reply");
    let Ok(rd) = fs::read_dir(&reply) else {
        return vec![];
    };
    let mut names: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("codemodel-v2-"))
        })
        .collect();
    names.sort_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    let Some(model) = names.last().and_then(|p| read_json(p)) else {
        return vec![];
    };
    let mut out = vec![];
    for cfg in model["configurations"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
    {
        for t in cfg["targets"].as_array().map_or(&[][..], Vec::as_slice) {
            let Some(file) = t["jsonFile"].as_str() else {
                continue;
            };
            let Some(info) = read_json(&reply.join(file)) else {
                continue;
            };
            if info["type"] != "EXECUTABLE" {
                continue;
            }
            let (Some(name), Some(art)) =
                (info["name"].as_str(), info["artifacts"][0]["path"].as_str())
            else {
                continue;
            };
            let art = Path::new(art);
            let path = if art.is_absolute() {
                art.to_path_buf()
            } else {
                build.join(art)
            };
            out.push(Target {
                name: name.to_string(),
                path: path.to_string_lossy().into_owned(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[derive(Serialize)]
pub struct Sys {
    pub cores: usize,
    pub total_mb: u64,
    pub avail_mb: u64,
    pub home: String,
}

#[cfg(target_os = "linux")]
fn memory_mb() -> (u64, u64) {
    let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |name: &str| {
        info.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .unwrap_or(0)
            / 1024
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

#[cfg(windows)]
fn memory_mb() -> (u64, u64) {
    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total: u64,
        avail: u64,
        total_page: u64,
        avail_page: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }
    let mut s = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        load: 0,
        total: 0,
        avail: 0,
        total_page: 0,
        avail_page: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended: 0,
    };
    if unsafe { GlobalMemoryStatusEx(&mut s) } == 0 {
        return (0, 0);
    }
    (s.total >> 20, s.avail >> 20)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn memory_mb() -> (u64, u64) {
    (0, 0)
}

pub fn sys_info() -> Sys {
    let (total, avail) = memory_mb();
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/".into());
    Sys {
        cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
        total_mb: total,
        avail_mb: avail,
        home,
    }
}

/// Where `needle` (already lowercase) is in `hay`, ignoring case.
fn find_ci(hay: &str, needle: &str, from: usize) -> Option<(usize, usize)> {
    let lower = hay.to_lowercase();
    if lower.len() == hay.len() {
        return lower[from..].find(needle).map(|i| (from + i, from + i + needle.len()));
    }
    // Some letters change length in lowercase: fall back to comparing ASCII only.
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    (from..=h.len().checked_sub(n.len())?)
        .find(|&i| hay.is_char_boundary(i) && hay.is_char_boundary(i + n.len()) && h[i..i + n.len()].eq_ignore_ascii_case(n))
        .map(|i| (i, i + n.len()))
}

/// Replaces `query` with `with` in every text file of the project, ignoring case.
/// Returns how many files and how many places changed.
pub fn replace_all(root: &str, query: &str, with: &str, skip: &[PathBuf]) -> Result<(usize, usize), String> {
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return Ok((0, 0));
    }
    let base = Path::new(root);
    let (mut files, mut places) = (0, 0);
    let mut failure: Option<String> = None;
    walk(base, &mut |p, meta| {
        if meta.len() > MAX_GREP_FILE || skip.iter().any(|s| s == p) {
            return true;
        }
        let Ok(bytes) = fs::read(p) else { return true };
        if bytes.iter().take(4096).any(|b| *b == 0) {
            return true;
        }
        let Ok(text) = String::from_utf8(bytes) else { return true };
        let mut out = String::with_capacity(text.len());
        let (mut at, mut n) = (0, 0);
        while let Some((s, e)) = find_ci(&text, &needle, at) {
            out.push_str(&text[at..s]);
            out.push_str(with);
            at = e;
            n += 1;
        }
        if n == 0 {
            return true;
        }
        out.push_str(&text[at..]);
        let tmp = p.with_extension(format!("replace-{}.tmp", std::process::id()));
        let written = fs::write(&tmp, out).and_then(|()| {
            if let Ok(m) = fs::metadata(p) {
                let _ = fs::set_permissions(&tmp, m.permissions());
            }
            fs::rename(&tmp, p)
        });
        match written {
            Ok(()) => {
                files += 1;
                places += n;
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                failure = Some(format!("{}: {e}", p.display()));
                return false;
            }
        }
        true
    });
    match failure {
        Some(e) => Err(e),
        None => Ok((files, places)),
    }
}

#[cfg(test)]
mod replace_tests {
    use super::*;

    #[test]
    fn replacing_in_the_project_ignores_case_and_keeps_line_endings() {
        let dir = std::env::temp_dir().join(format!("proj-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/a.txt"), "Foo foo FOO\r\nbar\r\n").unwrap();
        fs::write(dir.join("src/b.txt"), "nothing\n").unwrap();
        fs::write(dir.join("src/bin.dat"), b"foo\0foo").unwrap();
        let (files, places) = replace_all(dir.to_str().unwrap(), "Foo", "baz", &[]).unwrap();
        assert_eq!((files, places), (1, 3));
        assert_eq!(fs::read_to_string(dir.join("src/a.txt")).unwrap(), "baz baz baz\r\nbar\r\n");
        assert_eq!(fs::read(dir.join("src/bin.dat")).unwrap(), b"foo\0foo");
        let skipped = dir.join("src/a.txt");
        fs::write(&skipped, "foo").unwrap();
        assert_eq!(replace_all(dir.to_str().unwrap(), "foo", "x", std::slice::from_ref(&skipped)).unwrap(), (0, 0));
        assert_eq!(find_ci("İfoo", "foo", 0), Some((2, 5)));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names_cannot_escape_the_parent() {
        assert!(create_project("/tmp", "../evil", true).is_err());
        assert!(create_project("/tmp", "", true).is_err());
    }
}
