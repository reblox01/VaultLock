import "@fontsource-variable/inter";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";

const $ = (id) => document.getElementById(id);

// Small password prompt mode: ?mode=access opens just the prompt
// window instead of the whole app (double-click a vault, or the
// right-click "Access" entry).
const params = new URLSearchParams(location.search);
const isPrompt = params.get("mode") === "access";
const promptSolo = params.get("solo") === "1";

// ---------- Icons: SF Symbols style, monoline, 1.75px stroke ----------
const ICONS = {
  lock: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>',
  "lock-open":
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 7.5-2"/></svg>',
  archive:
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M4 8l2-5h12l2 5"/><path d="M4 8h16v11a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V8z"/><path d="M10 12h4"/></svg>',
  gear: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3.2"/><path d="M12 2.8v2.4M12 18.8v2.4M2.8 12h2.4M18.8 12h2.4M5.5 5.5l1.7 1.7M16.8 16.8l1.7 1.7M18.5 5.5l-1.7 1.7M7.2 16.8l-1.7 1.7"/></svg>',
  folder:
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z"/></svg>',
  "folder-plus":
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z"/><path d="M12 11v6M9 14h6"/></svg>',
  doc: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M6 2h8l4 4v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z"/><path d="M14 2v4h4"/></svg>',
  search:
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="6"/><path d="M15.5 15.5L20 20"/></svg>',
  check:
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12l5 5 9-11"/></svg>',
  shield:
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 3v5c0 5-3.5 8-7 9-3.5-1-7-4-7-9V6l7-3z"/><path d="M9 12l2 2 4-4"/></svg>',
  key: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="15" r="4"/><path d="M11 12l8-8M16 4l2.5 2.5M13.5 6.5L16 9"/></svg>',
  trash:
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6.5 7l1 13h9l1-13"/></svg>',
};
for (const el of document.querySelectorAll("[data-icon]")) {
  el.innerHTML = ICONS[el.dataset.icon] || "";
}

// Windows uses Ctrl where macOS shows Cmd.
const isWin = navigator.platform.toLowerCase().includes("win");
for (const kbd of document.querySelectorAll("kbd.mod")) {
  kbd.textContent = kbd.textContent.replace("⌘", isWin ? "Ctrl+" : "⌘");
}

const state = {
  view: "lock",
  lockType: "file",
  lockPath: "",
  unlockPath: "",
  hoverZone: "",
  vaults: [],
  access: null,
};

const VIEWS = ["lock", "unlock", "vaults", "settings"];

function toast(text) {
  const el = $("toast");
  el.innerHTML = `${ICONS.check}<span></span>`;
  el.querySelector("span").textContent = text;
  el.classList.remove("hidden");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => el.classList.add("hidden"), 3000);
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
    $(`progress-${which}-fill`).style.width = `${pct}%`;
    $(`progress-${which}-text`).textContent = `${p.phase}: ${p.file} (${p.done}/${p.total})`;
  }
}

// ---------- Views ----------
function switchView(view) {
  if (!VIEWS.includes(view)) return;
  state.view = view;
  for (const v of VIEWS) {
    $(`panel-${v}`).classList.toggle("hidden", v !== view);
  }
  for (const btn of document.querySelectorAll(".nav-item")) {
    btn.classList.toggle("active", btn.dataset.view === view);
  }
}

$("tab-lock").addEventListener("click", () => switchView("lock"));
$("tab-unlock").addEventListener("click", () => switchView("unlock"));
$("nav-vaults").addEventListener("click", () => {
  switchView("vaults");
  refreshVaults();
});
$("nav-settings").addEventListener("click", () => switchView("settings"));

// ---------- Lock type ----------
function setLockType(type) {
  state.lockType = type;
  $("type-file").classList.toggle("active", type === "file");
  $("type-folder").classList.toggle("active", type === "folder");
  $("drop-kind").textContent = type;
}
$("type-file").addEventListener("click", () => setLockType("file"));
$("type-folder").addEventListener("click", () => setLockType("folder"));

