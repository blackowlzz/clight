use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

static FOUND: Mutex<Option<tauri_plugin_updater::Update>> = Mutex::new(None);
static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

#[derive(Serialize, Clone)]
struct Progress {
    done: u64,
    total: Option<u64>,
}

pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        notes: u.body.clone().unwrap_or_default(),
    });
    *FOUND.lock().unwrap() = update;
    Ok(info)
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("The update is already being installed".into());
    }
    let result = async {
        let update = FOUND
            .lock()
            .unwrap()
            .clone()
            .ok_or("No update to install")?;
        let mut done = 0u64;
        let bytes = update
            .download(
                |chunk, total| {
                    done += chunk as u64;
                    let _ = app.emit("update-progress", Progress { done, total });
                },
                || {},
            )
            .await
            .map_err(|e| format!("Couldn't download the update: {e}"))?;
        update
            .install(&bytes)
            .map_err(|e| format!("Couldn't install the update: {e}"))
    }
    .await;
    if result.is_err() {
        INSTALLING.store(false, Ordering::SeqCst);
    }
    result
}
