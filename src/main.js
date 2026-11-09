import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";

const $ = (id) => document.getElementById(id);

const state = {
  lockType: "file",
  lockPath: "",
  unlockPath: "",
  hoverZone: "",
};

function toast(text) {
  const el = $("toast");
  el.textContent = text;
  el.classList.remove("hidden");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => el.classList.add("hidden"), 3500);
}

function escapeHtml(s) {
  return String(s)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function fmtBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function fmtDate(unix) {
  return new Date(unix * 1000).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function setMsg(el, text, kind) {
  el.textContent = text;
  el.className = `msg ${kind || ""}`;
}

function showProgress(which, show) {
  $(`progress-${which}`).classList.toggle("hidden", !show);
  if (show) {
    $(`progress-${which}-fill`).style.width = "0%";
    $(`progress-${which}-text`).textContent = "Starting...";
  }
}

function updateProgress(event) {
  const p = event.payload || event;
  const total = Math.max(p.total || 0, 1);
  const pct = Math.min(100, Math.round(((p.done || 0) / total) * 100));
  for (const which of ["lock", "unlock"]) {
    if (!$("panel-" + which).classList.contains("hidden")) {
      $(`progress-${which}-fill`).style.width = `${pct}%`;
      $(`progress-${which}-text`).textContent = `${p.phase}: ${p.file} (${p.done}/${p.total})`;
    }
  }
}

// Tabs
function switchTab(tab) {
  $("tab-lock").classList.toggle("active", tab === "lock");
  $("tab-unlock").classList.toggle("active", tab === "unlock");
  $("panel-lock").classList.toggle("hidden", tab !== "lock");
  $("panel-unlock").classList.toggle("hidden", tab !== "unlock");
  $("tab-hint").textContent =
    tab === "lock"
      ? "Pick a file or folder, set a password, done."
      : "Pick a .vaultlock file and enter its password.";
}

$("tab-lock").addEventListener("click", () => switchTab("lock"));
$("tab-unlock").addEventListener("click", () => switchTab("unlock"));

// Lock type toggle
function setLockType(type) {
  state.lockType = type;
  $("type-file").classList.toggle("active", type === "file");
  $("type-folder").classList.toggle("active", type === "folder");
  $("drop-kind").textContent = type;
}
$("type-file").addEventListener("click", () => setLockType("file"));
$("type-folder").addEventListener("click", () => setLockType("folder"));

// Selection display
function setLockPath(path) {
  state.lockPath = path || "";
  $("selected-lock").classList.toggle("hidden", !path);
  $("selected-lock-path").textContent = path || "";
  if (path) setMsg($("msg-lock"), "");
}
function setUnlockPath(path) {
  state.unlockPath = path || "";
  $("selected-unlock").classList.toggle("hidden", !path);
  $("selected-unlock-path").textContent = path || "";
  if (path) setMsg($("msg-unlock"), "");
}

async function browseLock() {
  try {
    const picked =
      state.lockType === "folder" ? await invoke("pick_folder") : await invoke("pick_file");
    if (picked) setLockPath(picked);
  } catch (e) {
    setMsg($("msg-lock"), String(e), "err");
  }
}
async function browseUnlock() {
  try {
    const picked = await invoke("pick_vault");
    if (picked) setUnlockPath(picked);
  } catch (e) {
    setMsg($("msg-unlock"), String(e), "err");
  }
}

$("drop-lock").addEventListener("click", browseLock);
$("drop-unlock").addEventListener("click", browseUnlock);
$("drop-lock").addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") browseLock();
});
$("drop-unlock").addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") browseUnlock();
});
$("clear-lock").addEventListener("click", () => setLockPath(""));
$("clear-unlock").addEventListener("click", () => setUnlockPath(""));

// Drag and drop (Tauri webview event, paths come from the OS)
for (const [zoneId, zone] of [
  ["drop-lock", "lock"],
  ["drop-unlock", "unlock"],
]) {
  const el = $(zoneId);
  el.addEventListener("dragenter", (e) => {
    e.preventDefault();
    state.hoverZone = zone;
    el.classList.add("dragging");
  });
  el.addEventListener("dragover", (e) => {
    e.preventDefault();
    state.hoverZone = zone;
  });
  el.addEventListener("dragleave", () => {
    state.hoverZone = "";
    el.classList.remove("dragging");
  });
  el.addEventListener("drop", (e) => e.preventDefault());
}

getCurrentWebview()
  .onDragDropEvent((event) => {
    const payload = event.payload;
    if (payload.type !== "drop" || !payload.paths || !payload.paths.length) {
      $("drop-lock").classList.remove("dragging");
      $("drop-unlock").classList.remove("dragging");
      return;
    }
    const path = payload.paths[0];
    $("drop-lock").classList.remove("dragging");
    $("drop-unlock").classList.remove("dragging");
    if (state.hoverZone === "unlock" || path.toLowerCase().endsWith(".vaultlock")) {
      switchTab("unlock");
      setUnlockPath(path);
    } else {
      switchTab("lock");
      setLockPath(path);
    }
    state.hoverZone = "";
  })
  .catch(() => {});

// Show / hide password
for (const btn of document.querySelectorAll(".eye-btn")) {
  btn.addEventListener("click", () => {
    const input = $(btn.dataset.for);
    const show = input.type === "password";
    input.type = show ? "text" : "password";
    btn.textContent = show ? "Hide" : "Show";
  });
}