// ---------- Selection ----------
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

// ---------- Drag and drop (Tauri webview event, paths come from the OS) ----------
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

try {
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
        switchView("unlock");
        setUnlockPath(path);
      } else {
        switchView("lock");
        setLockPath(path);
      }
      state.hoverZone = "";
    })
    .catch(() => {});
} catch {
  /* Not running inside Tauri (plain browser preview): drag and drop from the OS is unavailable. */
}

// ---------- Show / hide password ----------
for (const btn of document.querySelectorAll(".eye-btn")) {
  btn.addEventListener("click", () => {
    const input = $(btn.dataset.for);
    const show = input.type === "password";
    input.type = show ? "text" : "password";
    btn.textContent = show ? "Hide" : "Show";
  });
}

// ---------- Password strength: 5 segments ----------
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
  $("lock-meter").dataset.level = pw ? String(Math.max(s, 1)) : "";
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

// ---------- Lock action ----------
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
    $("lock-meter").dataset.level = "";
    await refreshVaults();
  } catch (e) {
    showProgress("lock", false);
    setMsg($("msg-lock"), String(e), "err");
  } finally {
    btn.disabled = false;
  }
});

// ---------- Unlock action ----------
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

// ---------- Vault list ----------
function renderVaults() {
  const list = $("vault-list");
  const q = ($("vault-search").value || "").toLowerCase();
  const entries = state.vaults.filter(
    (e) => !q || e.name.toLowerCase().includes(q) || e.vault_path.toLowerCase().includes(q)
  );
  if (!state.vaults.length) {
    list.innerHTML = `
      <div class="vault-empty">
        <div class="drop-art"><span class="ic">${ICONS.archive}</span></div>
        <h3>No vaults yet</h3>
        <p>Lock your first file or folder and it will show up here.</p>
        <button class="cta" type="button" id="empty-goto-lock">Lock something</button>
      </div>`;
    $("empty-goto-lock").addEventListener("click", () => switchView("lock"));
    return;
  }
  if (!entries.length) {
    list.innerHTML = `<div class="vault-empty"><h3>No matches</h3><p>Nothing in your vaults matches “${escapeHtml(q)}”.</p></div>`;
    return;
  }
  list.innerHTML = entries
    .map(
      (e) => `
      <div class="vault-row">
        <span class="vault-icon"><span class="ic">${ICONS.lock}</span></span>
        <div class="vault-info">
          <div class="vault-name">${escapeHtml(e.name)}</div>
          <div class="vault-meta">${escapeHtml(e.kind)} · ${fmtBytes(e.size_bytes)} · locked ${fmtDate(e.locked_at_unix)}</div>
          <div class="vault-meta">${escapeHtml(e.vault_path)}</div>
        </div>
        <div class="vault-actions">
          <button class="mini-btn accent" type="button" data-unlock="${escapeHtml(e.vault_path)}">Unlock</button>
          <button class="mini-btn" type="button" data-access="${escapeHtml(e.vault_path)}">Access</button>
          <button class="mini-btn" type="button" data-reveal="${escapeHtml(e.vault_path)}">Show</button>
          <button class="mini-btn" type="button" data-forget="${escapeHtml(e.vault_path)}">Forget</button>
        </div>
      </div>`
    )
    .join("");
  for (const b of list.querySelectorAll("[data-unlock]")) {
    b.addEventListener("click", () => {
      switchView("unlock");
      setUnlockPath(b.dataset.unlock);
      $("unlock-password").focus();
    });
  }
  for (const b of list.querySelectorAll("[data-access]")) {
    b.addEventListener("click", () => {
      invoke("open_access_prompt", { vaultPath: b.dataset.access }).catch((e) => toast(String(e)));
    });
  }
  for (const b of list.querySelectorAll("[data-reveal]")) {
    b.addEventListener("click", () =>
      invoke("reveal_in_folder", { path: b.dataset.reveal }).catch((e) => toast(String(e)))
    );
  }
  for (const b of list.querySelectorAll("[data-forget]")) {
    b.addEventListener("click", async () => {
      await invoke("forget_recent", { vaultPath: b.dataset.forget });
      await refreshVaults();
    });
  }
}

