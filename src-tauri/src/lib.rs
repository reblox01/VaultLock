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

/// What the app was asked to do at launch, e.g. from the Windows
/// right-click menu: `--lock <path>` or `--unlock <path>`.
#[derive(Serialize, Deserialize, Clone)]
pub struct LaunchTarget {
    pub mode: String,
    pub path: String,
}

pub fn parse_launch_target(args: &[String]) -> Option<LaunchTarget> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lock" | "--unlock" => {
                let mode = if args[i] == "--lock" { "lock" } else { "unlock" };
                return args
                    .get(i + 1)
                    .map(|path| LaunchTarget { mode: mode.to_string(), path: path.clone() });
            }
            _ => i += 1,
        }
    }
    None
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

#[tauri::command]
fn get_launch_target(state: tauri::State<Option<LaunchTarget>>) -> Option<LaunchTarget> {
    state.inner().clone()
}

// ---------- Windows right-click menu (HKCU, no admin needed) ----------
#[cfg(target_os = "windows")]
mod ctx_menu {
    use std::path::PathBuf;
    use winreg::enums::*;
    use winreg::RegKey;

    fn exe_path() -> Result<String, String> {
        let exe: PathBuf =
            std::env::current_exe().map_err(|e| format!("Could not find VaultLock exe: {e}"))?;
        Ok(exe.to_string_lossy().into_owned())
    }

    fn classes() -> Result<RegKey, String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(r"Software\Classes", KEY_ALL_ACCESS)
            .map_err(|e| format!("Could not open registry: {e}"))
    }

    pub fn supported() -> bool {
        true
    }

    pub fn status() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Classes\*\shell\VaultLock")
            .is_ok()
    }

    pub fn set(enabled: bool) -> Result<bool, String> {
        let classes = classes()?;
        if enabled {
            let exe = exe_path()?;
            // Files: right-click any file -> Lock with VaultLock
            let (key, _) = classes
                .create_subkey(r"*\shell\VaultLock")
                .map_err(|e| e.to_string())?;
            key.set_value("", &"Lock with VaultLock")
                .map_err(|e| e.to_string())?;
            key.set_value("Icon", &exe).map_err(|e| e.to_string())?;
            let (cmd, _) = classes
                .create_subkey(r"*\shell\VaultLock\command")
                .map_err(|e| e.to_string())?;
            cmd.set_value("", &format!("\"{exe}\" --lock \"%1\""))
                .map_err(|e| e.to_string())?;
            // Folders: right-click a folder -> Lock with VaultLock
            let (key, _) = classes
                .create_subkey(r"Directory\shell\VaultLock")
                .map_err(|e| e.to_string())?;
            key.set_value("", &"Lock with VaultLock")
                .map_err(|e| e.to_string())?;
            key.set_value("Icon", &exe).map_err(|e| e.to_string())?;
            let (cmd, _) = classes
                .create_subkey(r"Directory\shell\VaultLock\command")
                .map_err(|e| e.to_string())?;
            cmd.set_value("", &format!("\"{exe}\" --lock \"%1\""))
                .map_err(|e| e.to_string())?;
            // .vaultlock files -> Unlock with VaultLock
            let (assoc, _) = classes.create_subkey(r".vaultlock").map_err(|e| e.to_string())?;
            assoc
                .set_value("", &"VaultLock.vault")
                .map_err(|e| e.to_string())?;
            let (key, _) = classes
                .create_subkey(r"VaultLock.vault\shell\open")
                .map_err(|e| e.to_string())?;
            key.set_value("", &"Unlock with VaultLock")
                .map_err(|e| e.to_string())?;
            key.set_value("Icon", &exe).map_err(|e| e.to_string())?;
            let (cmd, _) = classes
                .create_subkey(r"VaultLock.vault\shell\open\command")
                .map_err(|e| e.to_string())?;
            cmd.set_value("", &format!("\"{exe}\" --unlock \"%1\""))
                .map_err(|e| e.to_string())?;
        } else {
            let _ = classes.delete_subkey_all(r"*\shell\VaultLock");
            let _ = classes.delete_subkey_all(r"Directory\shell\VaultLock");
            let _ = classes.delete_subkey_all(r"VaultLock.vault");
            let _ = classes.delete_subkey_all(r".vaultlock");
        }
        Ok(enabled)
    }
}

#[cfg(not(target_os = "windows"))]
mod ctx_menu {
    pub fn supported() -> bool {
        false
    }

    pub fn status() -> bool {
        false
    }

    pub fn set(_enabled: bool) -> Result<bool, String> {
        Err("The right-click menu is only available on Windows.".to_string())
    }
}

#[tauri::command]
fn context_menu_supported() -> bool {
    ctx_menu::supported()
}

#[tauri::command]
fn context_menu_status() -> bool {
    ctx_menu::status()
}

#[tauri::command]
fn set_context_menu(enabled: bool) -> Result<bool, String> {
    ctx_menu::set(enabled)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(launch_target: Option<LaunchTarget>) {
    tauri::Builder::default()
        .manage(launch_target)
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(target) = parse_launch_target(&argv) {
                let _ = app.emit("vaultlock-open", target);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![
            pick_file,
            pick_folder,
            pick_vault,
            lock_item,
            unlock_item,
            get_recent,
            forget_recent,
            reveal_in_folder,
            get_launch_target,
            context_menu_supported,
            context_menu_status,
            set_context_menu
        ])
        .run(tauri::generate_context!())
        .expect("error while running VaultLock");
}
