use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Постоянные пользовательские настройки приложения. Любые поля опциональны,
/// неизвестные ключи во входном JSON игнорируются, так что схема может расти
/// без поломки старых файлов.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tab: Option<String>,
    /// Что мы знаем о последнем результате update-check (чтобы UI не висел в "Checking…").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_known_update: Option<serde_json::Value>,
}

pub struct SettingsStore {
    path: PathBuf,
    inner: Mutex<Settings>,
}

impl SettingsStore {
    pub fn open(app_data_dir: &Path) -> Result<Self> {
        let path = app_data_dir.join("settings.json");
        let inner = if path.exists() {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str::<Settings>(&text).unwrap_or_default()
        } else {
            Settings::default()
        };
        Ok(Self { path, inner: Mutex::new(inner) })
    }

    pub fn get(&self) -> Settings {
        self.inner.lock().unwrap().clone()
    }

    /// Применить частичное обновление: непустые (Some) поля patch перезаписывают
    /// существующие, None оставляет как есть. Возвращает обновлённое состояние.
    pub fn patch(&self, patch: Settings) -> Result<Settings> {
        let mut s = self.inner.lock().unwrap();
        if patch.language.is_some() { s.language = patch.language; }
        if patch.last_tab.is_some() { s.last_tab = patch.last_tab; }
        if patch.last_known_update.is_some() { s.last_known_update = patch.last_known_update; }
        let snapshot = s.clone();
        drop(s);
        self.write(&snapshot).context("write settings.json")?;
        Ok(snapshot)
    }

    fn write(&self, settings: &Settings) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let text = serde_json::to_string_pretty(settings)?;
        std::fs::write(&self.path, text)?;
        Ok(())
    }
}