async function refreshVaults() {
  try {
    state.vaults = await invoke("get_recent");
  } catch (e) {
    state.vaults = [];
    $("vault-list").innerHTML = `<div class="vault-empty"><h3>Could not load vaults</h3><p>${escapeHtml(String(e))}</p></div>`;
    return;
  }
  renderVaults();
}

$("refresh-vaults").addEventListener("click", refreshVaults);
$("vault-search").addEventListener("input", renderVaults);

// ---------- Progress events from Rust ----------
listen("vault-progress", updateProgress).catch(() => {});

// ---------- Keyboard shortcuts ----------
document.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;
  if (mod && ["1", "2", "3", "4"].includes(e.key)) {
    e.preventDefault();
    switchView(VIEWS[Number(e.key) - 1]);
    return;
  }
  if (e.key === "Escape") {
    if (state.view === "lock") setLockPath("");
    if (state.view === "unlock") setUnlockPath("");
    return;
  }
  if (e.key === "Enter") {
    const a = document.activeElement;
    if (a === $("lock-password") || a === $("lock-confirm")) {
      e.preventDefault();
      $("btn-lock").click();
    } else if (a === $("unlock-password")) {
      e.preventDefault();
      $("btn-unlock").click();
    }
  }
});

// ---------- Right-click menu (Windows) ----------
function setCtxToggle(on) {
  const t = $("ctx-toggle");
  t.classList.toggle("on", on);
  t.setAttribute("aria-checked", String(on));
  const st = $("ctx-status");
  st.textContent = on ? "Added. Right-click any file or folder to lock it." : "Not added.";
  st.classList.toggle("on", on);
}

async function initContextMenu() {
  try {
    const supported = await invoke("context_menu_supported");
    if (!supported) {
      $("ctx-toggle").disabled = true;
      $("ctx-status").textContent = "The right-click menu is only available on Windows.";
      return;
    }
    const on = await invoke("context_menu_status");
    setCtxToggle(on);
    $("ctx-toggle").addEventListener("click", async () => {
      const next = !$("ctx-toggle").classList.contains("on");
      try {
        await invoke("set_context_menu", { enabled: next });
        setCtxToggle(next);
        toast(next ? "Right-click menu added." : "Right-click menu removed.");
      } catch (e) {
        toast(String(e));
      }
    });
    // First launch on Windows: register once, silently.
    if (!localStorage.getItem("vl-ctx-registered") && !on) {
      try {
        await invoke("set_context_menu", { enabled: true });
        setCtxToggle(true);
        localStorage.setItem("vl-ctx-registered", "1");
        toast("Right-click menu added. Right-click any file or folder to lock it.");
      } catch (e) {
        localStorage.setItem("vl-ctx-registered", "1");
      }
    }
  } catch (e) {
    $("ctx-status").textContent = String(e);
  }
}

// ---------- Launch from the right-click menu / second instance ----------
function applyLaunchTarget(t) {
  if (!t || !t.path) return;
  if (t.mode === "access") {
    invoke("open_access_prompt", { vaultPath: t.path }).catch((e) => toast(String(e)));
    return;
  }
  if (t.mode === "unlock") {
    switchView("unlock");
    setUnlockPath(t.path);
    $("unlock-password").focus();
  } else {
    switchView("lock");
    setLockPath(t.path);
    $("lock-password").focus();
  }
}

// ---------- Access banner (main window) ----------
async function updateAccessBanner() {
  try {
    const s = await invoke("get_access_session");
    state.access = s;
    $("access-banner").classList.toggle("hidden", !s);
    if (s) $("access-name").textContent = s.name;
  } catch {
    /* ignore */
  }
}

$("btn-open-access-folder").addEventListener("click", () => {
  if (state.access) {
    invoke("reveal_in_folder", { path: state.access.temp_path }).catch((e) => toast(String(e)));
  }
});

