// TrustTunnel GUI — frontend logic
import { init as initI18n, t, listLocales, currentLocale, setLocale, applyToDom, onChange as onLocaleChange } from "./i18n.js";

const TAURI = window.__TAURI__ || {};
const invoke = TAURI.core?.invoke || ((cmd) => Promise.reject(new Error(`Tauri not ready: ${cmd}`)));
const listen = TAURI.event?.listen || (async () => () => {});

// Direct-IPC plugin wrappers (we don't ship the JS plugin bundles).
async function readText() {
  return await invoke("plugin:clipboard-manager|read_text");
}

async function openFileDialog(filters) {
  return await invoke("plugin:dialog|open", {
    options: {
      multiple: false,
      directory: false,
      filters: filters || [],
    },
  });
}

const state = {
  profiles: [],
  activeId: null,
  vpnState: { state: "disconnected", profile_id: null, profile_name: null, message: null, started_at: null },
  editingProfile: null,
};

// ===== utility =====
function $(sel) { return document.querySelector(sel); }
function $$(sel) { return Array.from(document.querySelectorAll(sel)); }

function toast(message, kind = "info", opts = {}) {
  const wrap = $("#toasts");
  const el = document.createElement("div");
  el.className = `toast ${kind} ${kind === "error" ? "sticky" : ""}`;
  const text = document.createElement("div");
  text.className = "toast-text";
  text.textContent = String(message);
  el.appendChild(text);
  if (kind === "error" || opts.copyable) {
    const actions = document.createElement("div");
    actions.className = "toast-actions";
    const copyBtn = document.createElement("button");
    copyBtn.className = "toast-btn";
    copyBtn.textContent = t("toast.copy");
    copyBtn.onclick = async (ev) => {
      ev.stopPropagation();
      try {
        await navigator.clipboard.writeText(String(message));
        copyBtn.textContent = t("toast.copied");
        setTimeout(() => (copyBtn.textContent = t("toast.copy")), 1400);
      } catch { /* ignore */ }
    };
    const closeBtn = document.createElement("button");
    closeBtn.className = "toast-btn";
    closeBtn.textContent = t("toast.close");
    closeBtn.onclick = () => el.remove();
    actions.appendChild(copyBtn);
    actions.appendChild(closeBtn);
    el.appendChild(actions);
  }
  wrap.appendChild(el);
  if (kind !== "error" && !opts.sticky) {
    setTimeout(() => el.remove(), 4400);
  }
}

function describeError(e) {
  if (e == null) return "Неизвестная ошибка";
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message + (e.stack ? "\n" + e.stack : "");
  try { return JSON.stringify(e, null, 2); } catch { return String(e); }
}

