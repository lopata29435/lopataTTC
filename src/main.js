// TrustTunnel GUI — frontend logic
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
    copyBtn.textContent = "Копировать";
    copyBtn.onclick = async (ev) => {
      ev.stopPropagation();
      try {
        await navigator.clipboard.writeText(String(message));
        copyBtn.textContent = "Скопировано ✓";
        setTimeout(() => (copyBtn.textContent = "Копировать"), 1400);
      } catch { /* ignore */ }
    };
    const closeBtn = document.createElement("button");
    closeBtn.className = "toast-btn";
    closeBtn.textContent = "Закрыть";
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
    root.innerHTML = `<div style="color: var(--text-dim); grid-column: 1/-1; text-align: center; padding: 40px;">
      Серверов пока нет. Добавь новый или импортируй <code>tt://</code>-ссылку.
    </div>`;
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
        <button class="btn primary" data-action="activate">Сделать активным</button>
        <button class="btn" data-action="edit">Изменить</button>
        <button class="btn danger" data-action="delete">Удалить</button>
      </div>
    `;
    card.querySelector(".profile-card-name").textContent = p.name;
    card.querySelector(".profile-card-host").textContent = addr;
    card.querySelector('[data-action="activate"]').onclick = async () => {
      await invoke("set_active_profile_id", { id: p.id });
      state.activeId = p.id;
      renderProfileList();
      renderActivePicker();
      toast(`Активный сервер: ${p.name}`, "success");
    };
    card.querySelector('[data-action="edit"]').onclick = () => openProfileModal(p);
    card.querySelector('[data-action="delete"]').onclick = async () => {
      if (!confirm(`Удалить «${p.name}»?`)) return;
      await invoke("delete_profile", { id: p.id });
      toast(`«${p.name}» удалён`, "success");
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
    value.textContent = "— нет серверов —";
    const empty = document.createElement("div");
    empty.className = "custom-select-empty";
    empty.textContent = "Добавь сервер на вкладке «Серверы»";
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
    } catch (e) { toast(`Ошибка отключения: ${e}`, "error"); }
  } else {
    if (!state.activeId) { toast("Сначала выбери сервер", "warn"); return; }
    try {
      await invoke("vpn_connect", { profileId: state.activeId });
    } catch (e) { toast(`Ошибка подключения: ${e}`, "error"); }
  }
});

function renderVpnState(s) {
  const btn = $("#connect-btn");
  const label = $("#connect-btn-label");
  const status = $("#status-line");
  btn.dataset.state = s.state;
  switch (s.state) {
    case "disconnected":
      label.textContent = "Подключиться";
      status.textContent = s.message || "Готово к подключению";
      $("#status-timer").textContent = "";
      break;
    case "connecting":
      label.textContent = "Подключение...";
      status.textContent = s.message || "Подключение...";
      $("#status-timer").textContent = "";
      break;
    case "connected":
      label.textContent = "Отключиться";
      status.textContent = `Подключено${s.profile_name ? " · " + s.profile_name : ""}`;
      break;
    case "error":
      label.textContent = "Повторить";
      status.textContent = s.message || "Ошибка";
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
  throw new Error("Не распознан формат. Поддерживается tt://... либо TOML.");
}

async function importFromClipboard() {
  try {
    const text = await readText();
    if (!text) { toast("Буфер пуст", "warn"); return; }
    const profile = await tryImportText(text);
    if (profile) {
      toast(`Импортирован: ${profile.name}`, "success");
      await refreshProfiles();
    }
  } catch (e) {
    toast(`Ошибка импорта: ${describeError(e)}`, "error");
  }
}

async function importFromFile() {
  try {
    const result = await openFileDialog([{ name: "TOML config", extensions: ["toml"] }]);
    if (!result) return;
    const path = typeof result === "string" ? result : (result.path || result[0]);
    if (!path) return;
    const profile = await invoke("import_toml_file", { path });
    toast(`Импортирован: ${profile.name}`, "success");
    await refreshProfiles();
  } catch (e) {
    toast(`Ошибка импорта: ${describeError(e)}`, "error");
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
    toast("Путь скопирован в буфер", "success");
  } catch (e) {
    toast(`Не удалось скопировать: ${describeError(e)}`, "error");
  }
});

// ===== profile modal =====
function openProfileModal(profile) {
  state.editingProfile = JSON.parse(JSON.stringify(profile));
  $("#profile-modal-title").textContent = state.editingProfile.name ? `Профиль · ${state.editingProfile.name}` : "Новый профиль";
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
    const btn = $("#f-password-toggle");
    if (btn) {
      btn.querySelector(".eye-open").style.display = "block";
      btn.querySelector(".eye-closed").style.display = "none";
    }
  }
  $("#profile-modal").hidden = false;
}

function closeProfileModal() {
  $("#profile-modal").hidden = true;
  state.editingProfile = null;
}

function setModalTab(name) {
  $$(".modal-tab").forEach(t => t.classList.toggle("active", t.dataset.modalTab === name));
  $$(".modal-section").forEach(s => s.classList.toggle("active", s.dataset.modalSection === name));
  // When switching to TOML view, render current form-state TOML if user hasn't typed anything custom yet
  if (name === "toml" && !$("#f-raw-toml").value.trim()) {
    invoke("profile_to_toml", { profile: collectFormProfile() }).then(t => {
      $("#f-raw-toml").value = t;
    });
  }
}

$$(".modal-tab").forEach(t => t.addEventListener("click", () => setModalTab(t.dataset.modalTab)));
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
  const show = input.type === "password";
  input.type = show ? "text" : "password";
  btn.querySelector(".eye-open").style.display = show ? "none" : "block";
  btn.querySelector(".eye-closed").style.display = show ? "block" : "none";
  btn.setAttribute("aria-label", show ? "Скрыть пароль" : "Показать пароль");
});

function updateExclusionsLabel() {
  const mode = $("#f-vpn-mode").value;
  if (mode === "selective") {
    $("#f-exclusions-label").textContent = "Сайты через VPN (по одному на строку)";
    $("#f-exclusions-hint").textContent = "только эти адреса пойдут через VPN, остальное — напрямую";
    $("#f-exclusions").placeholder = "youtube.com\n*.netflix.com";
  } else {
    $("#f-exclusions-label").textContent = "Исключения (по одному на строку)";
    $("#f-exclusions-hint").textContent = "эти адреса пойдут напрямую в обход VPN";
    $("#f-exclusions").placeholder = "*.ru\nya.ru";
  }
}
$("#f-vpn-mode")?.addEventListener("change", updateExclusionsLabel);

function collectFormProfile() {
  const p = state.editingProfile;
  p.name = $("#f-name").value.trim() || "Новый сервер";
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
    toast(`«${saved.name}» сохранён`, "success");
    closeProfileModal();
    await refreshProfiles();
  } catch (e) {
    toast(`Ошибка сохранения: ${e}`, "error");
  }
});

// ===== service =====
async function refreshServiceStatus() {
  try {
    const s = await invoke("service_status");
    const badge = $("#service-badge");
    if (s.installed && s.running) {
      badge.className = "badge ok";
      badge.textContent = "Установлена · работает";
    } else if (s.installed) {
      badge.className = "badge warn";
      badge.textContent = "Установлена · остановлена";
    } else {
      badge.className = "badge";
      badge.textContent = "Не установлена";
    }
  } catch (e) {
    console.warn(e);
  }
}

$("#service-install-btn").addEventListener("click", async () => {
  if (!state.activeId) { toast("Сначала выбери активный сервер", "warn"); return; }
  try {
    await invoke("service_install", { profileId: state.activeId });
    toast("Служба установлена (UAC подтверждён)", "success");
    setTimeout(refreshServiceStatus, 1500);
  } catch (e) {
    toast(`Ошибка: ${describeError(e)}`, "error");
  }
});
$("#service-uninstall-btn").addEventListener("click", async () => {
  try {
    await invoke("service_uninstall");
    toast("Служба удалена", "success");
    setTimeout(refreshServiceStatus, 1500);
  } catch (e) {
    toast(`Ошибка: ${describeError(e)}`, "error");
  }
});

async function refreshBinaryInfo() {
  try {
    const info = await invoke("binary_info");
    $("#binary-path-info").textContent = `${info.path}${info.exists ? "" : " · ⚠ не найден"}`;
  } catch (e) {
    $("#binary-path-info").textContent = String(e);
  }
  try {
    const dataDir = await invoke("app_data_dir");
    $("#data-path-info").textContent = dataDir;
  } catch (_) {}
}

async function refreshElevation() {
  try {
    const elevated = await invoke("is_elevated");
    $("#elevation-warning").hidden = elevated;
  } catch (_) {}
}

function renderUpdateStatus(s) {
  $("#update-current").textContent = s.current || "?";
  $("#update-latest").textContent = s.latest || "?";
  const badge = $("#update-badge");
  const installBtn = $("#install-update-btn");
  if (!s.platform_supported) {
    badge.className = "badge warn";
    badge.textContent = "Платформа не поддерживается";
    installBtn.hidden = true;
  } else if (s.update_available) {
    badge.className = "badge warn";
    badge.textContent = `Доступно обновление`;
    installBtn.hidden = false;
  } else if (s.latest) {
    badge.className = "badge ok";
    badge.textContent = "Актуальная версия";
    installBtn.hidden = true;
  } else {
    badge.className = "badge";
    badge.textContent = "Не удалось проверить";
    installBtn.hidden = true;
  }
}

async function refreshUpdateStatus() {
  $("#update-badge").className = "badge";
  $("#update-badge").textContent = "Проверка…";
  try {
    const s = await invoke("check_for_update");
    renderUpdateStatus(s);
  } catch (e) {
    $("#update-badge").className = "badge err";
    $("#update-badge").textContent = "Ошибка проверки";
    $("#update-current").textContent = "?";
    $("#update-latest").textContent = "?";
    toast(`Не удалось проверить обновления: ${describeError(e)}`, "error");
  }
}

$("#check-update-btn")?.addEventListener("click", refreshUpdateStatus);
$("#install-update-btn")?.addEventListener("click", async () => {
  $("#install-update-btn").disabled = true;
  $("#update-progress-line").hidden = false;
  $("#update-progress-text").textContent = "0%";
  try {
    const s = await invoke("install_update");
    renderUpdateStatus(s);
    toast(`Клиент обновлён до ${s.current}`, "success");
    await refreshBinaryInfo();
  } catch (e) {
    toast(`Ошибка установки обновления: ${describeError(e)}`, "error");
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
});

listen("update://status", evt => {
  renderUpdateStatus(evt.payload);
});

listen("update://installed", evt => {
  renderUpdateStatus(evt.payload);
  toast(`Клиент обновлён до ${evt.payload.current} в фоне`, "success");
  refreshBinaryInfo();
});

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

$("#restart-admin-btn")?.addEventListener("click", async () => {
  try {
    await invoke("restart_as_admin");
    toast("Запрос UAC отправлен — окно перезапустится", "success");
  } catch (e) {
    toast(`Не удалось перезапустить: ${describeError(e)}`, "error");
  }
});

$("#deeplink-register-btn")?.addEventListener("click", async () => {
  try {
    const msg = await invoke("register_deeplink_scheme");
    toast(msg, "success");
    $("#deeplink-badge").className = "badge ok";
    $("#deeplink-badge").textContent = "Зарегистрировано";
  } catch (e) {
    toast(`Регистрация tt:// не удалась: ${describeError(e)}`, "error");
  }
});

$("#deeplink-test-btn")?.addEventListener("click", async () => {
  try {
    const text = await readText();
    if (!text) {
      toast("Буфер пуст", "warn");
      return;
    }
    const preview = text.length > 800 ? text.slice(0, 800) + `\n…(ещё ${text.length - 800} символов)` : text;
    toast(`Буфер (${text.length} симв.):\n\n${preview}`, "info", { copyable: true, sticky: true });
  } catch (e) {
    toast(`Не удалось прочитать буфер: ${describeError(e)}`, "error");
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
        toast(`Импортирован deeplink: ${profile.name}`, "success");
        await refreshProfiles();
        setTab("profiles");
      } catch (e) {
        toast(`Не удалось импортировать ссылку: ${e}`, "error");
      }
    }
  }
});

// ===== bootstrap =====
(async function init() {
  await refreshProfiles();
  state.vpnState = await invoke("vpn_state");
  renderVpnState(state.vpnState);
  const logs = await invoke("vpn_logs");
  for (const l of logs) appendLog(l);
  await refreshServiceStatus();
  await refreshBinaryInfo();
  await refreshElevation();
  refreshUpdateStatus();
})();