$("btn-lock-again").addEventListener("click", async () => {
  try {
    await invoke("close_access");
    toast("Locked again. Changes saved back into the vault.");
    await updateAccessBanner();
    await refreshVaults();
  } catch (e) {
    toast(String(e));
  }
});

// ---------- Small password prompt window ----------
const promptState = { target: "", tempPath: "", active: false };

async function lockAgainFromPrompt() {
  const btn = $("btn-prompt-lock");
  btn.disabled = true;
  setMsg($("msg-prompt-session"), "Locking again...");
  try {
    await invoke("close_access");
    promptState.active = false;
    if (promptSolo) {
      await invoke("exit_app").catch(() => {});
    } else {
      await invoke("close_prompt_window").catch(() => {});
    }
  } catch (e) {
    setMsg($("msg-prompt-session"), String(e), "err");
    btn.disabled = false;
  }
}

async function initPrompt() {
  document.querySelector(".app").classList.add("hidden");
  $("prompt").classList.remove("hidden");
  document.body.classList.add("prompt-mode");
  try {
    promptState.target = (await invoke("get_prompt_target")) || "";
  } catch {
    promptState.target = "";
  }
  const name = promptState.target.split(/[\\/]/).pop() || "vault";
  $("prompt-name").textContent = name.replace(/\.vaultlock$/i, "");
  $("prompt-password").focus();

  // Closing the window mid-access must re-lock first, never leave
  // decrypted files sitting in the temp folder.
  try {
    const win = getCurrentWindow();
    win.onCloseRequested(async (event) => {
      if (promptState.active) {
        event.preventDefault();
        await lockAgainFromPrompt();
      } else if (promptSolo) {
        await invoke("exit_app").catch(() => {});
      }
    });
  } catch {
    /* plain browser preview */
  }

  $("prompt-password").addEventListener("keydown", (e) => {
    if (e.key === "Enter") $("btn-access").click();
  });

  $("btn-access").addEventListener("click", async () => {
    const password = $("prompt-password").value;
    if (!promptState.target) {
      setMsg($("msg-prompt"), "No vault to open.", "err");
      return;
    }
    if (!password) {
      setMsg($("msg-prompt"), "Enter the password for this vault.", "err");
      return;
    }
    const btn = $("btn-access");
    btn.disabled = true;
    setMsg($("msg-prompt"), "Decrypting...");
    try {
      const info = await invoke("access_vault", { vaultPath: promptState.target, password });
      promptState.tempPath = info.temp_path;
      promptState.active = true;
      $("prompt-session-name").textContent = info.name;
      $("prompt-enter").classList.add("hidden");
      $("prompt-session").classList.remove("hidden");
      setMsg($("msg-prompt"), "");
      await invoke("reveal_in_folder", { path: info.temp_path }).catch(() => {});
    } catch (e) {
      setMsg($("msg-prompt"), String(e), "err");
    } finally {
      btn.disabled = false;
    }
  });

  $("btn-cancel-prompt").addEventListener("click", async () => {
    if (promptSolo) {
      await invoke("exit_app").catch(() => {});
    } else {
      await invoke("close_prompt_window").catch(() => {});
    }
  });

  $("btn-prompt-open").addEventListener("click", () => {
    if (promptState.tempPath) {
      invoke("reveal_in_folder", { path: promptState.tempPath }).catch((e) =>
        setMsg($("msg-prompt-session"), String(e), "err")
      );
    }
  });

  $("btn-prompt-lock").addEventListener("click", lockAgainFromPrompt);
}

async function initLaunchTarget() {
  try {
    applyLaunchTarget(await invoke("get_launch_target"));
  } catch (e) {
    /* no launch target */
  }
  listen("vaultlock-open", (e) => applyLaunchTarget(e.payload)).catch(() => {});
}

if (isPrompt) {
  initPrompt();
} else {
  refreshVaults();
  initContextMenu();
  initLaunchTarget();
  updateAccessBanner();
  window.addEventListener("focus", updateAccessBanner);
}