function formatDuration(seconds) {
  if (!seconds || seconds < 0) return "";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

// ===== tabs =====
function setTab(name) {
  $$(".nav-item").forEach(b => b.classList.toggle("active", b.dataset.tab === name));
  $$(".tab").forEach(s => s.classList.toggle("active", s.dataset.tab === name));
}
$$(".nav-item").forEach(b => b.addEventListener("click", () => setTab(b.dataset.tab)));

// ===== profiles =====
async function refreshProfiles() {
  state.profiles = await invoke("list_profiles");
  state.activeId = await invoke("get_active_profile_id");
  renderProfileList();
  renderActivePicker();
}

function renderProfileList() {
  const root = $("#profile-list");
  root.innerHTML = "";
  if (state.profiles.length === 0) {
    const empty = document.createElement("div");
    empty.style.cssText = "color: var(--text-dim); grid-column: 1/-1; text-align: center; padding: 40px;";
    empty.textContent = t("profiles.empty");
    root.appendChild(empty);
    return;
  }
  for (const p of state.profiles) {
    const card = document.createElement("div");
    card.className = "profile-card" + (p.id === state.activeId ? " active" : "");
    const addr = (p.addresses && p.addresses[0]) || `${p.hostname}:443`;
    card.innerHTML = `
      <div class="profile-card-name"></div>
      <div class="profile-card-host"></div>
      <div class="profile-card-meta">
        <span class="badge">${p.upstream_protocol.toUpperCase()}</span>
        ${p.anti_dpi ? '<span class="badge warn">anti-DPI</span>' : ""}
        ${p.killswitch_enabled ? '<span class="badge">kill-switch</span>' : ""}
        <span class="badge">${p.vpn_mode}</span>
      </div>
      <div class="profile-card-actions">
        <button class="btn primary" data-action="activate"></button>
        <button class="btn" data-action="edit"></button>
        <button class="btn danger" data-action="delete"></button>
      </div>
    `;
    card.querySelector(".profile-card-name").textContent = p.name;
    card.querySelector(".profile-card-host").textContent = addr;
    const actBtn = card.querySelector('[data-action="activate"]');
    const editBtn = card.querySelector('[data-action="edit"]');
    const delBtn = card.querySelector('[data-action="delete"]');
    editBtn.textContent = t("profiles.card.edit");
    delBtn.textContent = t("profiles.card.delete");
    // "Make active" only makes sense when this card is NOT the active one.
    if (p.id === state.activeId) {
      actBtn.remove();
    } else {
      actBtn.textContent = t("profiles.card.activate");
      actBtn.onclick = async () => {
        await invoke("set_active_profile_id", { id: p.id });
        state.activeId = p.id;
        renderProfileList();
        renderActivePicker();
        toast(t("toast.activeChanged", { name: p.name }), "success");
      };
    }
    editBtn.onclick = () => openProfileModal(p);
    delBtn.onclick = async () => {
      if (!confirm(t("confirm.deleteProfile", { name: p.name }))) return;
      await invoke("delete_profile", { id: p.id });
      toast(t("toast.profileDeleted", { name: p.name }), "success");
      await refreshProfiles();
    };
    root.appendChild(card);
  }
}

function renderActivePicker() {
  const value = $("#active-profile-value");
  const menu = $("#active-profile-menu");
  const wrap = $("#active-profile-select");

  menu.innerHTML = "";
  if (state.profiles.length === 0) {
    value.textContent = t("connect.noServers");
    const empty = document.createElement("div");
    empty.className = "custom-select-empty";
    empty.textContent = t("connect.noServersHint");
    menu.appendChild(empty);
    wrap.classList.add("disabled");
    return;
  }
  wrap.classList.remove("disabled");

  const active = state.profiles.find(p => p.id === state.activeId);
  value.textContent = active ? active.name : state.profiles[0].name;

  for (const p of state.profiles) {
    const opt = document.createElement("div");
    opt.className = "custom-select-option" + (p.id === state.activeId ? " selected" : "");
    opt.dataset.id = p.id;
    const label = document.createElement("span");
    label.textContent = p.name;
    opt.appendChild(label);
    opt.addEventListener("click", async () => {
      closeActivePicker();
      if (p.id === state.activeId) return;
      await invoke("set_active_profile_id", { id: p.id });
      state.activeId = p.id;
      renderActivePicker();
      renderProfileList();
    });
    menu.appendChild(opt);
  }
}

function openActivePicker() {
  $("#active-profile-select").classList.add("open");
  $("#active-profile-menu").hidden = false;
}
function closeActivePicker() {
  $("#active-profile-select").classList.remove("open");
  $("#active-profile-menu").hidden = true;
}
$("#active-profile-trigger")?.addEventListener("click", (e) => {
  e.stopPropagation();
  const isOpen = $("#active-profile-select").classList.contains("open");
  if (isOpen) closeActivePicker(); else openActivePicker();
});
document.addEventListener("click", (e) => {
  if (!e.target.closest("#active-profile-select")) closeActivePicker();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeActivePicker();
});

// ===== connect =====
$("#connect-btn").addEventListener("click", async () => {
  const s = state.vpnState.state;
  if (s === "connected" || s === "connecting") {
    try {
      await invoke("vpn_disconnect");
    } catch (e) { toast(t("toast.disconnectErr", { err: describeError(e) }), "error"); }
  } else {
    if (!state.activeId) { toast(t("warn.selectServer"), "warn"); return; }
    try {
      await invoke("vpn_connect", { profileId: state.activeId });
    } catch (e) { toast(t("toast.connectErr", { err: describeError(e) }), "error"); }
  }
});

function renderVpnState(s) {
  const btn = $("#connect-btn");
  const label = $("#connect-btn-label");
  const status = $("#status-line");
  btn.dataset.state = s.state;
  switch (s.state) {
    case "disconnected":
      label.textContent = t("connect.btn.connect");
      status.textContent = s.message || t("connect.status.ready");
      $("#status-timer").textContent = "";
      break;
    case "connecting":
      label.textContent = t("connect.btn.connecting");
      status.textContent = s.message || t("connect.status.connecting");
      $("#status-timer").textContent = "";
      break;
    case "connected":
      label.textContent = t("connect.btn.disconnect");
      status.textContent = s.profile_name
        ? t("connect.status.connectedNamed", { name: s.profile_name })
        : t("connect.status.connected");
      break;
    case "error":
      label.textContent = t("connect.btn.retry");
      status.textContent = s.message || t("connect.status.error");
      $("#status-timer").textContent = "";
      break;
  }
}

setInterval(() => {
  if (state.vpnState.state === "connected" && state.vpnState.started_at) {
    const dur = Math.floor(Date.now() / 1000 - state.vpnState.started_at);
    $("#status-timer").textContent = formatDuration(dur);
  }
}, 1000);

// ===== logs =====
function appendLog(payload) {
  const view = $("#log-view");
  const line = document.createElement("div");
  line.className = `log-line ${payload.level}`;
  line.textContent = payload.line;
  view.appendChild(line);
  // cap at 1000 lines
  while (view.childElementCount > 1000) view.removeChild(view.firstChild);
  view.scrollTop = view.scrollHeight;
}

$("#logs-clear-btn").addEventListener("click", () => {
  $("#log-view").innerHTML = "";
});

// ===== import =====
async function tryImportText(text) {
  const trimmed = (text || "").trim();
  if (!trimmed) return null;
  if (trimmed.startsWith("tt://")) {
    return await invoke("import_deeplink", { uri: trimmed });
  }
  // try to find tt:// inside arbitrary text
  const url = await invoke("extract_deeplink_from_text", { text: trimmed });
  if (url) {
    return await invoke("import_deeplink", { uri: url });
  }
  // assume TOML
  if (trimmed.includes("hostname") || trimmed.includes("[endpoint]")) {
    return await invoke("import_toml_text", { text: trimmed, fallbackName: "Imported" });
  }
  throw new Error(t("import.unrecognizedFormat"));
}

async function importFromClipboard() {
  try {
    const text = await readText();
    if (!text) { toast(t("toast.clipboardEmpty"), "warn"); return; }
    const profile = await tryImportText(text);
    if (profile) {
      toast(t("toast.profileImported", { name: profile.name }), "success");
      await refreshProfiles();
    }
  } catch (e) {
    toast(t("toast.importErr", { err: describeError(e) }), "error");
  }
}

async function importFromFile() {
  try {
    const result = await openFileDialog([{ name: "TOML config", extensions: ["toml"] }]);
    if (!result) return;
    const path = typeof result === "string" ? result : (result.path || result[0]);
    if (!path) return;
    const profile = await invoke("import_toml_file", { path });
    toast(t("toast.profileImported", { name: profile.name }), "success");
    await refreshProfiles();
  } catch (e) {
    toast(t("toast.importErr", { err: describeError(e) }), "error");
  }
}

$("#paste-deeplink-btn").addEventListener("click", importFromClipboard);
$("#import-file-btn").addEventListener("click", importFromFile);
$("#profiles-import-btn").addEventListener("click", importFromClipboard);
$("#new-profile-btn").addEventListener("click", async () => {
  const p = await invoke("new_blank_profile");
  openProfileModal(p);
});

$("#copy-data-path-btn")?.addEventListener("click", async () => {
  const path = $("#data-path-info").textContent;
  try {
    await navigator.clipboard.writeText(path);
    toast(t("toast.pathCopied"), "success");
  } catch (e) {
    toast(t("toast.copyFailed", { err: describeError(e) }), "error");
  }
});

// ===== profile modal =====
function openProfileModal(profile) {
  state.editingProfile = JSON.parse(JSON.stringify(profile));
  $("#profile-modal-title").textContent = state.editingProfile.name
    ? t("modal.profile.titleNamed", { name: state.editingProfile.name })
    : t("modal.profile.titleNew");
  $("#f-name").value = profile.name || "";
  $("#f-hostname").value = profile.hostname || "";
  $("#f-address").value = (profile.addresses || [])[0] || "";
  $("#f-username").value = profile.username || "";
  $("#f-password").value = profile.password || "";
  $("#f-sni").value = profile.custom_sni || "";
  $("#f-protocol").value = profile.upstream_protocol || "http2";
  $("#f-vpn-mode").value = profile.vpn_mode || "general";
  $("#f-anti-dpi").checked = !!profile.anti_dpi;
  $("#f-killswitch").checked = !!profile.killswitch_enabled;
  $("#f-pq").checked = !!profile.post_quantum_group_enabled;
  $("#f-skip-verify").checked = !!profile.skip_verification;
  $("#f-dns").value = (profile.dns_upstreams || []).join("\n");
  $("#f-exclusions").value = (profile.exclusions || []).join("\n");
  $("#f-raw-toml").value = profile.raw_toml || "";
  setModalTab("form");
  updateExclusionsLabel();
  // Reset the password reveal state every time the modal opens.
  const pw = $("#f-password");
  if (pw) {
    pw.type = "password";
    const icon = $("#f-password-icon");
    if (icon) {
      icon.classList.add("icon-eye-open");
      icon.classList.remove("icon-eye-closed");
    }
  }
  $("#profile-modal").hidden = false;
}

function closeProfileModal() {
  $("#profile-modal").hidden = true;
  state.editingProfile = null;
}

function setModalTab(name) {
  $$(".modal-tab").forEach(tab => tab.classList.toggle("active", tab.dataset.modalTab === name));
  $$(".modal-section").forEach(s => s.classList.toggle("active", s.dataset.modalSection === name));
  // When switching to TOML view, render current form-state TOML if user hasn't typed anything custom yet
  if (name === "toml" && !$("#f-raw-toml").value.trim()) {
    invoke("profile_to_toml", { profile: collectFormProfile() }).then(toml => {
      $("#f-raw-toml").value = toml;
    });
  }
}

$$(".modal-tab").forEach(tab => tab.addEventListener("click", () => setModalTab(tab.dataset.modalTab)));
$("#profile-modal-close").addEventListener("click", closeProfileModal);
$("#profile-modal-cancel").addEventListener("click", closeProfileModal);
// Suppress label-click → focus-next behaviour: when the button lives inside a
// <label>, clicking it would otherwise also fire a synthetic click on the label
// which moves focus to the next form control in the row (SNI in this layout).
$("#f-password-toggle")?.addEventListener("mousedown", (e) => {
  e.preventDefault();
});
$("#f-password-toggle")?.addEventListener("click", (e) => {
  e.preventDefault();
  e.stopPropagation();
  const input = $("#f-password");
  const btn = $("#f-password-toggle");
  const icon = $("#f-password-icon");
  const show = input.type === "password";
  input.type = show ? "text" : "password";
  icon.classList.toggle("icon-eye-open", !show);
  icon.classList.toggle("icon-eye-closed", show);
  btn.setAttribute("aria-label", t(show ? "form.hidePasswordTitle" : "form.showPasswordTitle"));
});

function updateExclusionsLabel() {
  const mode = $("#f-vpn-mode").value;
  if (mode === "selective") {
    $("#f-exclusions-label").textContent = t("form.exclusions.selective");
    $("#f-exclusions-hint").textContent = t("form.exclusions.selective.hint");
    $("#f-exclusions").placeholder = "youtube.com\n*.netflix.com";
  } else {
    $("#f-exclusions-label").textContent = t("form.exclusions.general");
    $("#f-exclusions-hint").textContent = t("form.exclusions.general.hint");
    $("#f-exclusions").placeholder = "*.ru\nya.ru";
  }
}
$("#f-vpn-mode")?.addEventListener("change", updateExclusionsLabel);

function collectFormProfile() {
  const p = state.editingProfile;
  p.name = $("#f-name").value.trim() || t("profiles.new");
  p.hostname = $("#f-hostname").value.trim();
  const addr = $("#f-address").value.trim();
  p.addresses = addr ? [addr] : [];
  p.username = $("#f-username").value;
  p.password = $("#f-password").value;
  p.custom_sni = $("#f-sni").value.trim();
  p.upstream_protocol = $("#f-protocol").value;
  p.vpn_mode = $("#f-vpn-mode").value;
  p.anti_dpi = $("#f-anti-dpi").checked;
  p.killswitch_enabled = $("#f-killswitch").checked;
  p.post_quantum_group_enabled = $("#f-pq").checked;
  p.skip_verification = $("#f-skip-verify").checked;
  p.dns_upstreams = $("#f-dns").value.split(/\r?\n/).map(s => s.trim()).filter(Boolean);
  p.exclusions = $("#f-exclusions").value.split(/\r?\n/).map(s => s.trim()).filter(Boolean);
  return p;
}

$("#profile-modal-save").addEventListener("click", async () => {
  try {
    const profile = collectFormProfile();
    const rawToml = $("#f-raw-toml").value.trim();
    profile.raw_toml = rawToml ? rawToml : null;
    const saved = await invoke("save_profile", { profile });
    toast(t("toast.profileSaved", { name: saved.name }), "success");
    closeProfileModal();
    await refreshProfiles();
  } catch (e) {
    toast(t("toast.saveErr", { err: describeError(e) }), "error");
  }
});

// ===== service =====
async function refreshServiceStatus() {
  try {
    const platform = await invoke("platform_info");
    const badge = $("#service-badge");
    const installBtn = $("#service-install-btn");
    const uninstallBtn = $("#service-uninstall-btn");

    if (!platform.autostart_supported) {
      badge.className = "badge";
      badge.textContent = t("settings.autostart.statusUnsupported");
      installBtn.disabled = true;
      uninstallBtn.disabled = true;
      $("#autostart-desc").textContent = t("settings.autostart.bodyUnsupported");
      return;
    }

    const s = await invoke("service_status");
    if (s.installed && s.running) {
      badge.className = "badge ok";
      badge.textContent = t("settings.autostart.statusEnabled");
    } else if (s.installed) {
      badge.className = "badge warn";
      badge.textContent = t("settings.autostart.statusEnabledNotRunning");
    } else {
      badge.className = "badge";
      badge.textContent = t("settings.autostart.statusDisabled");
    }
  } catch (e) {
    console.warn(e);
  }
}

$("#service-install-btn").addEventListener("click", async () => {
  if (!state.activeId) { toast(t("warn.selectActive"), "warn"); return; }
  try {
    await invoke("service_install", { profileId: state.activeId });
    toast(t("toast.serviceInstalled"), "success");
    setTimeout(refreshServiceStatus, 1500);
  } catch (e) {
    toast(t("toast.error", { err: describeError(e) }), "error");
  }
});
$("#service-uninstall-btn").addEventListener("click", async () => {
  try {
    await invoke("service_uninstall");
    toast(t("toast.serviceUninstalled"), "success");
    setTimeout(refreshServiceStatus, 1500);
  } catch (e) {
    toast(t("toast.error", { err: describeError(e) }), "error");
  }
});

async function refreshBinaryInfo() {
  try {
    const info = await invoke("binary_info");
    const pathEl = $("#binary-path-info");
    pathEl.textContent = info.path;
    if (!info.exists) {
      const warn = document.createElement("span");
      warn.className = "missing-tag";
      const ic = document.createElement("span");
      ic.className = "icon icon-alert";
      warn.appendChild(ic);
      warn.appendChild(document.createTextNode(t("settings.paths.notFound")));
      pathEl.appendChild(document.createTextNode(" "));
      pathEl.appendChild(warn);
    }
  } catch (e) {
    $("#binary-path-info").textContent = String(e);
  }
  try {
    const dataDir = await invoke("app_data_dir");
    $("#data-path-info").textContent = dataDir;
  } catch (_) {}
}

async function setupUninstallCard() {
  try {
    const platform = await invoke("platform_info");
    // Windows users uninstall via "Apps & features"; the in-app button covers
    // the platforms that have no standard uninstall flow (AppImage, .dmg, …).
    $("#uninstall-card").hidden = !(platform.is_linux || platform.is_macos);
  } catch (_) {}
}

$("#uninstall-btn")?.addEventListener("click", async () => {
  if (!confirm(t("confirm.uninstall"))) return;
  const btn = $("#uninstall-btn");
  btn.disabled = true;
  try {
    await invoke("uninstall_app");
    toast(t("toast.uninstalled"), "success");
    // The backend exits the app shortly after.
  } catch (e) {
    btn.disabled = false;
    toast(t("toast.error", { err: describeError(e) }), "error");
  }
});

async function refreshElevation() {
  try {
    const platform = await invoke("platform_info");
    const elevated = await invoke("is_elevated");
    // The banner is Windows-only: on Linux/macOS the GUI deliberately runs
    // unprivileged and the system asks for the admin password at connect time.
    $("#elevation-warning").hidden = !platform.is_windows || elevated;
  } catch (_) {}
}

function renderUpdateStatus(s) {
  $("#update-current").textContent = s.current || t("settings.update.unknown");
  $("#update-latest").textContent = s.latest || t("settings.update.unknown");
  const badge = $("#update-badge");
  const installBtn = $("#install-update-btn");
  if (!s.platform_supported) {
    badge.className = "badge warn";
    badge.textContent = t("settings.update.platformUnsupported");
    installBtn.hidden = true;
  } else if (s.update_available) {
    badge.className = "badge warn";
    badge.textContent = t("settings.update.available");
    installBtn.hidden = false;
  } else if (s.latest) {
    badge.className = "badge ok";
    badge.textContent = t("settings.update.upToDate");
    installBtn.hidden = true;
  } else {
    badge.className = "badge";
    badge.textContent = t("settings.update.checkFailed");
    installBtn.hidden = true;
  }
}

function renderAppUpdateStatus(s) {
  $("#app-current").textContent = s.current || t("settings.update.unknown");
  $("#app-latest").textContent = s.latest || t("settings.update.unknown");
  const badge = $("#app-update-badge");
  const installBtn = $("#install-app-update-btn");
  const openBtn = $("#open-app-release-btn");
  if (s.update_available) {
    badge.className = "badge warn";
    badge.textContent = t("settings.update.available");
    installBtn.hidden = false;
    openBtn.hidden = false;
  } else if (s.latest) {
    badge.className = "badge ok";
    badge.textContent = t("settings.update.upToDate");
    installBtn.hidden = true;
    openBtn.hidden = true;
  } else {
    badge.className = "badge";
    badge.textContent = t("settings.update.checkFailed");
    installBtn.hidden = true;
    openBtn.hidden = true;
  }
}

async function refreshAllUpdates({ silent = false } = {}) {
  if (!silent) {
    for (const id of ["#update-badge", "#app-update-badge"]) {
      $(id).className = "badge";
      $(id).textContent = t("settings.update.checking");
    }
  }
  try {
    const both = await invoke("check_all_updates");
    renderUpdateStatus(both.client);
    renderAppUpdateStatus(both.app);
  } catch (e) {
    $("#update-badge").className = "badge err";
    $("#update-badge").textContent = t("settings.update.checkFailed");
    $("#app-update-badge").className = "badge err";
    $("#app-update-badge").textContent = t("settings.update.checkFailed");
    if (!silent) {
      toast(t("toast.updateCheckErr", { err: describeError(e) }), "error");
    }
  }
}

$("#check-update-btn")?.addEventListener("click", () => refreshAllUpdates({ silent: false }));
$("#open-app-release-btn")?.addEventListener("click", async () => {
  try {
    await invoke("open_app_release_page");
  } catch (e) {
    toast(t("toast.error", { err: describeError(e) }), "error");
  }
});

$("#install-app-update-btn")?.addEventListener("click", async () => {
  const btn = $("#install-app-update-btn");
  const badge = $("#app-update-badge");
  btn.disabled = true;
  const originalBadge = badge.textContent;
  badge.textContent = t("settings.update.downloading") + " 0%";
  try {
    await invoke("install_app_update");
    // app.restart() is called from Rust on success — we don't reach here normally.
  } catch (e) {
    badge.textContent = originalBadge;
    // tauri-plugin-updater will fail if signing isn't configured yet. In that
    // case point the user at the release page so they can install manually.
    toast(t("toast.updateInstallErr", { err: describeError(e) }), "error");
  } finally {
    btn.disabled = false;
  }
});

listen("app-update://progress", evt => {
  const { done, total } = evt.payload || {};
  const badge = $("#app-update-badge");
  if (total) {
    const pct = Math.floor((done / total) * 100);
    badge.textContent = t("settings.update.downloading") + ` ${pct}%`;
  } else {
    badge.textContent = t("settings.update.downloading") + ` ${formatBytes(done)}`;
  }
});
$("#install-update-btn")?.addEventListener("click", async () => {
  $("#install-update-btn").disabled = true;
  $("#update-progress-line").hidden = false;
  $("#update-progress-text").textContent = "0%";
  try {
    const s = await invoke("install_update");
    renderUpdateStatus(s);
    toast(t("toast.updateInstalled", { version: s.current }), "success");
    await refreshBinaryInfo();
  } catch (e) {
    toast(t("toast.updateInstallErr", { err: describeError(e) }), "error");
  } finally {
    $("#install-update-btn").disabled = false;
    $("#update-progress-line").hidden = true;
  }
});

listen("update://progress", evt => {
  const { done, total } = evt.payload || {};
  $("#update-progress-line").hidden = false;
  if (total) {
    const pct = Math.floor((done / total) * 100);
    $("#update-progress-text").textContent = `${pct}% (${formatBytes(done)} / ${formatBytes(total)})`;
  } else {
    $("#update-progress-text").textContent = formatBytes(done);
  }
  // Also drive the first-launch overlay if visible.
  if (!setupOverlay.hidden) setSetupProgress(done, total);
});

listen("update://status", evt => {
  const status = evt.payload || {};
  renderUpdateStatus(status);
  // First-launch: nothing installed yet → keep the setup overlay up.
  if (status.needs_initial_install) {
    showSetup();
  }
});

listen("update://app-status", evt => {
  renderAppUpdateStatus(evt.payload || {});
});

listen("update://installed", evt => {
  const status = evt.payload || {};
  renderUpdateStatus(status);
  if (!setupOverlay.hidden) {
    hideSetup();
    toast(t("toast.updateInstalled", { version: status.current }), "success");
  } else {
    toast(t("toast.updateInstalledBg", { version: status.current }), "success");
  }
  refreshBinaryInfo();
});

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

// ===== first-launch setup overlay =====
const setupOverlay = $("#setup-overlay");

function showSetup() {
  setupOverlay.hidden = false;
  setupOverlay.classList.remove("error");
  $("#setup-title").textContent = t("setup.title");
  $("#setup-body").textContent = t("setup.body");
  $("#setup-progress").hidden = true;
  $("#setup-progress-bar").style.width = "0%";
  $("#setup-actions").hidden = true;
  $("#setup-spinner").style.display = "block";
  // While we're setting up, disable Connect.
  $("#connect-btn").disabled = true;
}
function showSetupError() {
  setupOverlay.hidden = false;
  setupOverlay.classList.add("error");
  $("#setup-title").textContent = t("setup.error.title");
  $("#setup-body").textContent = t("setup.error.body", { size: "10" });
  $("#setup-progress").hidden = true;
  $("#setup-actions").hidden = false;
  $("#setup-spinner").style.display = "none";
  $("#connect-btn").disabled = true;
}
function hideSetup() {
  setupOverlay.hidden = true;
  $("#connect-btn").disabled = false;
}
function setSetupProgress(done, total) {
  $("#setup-progress").hidden = false;
  if (total) {
    const pct = Math.floor((done / total) * 100);
    $("#setup-progress-bar").style.width = `${pct}%`;
    $("#setup-body").textContent = t("setup.downloading", {
      percent: pct,
      done: formatBytes(done),
      total: formatBytes(total),
    });
  } else {
    $("#setup-body").textContent = formatBytes(done);
  }
}

$("#setup-retry-btn")?.addEventListener("click", async () => {
  showSetup();
  try {
    await invoke("install_update");
  } catch (e) {
    showSetupError();
  }
});

$("#restart-admin-btn")?.addEventListener("click", async () => {
  try {
    await invoke("restart_as_admin");
    toast(t("toast.uacSent"), "success");
  } catch (e) {
    toast(t("toast.restartFailed", { err: describeError(e) }), "error");
  }
});

$("#deeplink-register-btn")?.addEventListener("click", async () => {
  try {
    await invoke("register_deeplink_scheme");
    toast(t("toast.deeplinkRegOk"), "success");
    $("#deeplink-badge").className = "badge ok";
    $("#deeplink-badge").textContent = t("settings.deeplink.registered");
  } catch (e) {
    toast(t("toast.deeplinkRegErr", { err: describeError(e) }), "error");
  }
});

$("#deeplink-test-btn")?.addEventListener("click", async () => {
  try {
    const text = await readText();
    if (!text) {
      toast(t("toast.clipboardEmpty"), "warn");
      return;
    }
    let preview = text;
    if (text.length > 800) {
      preview = text.slice(0, 800) + t("toast.bufferMore", { more: text.length - 800 });
    }
    toast(
      t("toast.bufferContent", { len: text.length, preview }),
      "info",
      { copyable: true, sticky: true }
    );
  } catch (e) {
    toast(t("toast.bufferReadErr", { err: describeError(e) }), "error");
  }
});

// ===== event subscriptions =====
listen("vpn://state", evt => {
  state.vpnState = evt.payload;
  renderVpnState(state.vpnState);
});

listen("vpn://log", evt => {
  appendLog(evt.payload);
});

listen("deep-link://new-url", async evt => {
  const urls = evt.payload || [];
  for (const u of urls) {
    if (typeof u === "string" && u.startsWith("tt://")) {
      try {
        const profile = await invoke("import_deeplink", { uri: u });
        toast(t("toast.deeplinkImported", { name: profile.name }), "success");
        await refreshProfiles();
        setTab("profiles");
      } catch (e) {
        toast(t("toast.importErr", { err: describeError(e) }), "error");
      }
    }
  }
});

// ===== language switcher =====
async function loadLanguageNames() {
  // Each locale file has a "lang.name" key with its own native display name.
  const names = {};
  for (const code of listLocales()) {
    try {
      const res = await fetch(`./assets/locales/${code}.json`);
      const data = await res.json();
      names[code] = data["lang.name"] || code;
    } catch {
      names[code] = code;
    }
  }
  return names;
}

function setupLanguageSwitcher(names) {
  const wrap = $("#language-select");
  const menu = $("#language-menu");
  const value = $("#language-value");
  const trigger = $("#language-trigger");

  function rebuild() {
    menu.innerHTML = "";
    value.textContent = names[currentLocale()] || currentLocale();
    for (const code of listLocales()) {
      const opt = document.createElement("div");
      opt.className = "custom-select-option" + (code === currentLocale() ? " selected" : "");
      const span = document.createElement("span");
      span.textContent = names[code] || code;
      opt.appendChild(span);
      opt.addEventListener("click", async () => {
        wrap.classList.remove("open");
        menu.hidden = true;
        if (code === currentLocale()) return;
        await setLocale(code);
        toast(t("toast.langChanged"), "success");
      });
      menu.appendChild(opt);
    }
  }
  rebuild();

  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    const open = wrap.classList.contains("open");
    if (open) { wrap.classList.remove("open"); menu.hidden = true; }
    else { wrap.classList.add("open"); menu.hidden = false; }
  });
  document.addEventListener("click", (e) => {
    if (!e.target.closest("#language-select")) {
      wrap.classList.remove("open"); menu.hidden = true;
    }
  });

  onLocaleChange(() => rebuild());
}

