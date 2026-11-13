//! Updates from the GitHub releases of the app. The latest release is compared with this version,
//! and its installer for the way the app was installed is downloaded, checked against the size and
//! the SHA-256 that GitHub lists for it, and run. No signing key is involved, so none can be lost.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const REPO: &str = "blackowlzz/clight";
const NAME: &str = "clight";
/// Far more than any installer of the app; a bigger file is not ours.
const MAX_SIZE: u64 = 400 << 20;

static FOUND: Mutex<Option<Found>> = Mutex::new(None);
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// How this copy of the app was installed, which decides the file to download.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    AppImage,
    Deb,
    Rpm,
    Nsis,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize, Clone)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Clone, Debug)]
struct Found {
    version: String,
    notes: String,
    page: String,
    /// None when this copy can't update itself (a build that is not from an installer, or a
    /// release without a file for this system): the page is offered instead.
    file: Option<(Kind, String, String, u64, Option<String>)>,
}

#[derive(Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    /// The update has to be downloaded from the release page by hand.
    pub manual: bool,
}

#[derive(Serialize, Clone)]
struct Progress {
    done: u64,
    total: Option<u64>,
}

/// What the app does once the update is in place.
#[derive(PartialEq)]
pub enum After {
    Restart,
    /// The Windows installer is running and starts the app again when it is done.
    Exit,
}

fn this_kind() -> Option<Kind> {
    use tauri::utils::config::BundleType;
    match tauri::utils::platform::bundle_type()? {
        BundleType::AppImage if std::env::var_os("APPIMAGE").is_some() => Some(Kind::AppImage),
        BundleType::Deb => Some(Kind::Deb),
        BundleType::Rpm => Some(Kind::Rpm),
        BundleType::Nsis => Some(Kind::Nsis),
        _ => None,
    }
}

/// The numbers of a version, without a leading v or anything after a dash.
fn numbers(v: &str) -> Vec<u64> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split(['-', '+']).next().unwrap_or("");
    v.split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

