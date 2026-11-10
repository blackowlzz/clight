use crate::runner::clean_env;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use zip::ZipArchive;

#[derive(Clone, Copy)]
enum Kind {
    Zip,
    SelfExtracting,
    TarGz,
    File,
}

#[derive(Clone, Copy)]
enum Os {
    All,
    Windows,
    Linux,
}

struct Tool {
    id: &'static str,
    title: &'static str,
    detail: &'static str,
    size: u64,
    url: &'static str,
    sha256: &'static str,
    bin: &'static str,
    exe: &'static str,
    kind: Kind,
    group: &'static str,
    os: Os,
    recommended: bool,
}

static TOOLS: [Tool; 9] = [
    Tool {
        id: "clangd",
        title: "clangd",
        detail: "Live errors, completion, go to definition and rename",
        size: 29_690_805,
        url: "https://github.com/clangd/clangd/releases/download/23.1.0/clangd-windows-23.1.0.zip",
        sha256: "23412a240756a162e7b98a282f36aa2a23a88db5ce16a0cbc4fef7253768c810",
        bin: "clangd_23.1.0/bin",
        exe: "clangd.exe",
        kind: Kind::Zip,
        group: "dev",
        os: Os::Windows,
        recommended: true,
    },
    Tool {
        id: "clang-format",
        title: "clang-format",
        detail: "Reformats your code with the style from Settings",
        size: 1_537_547,
        url: "https://files.pythonhosted.org/packages/7b/67/b91fb3483ca5cdb468fa5e72d836736de5ec29154cc3f58718de6192bc12/clang_format-23.1.5-py2.py3-none-win_amd64.whl",
        sha256: "18a4a1a698dd52ed4e288c97d99e93910f8f9d0128b266f65021ecf39c85d034",
        bin: "clang_format/data/bin",
        exe: "clang-format.exe",
        kind: Kind::Zip,
        group: "dev",
        os: Os::Windows,
        recommended: true,
    },
    Tool {
        id: "gcc",
        title: "GCC compiler",
        detail: "w64devkit: gcc and g++, needed to build and run",
        size: 68_801_240,
        url: "https://github.com/skeeto/w64devkit/releases/download/v2.11.0/w64devkit-x64-2.11.0.7z.exe",
        sha256: "54c2bb9962e8a30c8270e34c5efc3d5fee579f53087fa7d435b95fbba2af7b2b",
        bin: "w64devkit/bin",
        exe: "gcc.exe",
        kind: Kind::SelfExtracting,
        group: "dev",
        os: Os::Windows,
        recommended: true,
    },
    Tool {
        id: "cmake",
        title: "CMake",
        detail: "Builds projects with a CMakeLists.txt",
        size: 54_412_232,
        url: "https://github.com/Kitware/CMake/releases/download/v4.4.5/cmake-4.4.5-windows-x86_64.zip",
        sha256: "326c826486d46a23cf9d5a1c9ee3a567d427bdd02c4a34071937a2a675cff004",
        bin: "cmake-4.4.5-windows-x86_64/bin",
        exe: "cmake.exe",
        kind: Kind::Zip,
        group: "dev",
        os: Os::Windows,
        recommended: true,
    },
    Tool {
        id: "ninja",
        title: "Ninja",
        detail: "Lets CMake build with GCC",
        size: 291_570,
        url: "https://github.com/ninja-build/ninja/releases/download/v1.13.2/ninja-win.zip",
        sha256: "07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65",
        bin: "",
        exe: "ninja.exe",
        kind: Kind::Zip,
        group: "dev",
        os: Os::Windows,
        recommended: true,
    },
    Tool {
        id: "git",
        title: "Git",
        detail: "MinGit, for the commit panel",
        size: 39_846_926,
        url: "https://github.com/git-for-windows/git/releases/download/v2.56.0.windows.4/MinGit-2.56.0.4-64-bit.zip",
        sha256: "e3bf78351fe777fba47ddee5950082f8a98bb0f02604fdc7ba3ee2cb68ffa877",
        bin: "cmd",
        exe: "git.exe",
        kind: Kind::Zip,
        group: "dev",
        os: Os::Windows,
        recommended: false,
    },
    Tool {
        id: "ai-engine",
        title: "AI engine",
        detail: "llama.cpp, runs the small AI on your CPU",
        size: 24_931_485,
        url: "https://github.com/ggml-org/llama.cpp/releases/download/b12246/llama-b12246-bin-win-cpu-x64.zip",
        sha256: "4ff558826078209d9502037f1bb751fa229ecff246ec9362ec2632d08b2948b4",
        bin: "",
        exe: "llama-server.exe",
        kind: Kind::Zip,
        group: "ai",
        os: Os::Windows,
        recommended: false,
    },
    Tool {
        id: "ai-engine",
        title: "AI engine",
        detail: "llama.cpp, runs the small AI on your CPU",
        size: 23_158_204,
        url: "https://github.com/ggml-org/llama.cpp/releases/download/b12246/llama-b12246-bin-ubuntu-x64.tar.gz",
        sha256: "791f48a17841ddcce8a8552dfbbac108fda72389f3dd81508691468e8f114c89",
        bin: "llama-b12246",
        exe: "llama-server",
        kind: Kind::TarGz,
        group: "ai",
        os: Os::Linux,
        recommended: false,
    },
    Tool {
        id: "ai-model",
        title: "Small AI model",
        detail: "Qwen2.5-Coder 0.5B, a small model for code",
        size: 397_808_288,
        url: "https://huggingface.co/bartowski/Qwen2.5-Coder-0.5B-Instruct-GGUF/resolve/main/Qwen2.5-Coder-0.5B-Instruct-Q4_K_M.gguf",
        sha256: "0128e77564e43d40682f82d7ebe8a9abdf0c24c8f55fa85629f8cc156b1b6560",
        bin: "",
        exe: "model.gguf",
        kind: Kind::File,
        group: "ai",
        os: Os::All,
        recommended: false,
    },
];

