# VaultLock

Lock any file or folder with a password. A fresh, cross-platform desktop
app built with Tauri v2 and Rust.

## Why a new one

The old repo this replaces, Albert-W/Folder-locker, is a C# WinForms app
that does **no encryption at all**. It only renames a folder to
`name.{2559a1f2-21d7-11d4-bdaf-00c04f60b9f0}`, a Windows Explorer trick,
stores the password as an MD5 hash in SQLite, and ships a universal
backdoor password that unlocks every folder. Anyone who finds the folder
can rename it back and read everything.

VaultLock fixes all of that:

| Old Folder-locker | VaultLock |
| --- | --- |
| No encryption, rename trick only | Real AES-256-GCM encryption, every file |
| Password stored as MD5 in SQLite | Password never stored anywhere |
| Universal backdoor password | No backdoor, no recovery key, no reset |
| Windows only, WinForms | Windows, macOS and Linux, Tauri + Rust |
| Original files left readable on disk | Originals removed after the vault verifies |

## Features

- Lock a single file or a whole folder (subfolders included).
- One `.vaultlock` file is created next to the original.
- Wrong password fails authentication and restores nothing.
- Every file is integrity checked with SHA-256 before restore.
- Restore stages to a hidden temp folder and only moves files into
  place after every file verifies, so a damaged vault never leaves a
  half restored folder behind.
- Drag and drop, browse dialogs, password strength meter, progress
  bar, and a list of your vaults with unlock, show in folder, forget.
- macOS-style interface: sidebar navigation, system palette in light
  and dark, SF-style icons, keyboard shortcuts (Ctrl/Cmd+1 to 4 to
  switch views, Enter to lock or unlock, Esc to clear).
- Temporary access: double-click a vault (or right-click it and pick
  Access) and a small password prompt opens instead of the whole app.
  Your files decrypt into a temp folder, the vault stays locked, and
  Lock again saves any changes back into the vault and wipes the temp
  folder. The password is never stored.
- 100% offline. Nothing leaves your machine.

## Right-click menu (Windows)

On first launch on Windows, VaultLock adds itself to the Explorer
right-click menu (HKCU, no admin needed):

- Right-click any file or folder: **Lock with VaultLock**. The app
  opens on the Lock view with that item already picked.
- Right-click any `.vaultlock` file: **Access with VaultLock**. A small
  password prompt opens, the files decrypt into a temp folder, and the
  vault stays locked. **Unlock with VaultLock** is also there for a
  permanent restore, which deletes the vault.

You can remove or re-add it any time from Settings. The menu works
through a single instance: if VaultLock is already running, the
right-click opens the existing window instead of a second copy.

## Security details

- Key derivation: Argon2id, 64 MiB memory, 3 iterations, 1 lane.
- Cipher: AES-256-GCM, random 12 byte nonce per file and for the header.
- The header (file names, sizes, hashes) is encrypted too, so a vault
  file on its own does not even reveal what is inside it.
- Keys are zeroed from memory after use.
- If you lose the password, the files are gone. That is the point.
  There is no reset, no recovery key, and no support backdoor.

## Install

### Windows (easiest)

Download `VaultLock_x64.msi` or `VaultLock_x64-setup.exe` from the
GitHub Releases page and install it. Releases are automatic: every
push to `main` that touches code bumps the patch version, tags it,
and GitHub Actions builds and publishes the release. To push code
without releasing, put `[skip release]` in the commit message.

### Build it yourself

Prerequisites: Rust (stable), Node.js 20+, and the Tauri system
libraries for your OS. See the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
git clone https://github.com/reblox01/VaultLock
cd VaultLock
npm install
npm run tauri build
```

The installer lands in `src-tauri/target/release/bundle/`.

Run it in development mode:

```bash
npm install
npm run tauri dev
```

The Tauri CLI is installed on demand by `npm run tauri` (it uses the
`@tauri-apps/cli` package, so no global install is needed).

## Usage

1. Open VaultLock, stay on Lock in the sidebar.
2. Pick File or Folder, then click the drop zone or drop something on it.
3. Set a password (at least 8 characters, a passphrase is best) and
   confirm it.
4. Press Lock it. The original is deleted only after the vault is
   written and verified to open with your password.
5. To unlock, go to Unlock in the sidebar, pick the `.vaultlock` file,
   enter the password, and press Unlock it. The original appears next
   to the vault, and the vault file is deleted.
6. To just look inside without unlocking, double-click the vault (or
   pick Access in My Vaults). A small password prompt opens, the files
   decrypt into a temp folder, and the vault stays locked. Press Lock
   again when you are done: any changes are saved back into the vault
   and the temp folder is wiped.

## Vault format

```
magic "VL01" (4) | salt (16) | header nonce (12) | header len u32
| header ciphertext | per file: ciphertext len u64 | ciphertext
```

The header is JSON (version, file or folder, original name, empty
folders, and per file: relative path, size, SHA-256, nonce), encrypted
with the password derived key. File bodies follow, each encrypted
with its own nonce.

## Roadmap

- Change password on an existing vault.
- Secure overwrite of originals before deletion.
- Auto-lock timer for folders left unlocked.

## License

MIT.