fn newer(latest: &str, current: &str) -> bool {
    let (mut a, mut b) = (numbers(latest), numbers(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

fn pick(assets: &[Asset], kind: Kind, arch: &str) -> Option<Asset> {
    let x64 = arch == "x86_64";
    let (end, tag) = match kind {
        Kind::AppImage => (".AppImage", if x64 { "_amd64" } else { "_aarch64" }),
        Kind::Deb => (".deb", if x64 { "_amd64" } else { "_arm64" }),
        Kind::Rpm => (".rpm", if x64 { ".x86_64" } else { ".aarch64" }),
        Kind::Nsis => ("-setup.exe", if x64 { "_x64" } else { "_arm64" }),
    };
    assets.iter().find(|a| a.name.ends_with(end) && a.name.contains(tag)).cloned()
}

fn found_in(release: &Release, current: &str, kind: Option<Kind>, arch: &str) -> Option<Found> {
    if !newer(&release.tag_name, current) {
        return None;
    }
    let file = kind.and_then(|k| {
        let a = pick(&release.assets, k, arch)?;
        let sha = a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase);
        Some((k, a.name, a.browser_download_url, a.size, sha))
    });
    Some(Found {
        version: release.tag_name.trim_start_matches('v').to_string(),
        notes: release.body.clone().unwrap_or_default(),
        page: release.html_url.clone(),
        file,
    })
}

fn client() -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(NAME)
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

async fn latest() -> Result<Option<Release>, String> {
    let res = client()?
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub can't be reached: {e}"))?;
    // No release yet.
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let text = res.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map(Some).map_err(|e| format!("Unexpected answer from GitHub: {e}"))
}

pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    let found = latest().await?.and_then(|r| found_in(&r, &current, this_kind(), std::env::consts::ARCH));
    let info = found.as_ref().map(|f| UpdateInfo { version: f.version.clone(), notes: f.notes.clone(), manual: f.file.is_none() });
    *FOUND.lock().unwrap() = found;
    Ok(info)
}

/// Opens the page of the release that was found, for a copy that can't update itself.
pub fn open_page() -> Result<(), String> {
    let page = FOUND.lock().unwrap().as_ref().map(|f| f.page.clone()).ok_or("No update was found")?;
    if !page.starts_with(&format!("https://github.com/{REPO}/")) {
        return Err("Unexpected release page".into());
    }
    #[cfg(windows)]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(not(windows))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&page).spawn().map(|_| ()).map_err(|e| format!("Couldn't open {page}: {e}"))
}

async fn download(url: &str, size: u64, sha: Option<&str>, dest: &Path, mut progress: impl FnMut(u64)) -> Result<(), String> {
    if size > MAX_SIZE {
        return Err("The update is too big".into());
    }
    let mut res = client()?
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Couldn't download the update: {e}"))?;
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut done = 0u64;
    while let Some(chunk) = res.chunk().await.map_err(|e| format!("Couldn't download the update: {e}"))? {
        done += chunk.len() as u64;
        if done > size {
            return Err("The update is bigger than GitHub says".into());
        }
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        hash.update(&chunk);
        progress(done);
    }
    file.sync_all().map_err(|e| e.to_string())?;
    if done != size {
        return Err("The download of the update was cut short".into());
    }
    if let Some(want) = sha {
        if format!("{:x}", hash.finalize()) != want {
            return Err("The update doesn't match the file on GitHub".into());
        }
    }
    Ok(())
}

pub async fn install(app: &AppHandle) -> Result<After, String> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("The update is already being installed".into());
    }
    let result = async {
        let found = FOUND.lock().unwrap().clone().ok_or("No update to install")?;
        let (kind, name, url, size, sha) = found.file.clone().ok_or("This copy can't update itself: download the update from its page")?;
        let dir = std::env::temp_dir().join(format!("{NAME}-update"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // Only the name of the file: nothing from the answer decides where it is written.
        let file = dir.join(Path::new(&name).file_name().ok_or("Unexpected file name")?);
        download(&url, size, sha.as_deref(), &file, |done| {
            let _ = app.emit("update-progress", Progress { done, total: Some(size) });
        })
        .await?;
        let after = put_in_place(kind, &file).map_err(|e| format!("Couldn't install the update: {e}"))?;
        Ok::<_, String>(after)
    }
    .await;
    if result.is_err() {
        INSTALLING.store(false, Ordering::SeqCst);
    }
    result
}

fn put_in_place(kind: Kind, file: &Path) -> Result<After, String> {
    match kind {
        Kind::Nsis => {
            // Like Tauri's own updater: passive, as an update, and the app starts again after.
            std::process::Command::new(file).args(["/P", "/UPDATE", "/R"]).spawn().map_err(|e| e.to_string())?;
            Ok(After::Exit)
        }
        #[cfg(unix)]
        Kind::AppImage => {
            let current = std::path::PathBuf::from(std::env::var_os("APPIMAGE").ok_or("Not running as an AppImage")?);
            replace_appimage(file, &current)?;
            let _ = std::fs::remove_file(file);
            Ok(After::Restart)
        }
        #[cfg(not(unix))]
        Kind::AppImage => Err("AppImages are for Linux".into()),
        Kind::Deb | Kind::Rpm => {
            let (tool, arg) = if kind == Kind::Deb { ("dpkg", "-i") } else { ("rpm", "-U") };
            let status = std::process::Command::new("pkexec").args([tool, arg]).arg(file).status().map_err(|e| format!("pkexec is needed to install it: {e}"))?;
            let _ = std::fs::remove_file(file);
            if !status.success() {
                return Err(format!("{tool} didn't install it"));
            }
            Ok(After::Restart)
        }
    }
}

/// Writes the new AppImage next to the old one and renames it over it, so that a failure leaves
/// the old one working.
#[cfg(unix)]
fn replace_appimage(new: &Path, current: &Path) -> Result<(), String> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let mut magic = [0u8; 4];
    std::fs::File::open(new).and_then(|mut f| f.read_exact(&mut magic)).map_err(|e| e.to_string())?;
    if &magic != b"\x7fELF" {
        return Err("the download is not an AppImage".into());
    }
    let dir = current.parent().ok_or("no folder for the AppImage")?;
    let next = dir.join(format!(".{NAME}-update.AppImage"));
    let mode = std::fs::metadata(current).map(|m| m.permissions().mode()).unwrap_or(0o755) | 0o100;
    let copy = std::fs::copy(new, &next)
        .and_then(|_| std::fs::set_permissions(&next, std::fs::Permissions::from_mode(mode)))
        .and_then(|_| std::fs::rename(&next, current));
    if let Err(e) = copy {
        let _ = std::fs::remove_file(&next);
        return Err(format!("{} can't be written: {e}", dir.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset { name: name.into(), size: 10, browser_download_url: format!("https://github.com/{REPO}/releases/download/v0.2.0/{name}"), digest: Some("sha256:ABCD".into()) }
    }

    fn release() -> Release {
        let names = ["clight-0.2.0-1.x86_64.rpm", "clight-0.2.0-1.x86_64.rpm.sig", "clight_0.2.0_amd64.AppImage", "clight_0.2.0_amd64.AppImage.sig", "clight_0.2.0_amd64.deb", "clight_0.2.0_x64-setup.exe", "clight_0.2.0_x64-setup.exe.sig", "latest.json"];
        Release { tag_name: "v0.2.0".into(), body: Some("notes".into()), html_url: format!("https://github.com/{REPO}/releases/tag/v0.2.0"), assets: names.iter().map(|n| asset(n)).collect() }
    }

    #[test]
    fn versions_are_compared_as_numbers() {
        assert!(newer("v0.1.10", "0.1.9"));
        assert!(newer("0.2", "0.1.13"));
        assert!(!newer("v0.1.13", "0.1.13"));
        assert!(!newer("0.1.12", "0.1.13"));
        assert!(!newer("v0.1.13-beta", "0.1.13"));
    }

    #[test]
    fn the_file_for_each_kind_of_install_is_picked() {
        let r = release();
        let name = |k| pick(&r.assets, k, "x86_64").map(|a| a.name);
        assert_eq!(name(Kind::AppImage).as_deref(), Some("clight_0.2.0_amd64.AppImage"));
        assert_eq!(name(Kind::Deb).as_deref(), Some("clight_0.2.0_amd64.deb"));
        assert_eq!(name(Kind::Rpm).as_deref(), Some("clight-0.2.0-1.x86_64.rpm"));
        assert_eq!(name(Kind::Nsis).as_deref(), Some("clight_0.2.0_x64-setup.exe"));
        assert!(pick(&r.assets, Kind::Nsis, "aarch64").is_none());
    }

    #[test]
    fn a_newer_release_is_found_with_its_checksum() {
        let f = found_in(&release(), "0.1.13", Some(Kind::Nsis), "x86_64").unwrap();
        assert_eq!(f.version, "0.2.0");
        let (kind, name, _, size, sha) = f.file.unwrap();
        assert_eq!((kind, name.as_str(), size, sha.as_deref()), (Kind::Nsis, "clight_0.2.0_x64-setup.exe", 10, Some("abcd")));
        assert!(found_in(&release(), "0.2.0", Some(Kind::Nsis), "x86_64").is_none());
        // A copy that is not from an installer gets the page.
        assert!(found_in(&release(), "0.1.13", None, "x86_64").unwrap().file.is_none());
    }

    #[test]
    fn the_answer_of_github_is_read() {
        let json = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/x/y/releases/tag/v0.2.0","body":null,"draft":false,
            "assets":[{"name":"a.deb","size":3,"browser_download_url":"https://x/a.deb","digest":"sha256:00","state":"uploaded"}]}"#;
        let r: Release = serde_json::from_str(json).unwrap();
        assert_eq!((r.assets.len(), r.assets[0].digest.as_deref()), (1, Some("sha256:00")));
    }

    /// Talks to GitHub and downloads the Windows installer of the latest release:
    /// `cargo test real_release -- --ignored`.
    #[test]
    #[ignore]
    fn real_release_is_found_and_downloaded_whole() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let release = latest().await.unwrap().expect("no release yet");
            let f = found_in(&release, "0.0.1", Some(Kind::Nsis), "x86_64").unwrap();
            let (_, name, url, size, sha) = f.file.expect("no installer in the release");
            assert!(sha.is_some(), "GitHub gave no checksum for {name}");
            let dest = std::env::temp_dir().join(format!("update-test-{}", std::process::id()));
            let mut last = 0;
            download(&url, size, sha.as_deref(), &dest, |d| last = d).await.unwrap();
            assert_eq!((last, std::fs::metadata(&dest).unwrap().len()), (size, size));
            // A wrong checksum is refused.
            assert!(download(&url, size, Some("00"), &dest, |_| {}).await.is_err());
            let _ = std::fs::remove_file(&dest);
            println!("{name}: {size} bytes, sha256 {}", sha.unwrap());
        });
    }

    #[cfg(unix)]
    #[test]
    fn an_appimage_is_replaced_only_by_an_executable() {
        let dir = std::env::temp_dir().join(format!("appimage-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let current = dir.join("app.AppImage");
        std::fs::write(&current, b"\x7fELFold").unwrap();
        let bad = dir.join("bad");
        std::fs::write(&bad, b"<html>").unwrap();
        assert!(replace_appimage(&bad, &current).is_err());
        assert_eq!(std::fs::read(&current).unwrap(), b"\x7fELFold");
        let good = dir.join("good");
        std::fs::write(&good, b"\x7fELFnew").unwrap();
        replace_appimage(&good, &current).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"\x7fELFnew");
        use std::os::unix::fs::PermissionsExt;
        assert!(std::fs::metadata(&current).unwrap().permissions().mode() & 0o100 != 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