static BUSY: AtomicBool = AtomicBool::new(false);

fn applies(t: &Tool) -> bool {
    match t.os {
        Os::All => true,
        Os::Windows => cfg!(windows) || (cfg!(feature = "selftest") && t.group == "dev"),
        Os::Linux => cfg!(target_os = "linux"),
    }
}

fn catalog() -> Vec<&'static Tool> {
    TOOLS.iter().filter(|t| applies(t)).collect()
}

fn find(id: &str) -> Option<&'static Tool> {
    catalog().into_iter().find(|t| t.id == id)
}

/// Where a tool that clight downloaded itself lives, when it is there.
pub fn tool_path(app: &AppHandle, id: &str) -> Option<PathBuf> {
    let t = find(id)?;
    let p = tools_dir(app).ok()?.join(t.id).join(t.bin).join(t.exe);
    p.is_file().then_some(p)
}

pub fn remove(app: &AppHandle, id: &str) {
    if let Ok(dir) = tools_dir(app) {
        let _ = fs::remove_dir_all(dir.join(id));
        if id == "ai-engine" {
            let _ = fs::remove_file(dir.join("ai-engine.log"));
        }
    }
}

fn tools_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("tools"))
}

pub fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find(|d| d.join(exe).is_file())
}

fn usual_install_dirs() -> Vec<PathBuf> {
    let llvm = ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .flat_map(|d| {
            let d = PathBuf::from(d);
            [d.join(r"LLVM\bin"), d.join(r"Programs\LLVM\bin")]
        });
    let msys2 = ["ucrt64", "clang64", "mingw64"]
        .iter()
        .map(|e| Path::new(r"C:\msys64").join(e).join("bin"));
    llvm.chain(msys2).collect()
}

/// Lets every program clight starts find the tools, without touching the user's Path.
pub fn extend_path(app: &AppHandle) {
    if !catalog().iter().any(|t| t.group == "dev") {
        return;
    }
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&current).collect();
    let before = dirs.len();
    let mut extra = usual_install_dirs();
    if let Ok(tools) = tools_dir(app) {
        extra.extend(
            catalog()
                .iter()
                .filter(|t| t.group == "dev")
                .map(|t| tools.join(t.id).join(t.bin)),
        );
    }
    for d in extra {
        if d.is_dir() && !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    if dirs.len() != before {
        if let Ok(path) = std::env::join_paths(dirs) {
            std::env::set_var("PATH", path);
        }
    }
}

#[derive(Serialize)]
pub struct Status {
    id: &'static str,
    title: &'static str,
    detail: &'static str,
    size: u64,
    recommended: bool,
    installed: bool,
    group: &'static str,
}

pub fn status(app: &AppHandle) -> Vec<Status> {
    catalog()
        .iter()
        .map(|t| Status {
            id: t.id,
            title: t.title,
            detail: t.detail,
            size: t.size,
            recommended: t.recommended,
            installed: if t.group == "ai" {
                tool_path(app, t.id).is_some()
            } else {
                find_on_path(t.exe).is_some()
            },
            group: t.group,
        })
        .collect()
}

#[derive(Serialize, Clone)]
struct Progress {
    id: &'static str,
    done: u64,
    total: u64,
}