// Password strength
function strength(pw) {
  let score = 0;
  if (pw.length >= 8) score += 1;
  if (pw.length >= 12) score += 1;
  if (pw.length >= 16) score += 1;
  if (/[a-z]/.test(pw) && /[A-Z]/.test(pw)) score += 1;
  if (/\d/.test(pw)) score += 1;
  if (/[^A-Za-z0-9]/.test(pw)) score += 1;
  return Math.min(score, 5);
}

$("lock-password").addEventListener("input", (e) => {
  const pw = e.target.value;
  const s = strength(pw);
  const meter = $("lock-meter");
  meter.style.width = `${(s / 5) * 100}%`;
  const colors = ["#f87171", "#f87171", "#fbbf24", "#a3e635", "#34d399", "#34d399"];
  meter.style.background = colors[s];
  const labels = [
    "Too short. Use at least 8 characters.",
    "Weak. Add more characters.",
    "Okay, but longer is better.",
    "Good. A passphrase is even better.",
    "Strong.",
    "Excellent. Long and varied.",
  ];
  $("lock-strength").textContent = pw
    ? labels[s]
    : "Use a long passphrase you will remember. There is no password reset.";
});

// Lock action
$("btn-lock").addEventListener("click", async () => {
  const password = $("lock-password").value;
  const confirm = $("lock-confirm").value;
  if (!state.lockPath) {
    setMsg($("msg-lock"), "Pick a file or folder first.", "err");
    return;
  }
  if (password.length < 8) {
    setMsg($("msg-lock"), "Password must be at least 8 characters.", "err");
    return;
  }
  if (password !== confirm) {
    setMsg($("msg-lock"), "Passwords do not match.", "err");
    return;
  }
  const btn = $("btn-lock");
  btn.disabled = true;
  showProgress("lock", true);
  setMsg($("msg-lock"), "");
  try {
    const result = await invoke("lock_item", { source: state.lockPath, password });
    showProgress("lock", false);
    const warn = result.warning ? ` ${result.warning}` : "";
    setMsg(
      $("msg-lock"),
      `Locked. ${result.files} file(s), ${fmtBytes(result.total_bytes)} encrypted into ${result.vault_path}.${warn}`,
      result.warning ? "warn" : "ok"
    );
    toast("Locked. Original removed, vault created.");
    setLockPath("");
    $("lock-password").value = "";
    $("lock-confirm").value = "";
    $("lock-meter").style.width = "0%";
    await refreshVaults();
  } catch (e) {
    showProgress("lock", false);
    setMsg($("msg-lock"), String(e), "err");
  } finally {
    btn.disabled = false;
  }
});

// Unlock action
$("btn-unlock").addEventListener("click", async () => {
  const password = $("unlock-password").value;
  if (!state.unlockPath) {
    setMsg($("msg-unlock"), "Pick a .vaultlock file first.", "err");
    return;
  }
  if (!password) {
    setMsg($("msg-unlock"), "Enter the password for this vault.", "err");
    return;
  }
  const btn = $("btn-unlock");
  btn.disabled = true;
  showProgress("unlock", true);
  setMsg($("msg-unlock"), "");
  try {
    const result = await invoke("unlock_item", { vaultPath: state.unlockPath, password });
    showProgress("unlock", false);
    setMsg(
      $("msg-unlock"),
      `Unlocked. ${result.files} file(s) restored to ${result.restored_path}.`,
      "ok"
    );
    toast("Unlocked and restored.");
    setUnlockPath("");
    $("unlock-password").value = "";
    await refreshVaults();
  } catch (e) {
    showProgress("unlock", false);
    setMsg($("msg-unlock"), String(e), "err");
  } finally {
    btn.disabled = false;
  }
});

// Vault list
async function refreshVaults() {
  const list = $("vault-list");
  try {
    const entries = await invoke("get_recent");
    if (!entries.length) {
      list.innerHTML = `<div class="vault-empty">No vaults yet. Lock something and it will show up here.</div>`;
      return;
    }
    list.innerHTML = entries
      .map(
        (e) => `
      <div class="vault-row">
        <div class="vault-info">
          <div class="vault-name">${escapeHtml(e.name)}</div>
          <div class="vault-meta">${escapeHtml(e.kind)} - ${fmtBytes(e.size_bytes)} - locked ${fmtDate(e.locked_at_unix)}<br>${escapeHtml(e.vault_path)}</div>
        </div>
        <div class="vault-actions">
          <button class="mini-btn unlock" type="button" data-unlock="${escapeHtml(e.vault_path)}">Unlock</button>
          <button class="mini-btn" type="button" data-reveal="${escapeHtml(e.vault_path)}">Show</button>
          <button class="mini-btn" type="button" data-forget="${escapeHtml(e.vault_path)}">Forget</button>
        </div>
      </div>`
      )
      .join("");
    for (const b of list.querySelectorAll("[data-unlock]")) {
      b.addEventListener("click", () => {
        switchTab("unlock");
        setUnlockPath(b.dataset.unlock);
        $("unlock-password").focus();
      });
    }
    for (const b of list.querySelectorAll("[data-reveal]")) {
      b.addEventListener("click", () => invoke("reveal_in_folder", { path: b.dataset.reveal }).catch((e) => toast(String(e))));
    }
    for (const b of list.querySelectorAll("[data-forget]")) {
      b.addEventListener("click", async () => {
        await invoke("forget_recent", { vaultPath: b.dataset.forget });
        await refreshVaults();
      });
    }
  } catch (e) {
    list.innerHTML = `<div class="vault-empty">Could not load vaults: ${escapeHtml(String(e))}</div>`;
  }
}

$("refresh-vaults").addEventListener("click", refreshVaults);

// Progress events from Rust
listen("vault-progress", updateProgress).catch(() => {});

refreshVaults();
