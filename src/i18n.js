// Lightweight i18n.
//
// • Каждый язык — отдельный JSON в src/assets/locales/<lang>.json.
// • Ключ ↔ строка. Параметры в строках в фигурных скобках: {name}, {err}, ...
// • В HTML: data-i18n="ключ" (для textContent),
//           data-i18n-placeholder="ключ", data-i18n-title="ключ", data-i18n-aria-label="ключ".
// • В JS: import { t, setLocale } from './i18n.js'; t('toast.copied', { name: 'X' })
//
// Постоянное хранение: бэкенд (src-tauri/settings.json через команды get_settings /
// update_settings). localStorage оставлен как fallback на случай, если бэкенд
// недоступен в момент инициализации.

const TAURI_CORE = (window.__TAURI__ && window.__TAURI__.core) || null;
async function backendGet() {
  if (!TAURI_CORE) return null;
  try { return await TAURI_CORE.invoke("get_settings"); } catch { return null; }
}
async function backendSetLanguage(lang) {
  if (!TAURI_CORE) return;
  try { await TAURI_CORE.invoke("update_settings", { patch: { language: lang } }); } catch {}
}

const LOCALES_DIR = "./assets/locales";
const SUPPORTED = ["ru", "en"];
// Fallback dictionary (lookups for missing keys). English is the lingua franca.
const FALLBACK = "en";

const dict = {};      // current language strings
const fallbackDict = {}; // fallback language strings
let current = FALLBACK;

const listeners = new Set();

async function loadJson(lang) {
  const url = `${LOCALES_DIR}/${lang}.json?v=${Date.now()}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`locale ${lang}: HTTP ${res.status}`);
  return await res.json();
}

export function listLocales() {
  return [...SUPPORTED];
}

export function currentLocale() {
  return current;
}

export function format(template, params) {
  if (!template) return "";
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (_, k) => (params[k] ?? `{${k}}`));
}

export function t(key, params) {
  const tpl = dict[key] ?? fallbackDict[key] ?? key;
  return format(tpl, params);
}

export async function init() {
  // Always have the fallback language (English) ready so missing keys still resolve.
  try {
    Object.assign(fallbackDict, await loadJson(FALLBACK));
  } catch (e) {
    console.warn("i18n: failed to load fallback locale", e);
  }

  // Pick initial language:
  //   1. persisted backend setting (settings.json)
  //   2. legacy localStorage fallback
  //   3. system locale: ru-* → ru, everything else → en
  const backend = await backendGet();
  const stored = (backend && backend.language) || localStorage.getItem("ui.lang");
  const browserLang = (navigator.language || "en").toLowerCase();
  const isRussian = browserLang.startsWith("ru");
  const candidate = stored || (isRussian ? "ru" : "en");
  await applyLocale(candidate);
}

async function applyLocale(lang) {
  if (!SUPPORTED.includes(lang)) lang = FALLBACK;
  current = lang;
  const data = await loadJson(lang);
  for (const k of Object.keys(dict)) delete dict[k];
  Object.assign(dict, data);
  applyToDom();
  listeners.forEach(fn => { try { fn(lang); } catch (_) {} });
}

export async function setLocale(lang) {
  // Persist to both backend (durable) and localStorage (instant fallback).
  localStorage.setItem("ui.lang", lang);
  await backendSetLanguage(lang);
  await applyLocale(lang);
}

export function onChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function applyToDom(root) {
  root = root || document;
  for (const el of root.querySelectorAll("[data-i18n]")) {
    const key = el.getAttribute("data-i18n");
    el.textContent = t(key);
  }
  for (const el of root.querySelectorAll("[data-i18n-placeholder]")) {
    el.setAttribute("placeholder", t(el.getAttribute("data-i18n-placeholder")));
  }
  for (const el of root.querySelectorAll("[data-i18n-title]")) {
    el.setAttribute("title", t(el.getAttribute("data-i18n-title")));
  }
  for (const el of root.querySelectorAll("[data-i18n-aria-label]")) {
    el.setAttribute("aria-label", t(el.getAttribute("data-i18n-aria-label")));
  }
}