async fn fetch(app: &AppHandle, tool: &Tool, dest: &Path) -> Result<(), String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .user_agent("clight")
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(tool.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let mut file = File::create(dest).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let (mut done, mut shown) = (0u64, 0u64);
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        hash.update(&chunk);
        done += chunk.len() as u64;
        if done - shown >= 1 << 18 || done == tool.size {
            shown = done;
            let _ = app.emit(
                "tools-progress",
                Progress {
                    id: tool.id,
                    done,
                    total: tool.size,
                },
            );
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    if format!("{:x}", hash.finalize()) != tool.sha256 {
        return Err("The download is corrupted".into());
    }
    Ok(())
}

fn unzip(archive: &Path, into: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let to = into.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = File::create(&to).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn unpack(kind: Kind, archive: &Path, into: &Path) -> Result<(), String> {
    fs::create_dir_all(into).map_err(|e| e.to_string())?;
    match kind {
        Kind::Zip => unzip(archive, into),
        Kind::TarGz => {
            let status = Command::new("tar")
                .arg("-xzf")
                .arg(archive)
                .arg("-C")
                .arg(into)
                .status()
                .map_err(|e| format!("tar: {e}"))?;
            if status.success() {
                Ok(())
            } else {
                Err("The archive couldn't be unpacked".into())
            }
        }
        Kind::File => Err("A single file needs no unpacking".into()),
        Kind::SelfExtracting => {
            let mut cmd = Command::new(archive);
            clean_env(&mut cmd);
            let status = cmd
                .arg("-y")
                .arg(format!("-o{}", into.display()))
                .status()
                .map_err(|e| e.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err("The archive couldn't be unpacked".into())
            }
        }
    }
}

async fn install_one(app: &AppHandle, tool: &'static Tool) -> Result<(), String> {
    let dir = tools_dir(app)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ext = match tool.kind {
        Kind::Zip => "zip",
        Kind::SelfExtracting => "exe",
        Kind::TarGz => "tgz",
        Kind::File => "bin",
    };
    let archive = dir.join(format!("{}.download.{ext}", tool.id));
    let fresh = dir.join(format!("{}.new", tool.id));
    let _ = fs::remove_dir_all(&fresh);

    let result = async {
        fetch(app, tool, &archive).await?;
        let (a, f, kind) = (archive.clone(), fresh.clone(), tool.kind);
        if matches!(kind, Kind::File) {
            let place = fresh.join(tool.bin);
            fs::create_dir_all(&place).map_err(|e| e.to_string())?;
            fs::rename(&archive, place.join(tool.exe)).map_err(|e| e.to_string())?;
        } else {
            tauri::async_runtime::spawn_blocking(move || unpack(kind, &a, &f))
                .await
                .map_err(|e| e.to_string())??;
        }
        if !fresh.join(tool.bin).join(tool.exe).is_file() {
            return Err(format!("{} is missing from the download", tool.exe));
        }
        let target = dir.join(tool.id);
        let _ = fs::remove_dir_all(&target);
        fs::rename(&fresh, &target).map_err(|e| e.to_string())
    }
    .await;

    let _ = fs::remove_file(&archive);
    let _ = fs::remove_dir_all(&fresh);
    result
}

#[derive(Serialize)]
pub struct Failure {
    title: &'static str,
    error: String,
}

pub async fn install(app: &AppHandle, ids: &[String]) -> Result<Vec<Failure>, String> {
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("Already downloading".into());
    }
    let mut failures = vec![];
    for tool in catalog()
        .into_iter()
        .filter(|t| ids.iter().any(|i| i == t.id))
    {
        if let Err(error) = install_one(app, tool).await {
            failures.push(Failure {
                title: tool.title,
                error,
            });
        }
    }
    extend_path(app);
    BUSY.store(false, Ordering::SeqCst);
    Ok(failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn the_catalog_is_well_formed() {
        for t in &TOOLS {
            assert_eq!(t.sha256.len(), 64, "{}", t.id);
            assert!(t.url.starts_with("https://"), "{}", t.id);
            if matches!(t.os, Os::Windows) {
                assert!(t.exe.ends_with(".exe"), "{}", t.id);
            }
        }
    }

    #[test]
    fn a_tar_archive_is_unpacked_into_its_folder() {
        let base = std::env::temp_dir().join(format!("clight-tar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("src/pkg")).unwrap();
        fs::write(base.join("src/pkg/tool"), "x").unwrap();
        let archive = base.join("a.tgz");
        let made = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(base.join("src"))
            .arg("pkg")
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return;
        }
        let out = base.join("out");
        unpack(Kind::TarGz, &archive, &out).unwrap();
        assert_eq!(fs::read(out.join("pkg/tool")).unwrap(), b"x");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn unzip_keeps_entries_inside_the_target() {
        let base = std::env::temp_dir().join(format!("clight-unzip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let into = base.join("out");
        fs::create_dir_all(&into).unwrap();
        let archive = base.join("a.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        let options = SimpleFileOptions::default();
        zip.start_file("dir/ok.txt", options).unwrap();
        zip.write_all(b"fine").unwrap();
        zip.start_file("../evil.txt", options).unwrap();
        zip.write_all(b"nope").unwrap();
        zip.finish().unwrap();

        unzip(&archive, &into).unwrap();
        assert_eq!(fs::read(into.join("dir/ok.txt")).unwrap(), b"fine");
        assert!(!base.join("evil.txt").exists());
        fs::remove_dir_all(&base).unwrap();
    }
}