// ===== bootstrap =====
(async function bootstrap() {
  try {
    await initI18n();
  } catch (e) {
    // i18n must never take down the rest of the app. Fall through with hard-coded
    // defaults already present in the HTML.
    console.error("i18n init failed:", e);
  }
  // Re-render dynamic state whenever the language changes.
  onLocaleChange(() => {
    renderActivePicker();
    renderProfileList();
    renderVpnState(state.vpnState);
    refreshServiceStatus();
    refreshBinaryInfo();
    if (!$("#profile-modal").hidden) {
      // Re-render the open modal title and exclusions label.
      updateExclusionsLabel();
      const editing = state.editingProfile;
      if (editing) {
        $("#profile-modal-title").textContent = editing.name
          ? t("modal.profile.titleNamed", { name: editing.name })
          : t("modal.profile.titleNew");
      }
    }
  });

  try {
    const langNames = await loadLanguageNames();
    setupLanguageSwitcher(langNames);
  } catch (e) {
    console.error("language switcher setup failed:", e);
  }

  await refreshProfiles();
  state.vpnState = await invoke("vpn_state");
  renderVpnState(state.vpnState);
  const logs = await invoke("vpn_logs");
  for (const l of logs) appendLog(l);
  await refreshServiceStatus();
  await refreshBinaryInfo();
  await refreshElevation();
  await setupUninstallCard();

  // Update-status flow:
  //  1. If there's a cached status from a previous launch, render it now.
  //  2. The Rust-side `auto_update_check` task is already running in the
  //     background; it emits `update://status` once the network fetch completes
  //     and the listener at the top of this file picks it up.
  //  3. Safety net: if no event arrives within 15 s, mark the badge as
  //     "Could not check" so the user isn't stuck on a spinner.
  //
  // We intentionally do NOT call `refreshUpdateStatus()` here — that would
  // fire a second `check_for_update` HTTP request in parallel with the
  // auto-update task and, on Linux first-launch, race the ~10 MB client
  // download (which can make the badge appear to hang).
  let needsSetup = false;
  try {
    const info = await invoke("binary_info");
    if (info && info.exists === false) {
      const cachedForSetup = await invoke("cached_update_status");
      needsSetup = !(cachedForSetup && cachedForSetup.installed_path);
    }
    const cached = await invoke("cached_update_status");
    if (cached) renderUpdateStatus(cached);
  } catch (_) {}

  // Render the GUI's own version immediately (no network needed for "current"),
  // and the cached "latest" comparison if we have one from a previous launch.
  try {
    const current = await invoke("app_version");
    $("#app-current").textContent = current;
  } catch (_) {}
  try {
    const cachedApp = await invoke("cached_app_update_status");
    if (cachedApp) renderAppUpdateStatus(cachedApp);
  } catch (_) {}

  if (needsSetup) showSetup();

  setTimeout(() => {
    for (const id of ["#update-badge", "#app-update-badge"]) {
      const badge = $(id);
      if (badge && badge.textContent === t("settings.update.checking")) {
        badge.className = "badge err";
        badge.textContent = t("settings.update.checkFailed");
      }
    }
  }, 15000);

  // If after 12 seconds we still haven't seen the binary, show the error
  // overlay with a retry button. The auto-update task will still keep running
  // and surface its eventual outcome via update://installed.
  if (needsSetup) {
    setTimeout(async () => {
      try {
        const info = await invoke("binary_info");
        if (!info.exists && !setupOverlay.hidden) {
          showSetupError();
        }
      } catch (_) {}
    }, 12000);
  }

})();
