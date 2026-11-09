mod vault;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use vault::{lock_path, unlock_path, LockResult, ProgressEvent, UnlockResult};

#[derive(Serialize, Deserialize, Clone)]
pub struct RecentEntry {
    pub name: String,
    pub kind: String,
    pub vault_path: String,
    pub size_bytes: u64,
    pub locked_at_unix: u64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn recent_file(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    let _ = fs::create_dir_all(&dir);
    Some(dir.join("recent.json"))
}

fn load_recent(app: &AppHandle) -> Vec<RecentEntry> {
    let Some(path) = recent_file(app) else {
        return vec![];
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return vec![];
    };
    let mut entries: Vec<RecentEntry> = serde_json::from_str(&raw).unwrap_or_default();
    entries.retain(|e| PathBuf::from(&e.vault_path).exists());
    entries
}

fn save_recent(app: &AppHandle, entries: &[RecentEntry]) {
    if let Some(path) = recent_file(app) {
        let _ = fs::write(path, serde_json::to_string(entries).unwrap_or_default());
    }
}

#[tauri::command]
fn pick_file() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose a file to lock")
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn pick_folder() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose a folder to lock")
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn pick_vault() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose a vault to unlock")
        .add_filter("VaultLock vault", &["vaultlock"])
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
async fn lock_item(
    app: AppHandle,
    source: String,
    password: String,
) -> Result<LockResult, String> {
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let progress = |event: ProgressEvent| {
            let _ = handle.emit("vault-progress", &event);
        };
        lock_path(&PathBuf::from(&source), &password, &progress)
    })
    .await
    .map_err(|e| format!("Lock task failed: {e}"))??;
    let mut entries = load_recent(&app);
    entries.retain(|e| e.vault_path != result.vault_path);
    let size_bytes = fs::metadata(&result.vault_path).map(|m| m.len()).unwrap_or(0);
    entries.insert(
        0,
        RecentEntry {
            name: result.name.clone(),
            kind: result.kind.clone(),
            vault_path: result.vault_path.clone(),
            size_bytes,
            locked_at_unix: now_unix(),
        },
    );
    entries.truncate(50);
    save_recent(&app, &entries);
    Ok(result)
}

#[tauri::command]
async fn unlock_item(
    app: AppHandle,
    vault_path: String,
    password: String,
) -> Result<UnlockResult, String> {
    let handle = app.clone();
    let vault_path_for_recent = vault_path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let progress = |event: ProgressEvent| {
            let _ = handle.emit("vault-progress", &event);
        };
        unlock_path(&PathBuf::from(&vault_path), &password, &progress)
    })
    .await
    .map_err(|e| format!("Unlock task failed: {e}"))??;
    let mut entries = load_recent(&app);
    entries.retain(|e| e.vault_path != vault_path_for_recent);
    save_recent(&app, &entries);
    Ok(result)
}

#[tauri::command]
fn get_recent(app: AppHandle) -> Vec<RecentEntry> {
    load_recent(&app)
}

#[tauri::command]
fn forget_recent(app: AppHandle, vault_path: String) {
    let mut entries = load_recent(&app);
    entries.retain(|e| e.vault_path != vault_path);
    save_recent(&app, &entries);
}

#[tauri::command]
fn reveal_in_folder(path: String) -> Result<(), String> {
    let target = PathBuf::from(&path);
    let folder = if target.is_dir() {
        target
    } else {
        target.parent().map(|p| p.to_path_buf()).unwrap_or(target)
    };
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("explorer")
        .arg(&folder)
        .status();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(&folder).status();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let status = std::process::Command::new("xdg-open")
        .arg(&folder)
        .status();
    status
        .map(|_| ())
        .map_err(|e| format!("Could not open folder: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            pick_file,
            pick_folder,
            pick_vault,
            lock_item,
            unlock_item,
            get_recent,
            forget_recent,
            reveal_in_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running VaultLock");
}
