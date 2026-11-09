//! VaultLock crypto core.
//!
//! Vault format (.vaultlock), all little endian:
//!   magic "VL01" (4) | salt (16) | header nonce (12) | header len u32
//!   | header ciphertext | per file: ciphertext len u64 | ciphertext
//!
//! The header is JSON encrypted with AES-256-GCM under a key derived
//! from the password with Argon2id. Each file is encrypted the same
//! way with its own random nonce, and its SHA-256 is stored in the
//! header so an unlock verifies every byte before anything is restored.
//! No password is ever stored. No backdoor exists. A wrong password
//! fails authentication and restores nothing.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;
use zeroize::Zeroizing;

pub const MAGIC: &[u8; 4] = b"VL01";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: u64 = 16;
const MAX_HEADER_LEN: u32 = 10 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone)]
pub struct VaultEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub nonce: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct VaultHeader {
    pub version: u32,
    pub kind: String,
    pub name: String,
    pub created_unix: u64,
    pub empty_dirs: Vec<String>,
    pub entries: Vec<VaultEntry>,
}

#[derive(Serialize, Clone)]
pub struct ProgressEvent {
    pub phase: String,
    pub done: u64,
    pub total: u64,
    pub file: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct LockResult {
    pub vault_path: String,
    pub name: String,
    pub kind: String,
    pub files: u64,
    pub total_bytes: u64,
    pub warning: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct UnlockResult {
    pub restored_path: String,
    pub name: String,
    pub files: u64,
    pub total_bytes: u64,
}

/// Temporary access: where the decrypted files live while the vault
/// stays locked. The vault file itself is never touched by access.
#[derive(Serialize, Clone, Debug)]
pub struct AccessInfo {
    pub temp_path: String,
    pub name: String,
    pub kind: String,
    pub files: u64,
    pub total_bytes: u64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

fn derive_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let params = Params::new(65_536, 3, 1, Some(32)).map_err(|e| format!("KDF setup failed: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .map_err(|e| format!("Key derivation failed: {e}"))?;
    Ok(key)
}

fn encrypt(key: &[u8; 32], nonce_bytes: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("Cipher setup failed: {e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("Encryption failed: {e}"))
}

fn decrypt(key: &[u8; 32], nonce_bytes: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("Cipher setup failed: {e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "Wrong password or damaged vault.".to_string())
}

fn rel_posix(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

struct Collected {
    kind: String,
    name: String,
    files: Vec<(String, PathBuf, u64)>,
    empty_dirs: Vec<String>,
}

fn collect(source: &Path) -> Result<Collected, String> {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| "Could not read the name of that path.".to_string())?;
    if source.is_file() {
        let size = fs::metadata(source)
            .map_err(|e| format!("Could not read file: {e}"))?
            .len();
        return Ok(Collected {
            kind: "file".to_string(),
            name: name.clone(),
            files: vec![(name, source.to_path_buf(), size)],
            empty_dirs: vec![],
        });
    }
    if !source.is_dir() {
        return Err("That path is neither a file nor a folder.".to_string());
    }
    let mut files: Vec<(String, PathBuf, u64)> = Vec::new();
    let mut empty_dirs: Vec<String> = Vec::new();
    for entry in WalkDir::new(source).sort_by_file_name() {
        let entry = entry.map_err(|e| format!("Could not walk folder: {e}"))?;
        let path = entry.path();
        if path == source {
            continue;
        }
        if entry.file_type().is_dir() {
            empty_dirs.push(rel_posix(source, path));
        } else if entry.file_type().is_file() {
            let size = entry
                .metadata()
                .map_err(|e| format!("Could not read file: {e}"))?
                .len();
            files.push((rel_posix(source, path), path.to_path_buf(), size));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    empty_dirs.sort();
    Ok(Collected {
        kind: "folder".to_string(),
        name,
        files,
        empty_dirs,
    })
}

fn vault_output_path(source: &Path) -> PathBuf {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let file_name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());
    parent.join(format!("{file_name}.vaultlock"))
}

/// Lock a file or folder. Returns the vault that was created.
pub fn lock_path(
    source: &Path,
    password: &str,
    progress: &dyn Fn(ProgressEvent),
) -> Result<LockResult, String> {
    if password.chars().count() < 8 {
        return Err("Password must be at least 8 characters.".to_string());
    }
    if !source.exists() {
        return Err("That file or folder does not exist anymore.".to_string());
    }
    if source.extension().map(|e| e == "vaultlock").unwrap_or(false) {
        return Err("That is already a locked vault. Use Unlock instead.".to_string());
    }
    let collected = collect(source)?;
    let total = collected.files.len() as u64;
    let total_bytes: u64 = collected.files.iter().map(|f| f.2).sum();
    let vault_path = vault_output_path(source);
    if vault_path.exists() {
        return Err(format!(
            "A vault already exists next to it: {}. Move or rename it first.",
            vault_path.display()
        ));
    }

    let salt = random_bytes(SALT_LEN);
    let key = derive_key(password, &salt)?;
    let header_nonce = random_bytes(NONCE_LEN);

    // Encrypt every file first, keeping ciphertexts in memory only
    // one file at a time is not possible here because the header
    // (with hashes and nonces) is written before the bodies, so we
    // encrypt file by file into the temp vault and remember the
    // header entries as we go.
    let tmp_path = vault_path.with_extension("vaultlock.tmp");
    let mut entries: Vec<VaultEntry> = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    for (i, (rel, abs, size)) in collected.files.iter().enumerate() {
        progress(ProgressEvent {
            phase: "encrypting".to_string(),
            done: i as u64,
            total,
            file: rel.clone(),
        });
        let data = fs::read(abs).map_err(|e| format!("Could not read {rel}: {e}"))?;
        let sha256 = format!("{:x}", Sha256::digest(&data));
        let nonce = random_bytes(NONCE_LEN);
        let ct = encrypt(&key, &nonce, &data)?;
        body.extend_from_slice(&(ct.len() as u64).to_le_bytes());
        body.extend_from_slice(&ct);
        entries.push(VaultEntry {
            path: rel.clone(),
            size: *size,
            sha256,
            nonce: B64.encode(&nonce),
        });
    }

    let header = VaultHeader {
        version: 1,
        kind: collected.kind.clone(),
        name: collected.name.clone(),
        created_unix: now_unix(),
        empty_dirs: collected.empty_dirs.clone(),
        entries,
    };
    let header_json = serde_json::to_vec(&header).map_err(|e| format!("Header failed: {e}"))?;
    let header_ct = encrypt(&key, &header_nonce, &header_json)?;

    {
        let file = fs::File::create(&tmp_path)
            .map_err(|e| format!("Could not create vault file: {e}"))?;
        let mut out = BufWriter::new(file);
        out.write_all(MAGIC).map_err(|e| format!("Write failed: {e}"))?;
        out.write_all(&salt).map_err(|e| format!("Write failed: {e}"))?;
        out.write_all(&header_nonce)
            .map_err(|e| format!("Write failed: {e}"))?;
        out.write_all(&(header_ct.len() as u32).to_le_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;
        out.write_all(&header_ct)
            .map_err(|e| format!("Write failed: {e}"))?;
        out.write_all(&body).map_err(|e| format!("Write failed: {e}"))?;
        out.flush().map_err(|e| format!("Write failed: {e}"))?;
    }
    fs::rename(&tmp_path, &vault_path).map_err(|e| format!("Could not finish vault: {e}"))?;

    // Verify the vault opens with this password before deleting anything.
    verify_vault(&vault_path, password)?;

    progress(ProgressEvent {
        phase: "cleaning".to_string(),
        done: total,
        total,
        file: "Removing originals".to_string(),
    });
    let mut warning: Option<String> = None;
    let delete_result = if collected.kind == "file" {
        fs::remove_file(source)
    } else {
        fs::remove_dir_all(source)
    };
    if let Err(e) = delete_result {
        warning = Some(format!(
            "Vault is safe, but the original could not be deleted: {e}. Delete it by hand."
        ));
    }

    Ok(LockResult {
        vault_path: vault_path.to_string_lossy().into_owned(),
        name: collected.name,
        kind: collected.kind,
        files: total,
        total_bytes,
        warning,
    })
}

/// Read and decrypt only the header of a vault. Used to verify a
/// vault right after locking, and as the first step of unlocking.
fn read_header<R: Read>(reader: &mut R, password: &str) -> Result<(VaultHeader, Zeroizing<[u8; 32]>), String> {
    let mut magic = [0u8; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|_| "Not a VaultLock file.".to_string())?;
    if &magic != MAGIC {
        return Err("Not a VaultLock file.".to_string());
    }
    let mut salt = [0u8; SALT_LEN];
    let mut header_nonce = [0u8; NONCE_LEN];
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut salt)
        .map_err(|_| "Vault is truncated or damaged.".to_string())?;
    reader
        .read_exact(&mut header_nonce)
        .map_err(|_| "Vault is truncated or damaged.".to_string())?;
    reader
        .read_exact(&mut len_buf)
        .map_err(|_| "Vault is truncated or damaged.".to_string())?;
    let header_len = u32::from_le_bytes(len_buf);
    if header_len == 0 || header_len > MAX_HEADER_LEN {
        return Err("Vault header is damaged.".to_string());
    }
    let mut header_ct = vec![0u8; header_len as usize];
    reader
        .read_exact(&mut header_ct)
        .map_err(|_| "Vault is truncated or damaged.".to_string())?;
    let key = derive_key(password, &salt)?;
    let header_json = decrypt(&key, &header_nonce, &header_ct)?;
    let header: VaultHeader =
        serde_json::from_slice(&header_json).map_err(|_| "Vault header is damaged.".to_string())?;
    if header.version != 1 {
        return Err("This vault was made by a newer VaultLock.".to_string());
    }
    Ok((header, key))
}

fn verify_vault(path: &Path, password: &str) -> Result<VaultHeader, String> {
    let file = fs::File::open(path).map_err(|e| format!("Could not open vault: {e}"))?;
    let mut reader = BufReader::new(file);
    let (header, _) = read_header(&mut reader, password)?;
    Ok(header)
}

/// Decrypt every entry of an already-opened vault into `staging`,
/// verifying each file's SHA-256 as it goes. Shared by unlock and
/// temporary access. Returns the total plaintext bytes.
fn decrypt_entries<R: Read>(
    reader: &mut R,
    header: &VaultHeader,
    key: &[u8; 32],
    staging: &Path,
    progress: &dyn Fn(ProgressEvent),
) -> Result<u64, String> {
    for dir in &header.empty_dirs {
        fs::create_dir_all(staging.join(dir))
            .map_err(|e| format!("Could not stage restore: {e}"))?;
    }
    let total = header.entries.len() as u64;
    let mut total_bytes: u64 = 0;
    for (i, entry) in header.entries.iter().enumerate() {
        progress(ProgressEvent {
            phase: "decrypting".to_string(),
            done: i as u64,
            total,
            file: entry.path.clone(),
        });
        let mut len_buf = [0u8; 8];
        reader
            .read_exact(&mut len_buf)
            .map_err(|_| "Vault is truncated or damaged.".to_string())?;
        let ct_len = u64::from_le_bytes(len_buf);
        if ct_len != entry.size + TAG_LEN {
            return Err("Vault is damaged: a file size does not match.".to_string());
        }
        let mut ct = vec![0u8; ct_len as usize];
        reader
            .read_exact(&mut ct)
            .map_err(|_| "Vault is truncated or damaged.".to_string())?;
        let nonce = B64
            .decode(&entry.nonce)
            .map_err(|_| "Vault header is damaged.".to_string())?;
        let data = decrypt(key, &nonce, &ct)?;
        let sha256 = format!("{:x}", Sha256::digest(&data));
        if sha256 != entry.sha256 {
            return Err(format!("Integrity check failed for {}.", entry.path));
        }
        let out_path = staging.join(&entry.path);
        if let Some(dir) = out_path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Could not stage restore: {e}"))?;
        }
        fs::write(&out_path, &data).map_err(|e| format!("Could not write {}: {e}", entry.path))?;
        total_bytes += entry.size;
    }
    Ok(total_bytes)
}

/// Unlock a vault, restoring the original file or folder next to it.
pub fn unlock_path(
    vault: &Path,
    password: &str,
    progress: &dyn Fn(ProgressEvent),
) -> Result<UnlockResult, String> {
    if password.is_empty() {
        return Err("Enter the password for this vault.".to_string());
    }
    if !vault.exists() {
        return Err("That vault file does not exist anymore.".to_string());
    }
    let file = fs::File::open(vault).map_err(|e| format!("Could not open vault: {e}"))?;
    let mut reader = BufReader::new(file);
    let (header, key) = read_header(&mut reader, password)?;

    let parent = vault.parent().unwrap_or_else(|| Path::new("."));
    let dest = parent.join(&header.name);
    if dest.exists() {
        return Err(format!(
            "{} already exists next to the vault. Move or rename it first.",
            header.name
        ));
    }

    // Stage everything in a hidden folder next to the vault (same
    // filesystem, so the final move is a fast rename), verify every
    // file, and only then move it into place.
    let staging = parent.join(format!(
        ".vaultlock-restore-{}",
        hex_encode(&random_bytes(6))
    ));
    fs::create_dir_all(&staging).map_err(|e| format!("Could not stage restore: {e}"))?;
    let total_bytes = match decrypt_entries(&mut reader, &header, &key, &staging, progress) {
        Ok(n) => n,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    progress(ProgressEvent {
        phase: "restoring".to_string(),
        done: header.entries.len() as u64,
        total: header.entries.len() as u64,
        file: "Moving files into place".to_string(),
    });
    if header.kind == "file" {
        let staged_file = staging.join(&header.name);
        if let Err(e) = fs::rename(&staged_file, &dest) {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("Could not restore file: {e}"));
        }
    } else if let Err(e) = fs::rename(&staging, &dest) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("Could not restore folder: {e}"));
    }
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_file(vault);
    Ok(UnlockResult {
        restored_path: dest.to_string_lossy().into_owned(),
        name: header.name,
        files: header.entries.len() as u64,
        total_bytes,
    })
}

/// Decrypt a vault into a temp folder for temporary access. The vault
/// file is NOT touched: it stays locked, and the same password still
/// opens it afterwards. Use access_relock to save any changes back
/// into the vault and wipe the temp folder.
pub fn access_decrypt(
    vault: &Path,
    password: &str,
    progress: &dyn Fn(ProgressEvent),
) -> Result<AccessInfo, String> {
    if password.is_empty() {
        return Err("Enter the password for this vault.".to_string());
    }
    if !vault.exists() {
        return Err("That vault file does not exist anymore.".to_string());
    }
    let file = fs::File::open(vault).map_err(|e| format!("Could not open vault: {e}"))?;
    let mut reader = BufReader::new(file);
    let (header, key) = read_header(&mut reader, password)?;

    let temp_root = std::env::temp_dir().join(format!(
        "vaultlock-access-{}",
        hex_encode(&random_bytes(6))
    ));
    // Folder vault: the temp folder IS the folder. File vault: the
    // temp folder holds the single file. Either way temp_path is the
    // folder to open.
    let target = if header.kind == "file" {
        temp_root.clone()
    } else {
        temp_root.join(&header.name)
    };
    fs::create_dir_all(&target).map_err(|e| format!("Could not create temp folder: {e}"))?;
    let total_bytes = match decrypt_entries(&mut reader, &header, &key, &target, progress) {
        Ok(n) => n,
        Err(e) => {
            let _ = fs::remove_dir_all(&temp_root);
            return Err(e);
        }
    };
    Ok(AccessInfo {
        temp_path: target.to_string_lossy().into_owned(),
        name: header.name,
        kind: header.kind,
        files: header.entries.len() as u64,
        total_bytes,
    })
}

/// Re-lock the temp folder from an access session back into the vault:
/// encrypt whatever is in the temp folder now (edits included), verify
/// the new vault, move it over the original vault file, and wipe the
/// temp folder. The original vault is only replaced after the new one
/// verifies, so a failure leaves the old vault untouched.
pub fn access_relock(
    temp_path: &Path,
    name: &str,
    kind: &str,
    vault_path: &Path,
    password: &str,
    progress: &dyn Fn(ProgressEvent),
) -> Result<LockResult, String> {
    let source = if kind == "file" {
        temp_path.join(name)
    } else {
        temp_path.to_path_buf()
    };
    if !source.exists() {
        return Err(
            "The accessed files are gone. The vault itself was never touched and is still locked."
                .to_string(),
        );
    }
    let locked = lock_path(&source, password, progress)?;
    // lock_path created <parent>/<name>.vaultlock next to the source
    // and deleted the source after verifying. Move it over the
    // original vault.
    let new_vault = PathBuf::from(&locked.vault_path);
    if vault_path.exists() {
        fs::remove_file(vault_path)
            .map_err(|e| format!("Could not replace the old vault: {e}"))?;
    }
    if let Err(e) = fs::rename(&new_vault, vault_path) {
        return Err(format!(
            "Re-locked vault is at {}. Could not move it back: {e}",
            new_vault.display()
        ));
    }
    // Wipe whatever is left of the temp folder.
    let temp_root = if kind == "file" {
        temp_path.to_path_buf()
    } else {
        temp_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| temp_path.to_path_buf())
    };
    let _ = fs::remove_dir_all(&temp_root);
    Ok(LockResult {
        vault_path: vault_path.to_string_lossy().into_owned(),
        ..locked
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent(_: ProgressEvent) {}

    #[test]
    fn lock_and_unlock_folder_roundtrip() {
        let base = std::env::temp_dir().join(format!("vaultlock-test-{}", now_unix()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("My Photos");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), b"hello vault").unwrap();
        fs::write(src.join("sub").join("b.txt"), b"second file").unwrap();
        let locked = lock_path(&src, "correct horse battery staple", &silent).unwrap();
        assert!(!src.exists());
        assert!(Path::new(&locked.vault_path).exists());
        assert_eq!(locked.files, 2);
        let unlocked = unlock_path(Path::new(&locked.vault_path), "correct horse battery staple", &silent).unwrap();
        assert_eq!(fs::read_to_string(src.join("a.txt")).unwrap(), "hello vault");
        assert_eq!(fs::read_to_string(src.join("sub").join("b.txt")).unwrap(), "second file");
        assert!(!Path::new(&locked.vault_path).exists());
        assert_eq!(unlocked.files, 2);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn wrong_password_restores_nothing() {
        let base = std::env::temp_dir().join(format!("vaultlock-test-wrong-{}", now_unix()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("secret.txt");
        fs::create_dir_all(&base).unwrap();
        fs::write(&src, b"top secret").unwrap();
        let locked = lock_path(&src, "right password 1", &silent).unwrap();
        let err = unlock_path(Path::new(&locked.vault_path), "wrong password 1", &silent).unwrap_err();
        assert!(err.contains("Wrong password"));
        assert!(Path::new(&locked.vault_path).exists());
        assert!(!base.join("secret.txt").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn short_password_is_refused() {
        let base = std::env::temp_dir().join(format!("vaultlock-test-short-{}", now_unix()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let src = base.join("x.txt");
        fs::write(&src, b"x").unwrap();
        let err = lock_path(&src, "short", &silent).unwrap_err();
        assert!(err.contains("at least 8"));
        assert!(src.exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn vault_is_not_plaintext() {
        let base = std::env::temp_dir().join(format!("vaultlock-test-opaque-{}", now_unix()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let src = base.join("note.txt");
        fs::write(&src, b"my secret sentence 12345").unwrap();
        let locked = lock_path(&src, "another good password", &silent).unwrap();
        let raw = fs::read(&locked.vault_path).unwrap();
        let raw_str = String::from_utf8_lossy(&raw);
        assert!(!raw_str.contains("my secret sentence"));
        assert!(!raw_str.contains("note.txt"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn access_keeps_vault_locked_and_saves_edits_back() {
        let base = std::env::temp_dir().join(format!("vaultlock-test-access-{}", now_unix()));
        let _ = fs::remove_dir_all(&base);
        let src = base.join("Docs");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), b"original").unwrap();
        let locked = lock_path(&src, "access password 1", &silent).unwrap();
        let vault = PathBuf::from(&locked.vault_path);

        // Access: vault stays, contents readable in temp.
        let info = access_decrypt(&vault, "access password 1", &silent).unwrap();
        assert!(vault.exists());
        assert_eq!(fs::read_to_string(Path::new(&info.temp_path).join("a.txt")).unwrap(), "original");

        // Edit inside the temp folder, then lock again.
        fs::write(Path::new(&info.temp_path).join("a.txt"), b"edited").unwrap();
        assert!(Path::new(&info.temp_path).exists());
        let relocked = access_relock(
            Path::new(&info.temp_path),
            &info.name,
            &info.kind,
            &vault,
            "access password 1",
            &silent,
        )
        .unwrap();
        assert_eq!(relocked.vault_path, vault.to_string_lossy());
        assert!(!Path::new(&info.temp_path).exists());
        assert!(vault.exists());

        // Unlocking the re-locked vault shows the edit.
        let unlocked = unlock_path(&vault, "access password 1", &silent).unwrap();
        assert_eq!(fs::read_to_string(src.join("a.txt")).unwrap(), "edited");
        assert_eq!(unlocked.files, 1);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn access_wrong_password_fails_and_leaves_no_temp() {
        let base = std::env::temp_dir().join(format!("vaultlock-test-access-wrong-{}", now_unix()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let src = base.join("x.txt");
        fs::write(&src, b"x").unwrap();
        let locked = lock_path(&src, "right password 2", &silent).unwrap();
        let err = access_decrypt(Path::new(&locked.vault_path), "wrong password 2", &silent).unwrap_err();
        assert!(err.contains("Wrong password"));
        assert!(Path::new(&locked.vault_path).exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn access_single_file_roundtrip() {
        let base = std::env::temp_dir().join(format!("vaultlock-test-access-file-{}", now_unix()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let src = base.join("note.txt");
        fs::write(&src, b"file vault").unwrap();
        let locked = lock_path(&src, "file access pass", &silent).unwrap();
        let vault = PathBuf::from(&locked.vault_path);
        let info = access_decrypt(&vault, "file access pass", &silent).unwrap();
        assert_eq!(info.kind, "file");
        assert_eq!(
            fs::read_to_string(Path::new(&info.temp_path).join("note.txt")).unwrap(),
            "file vault"
        );
        access_relock(
            Path::new(&info.temp_path),
            &info.name,
            &info.kind,
            &vault,
            "file access pass",
            &silent,
        )
        .unwrap();
        assert!(vault.exists());
        assert!(!Path::new(&info.temp_path).exists());
        let _ = fs::remove_dir_all(&base);
    }
}
