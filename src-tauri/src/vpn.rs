use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;

use crate::profiles::Profile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatePayload {
    pub state: ConnectionState,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub message: Option<String>,
    pub started_at: Option<i64>, // unix seconds
}

#[derive(Debug, Clone, Serialize)]
pub struct LogPayload {
    pub line: String,
    pub level: String, // "info" | "warn" | "error"
}

const MAX_LOG_LINES: usize = 500;

pub struct VpnService {
    state: Arc<Mutex<StatePayload>>,
    log_buffer: Arc<Mutex<VecDeque<LogPayload>>>,
    child: Arc<AsyncMutex<Option<Child>>>,
    binary_path: Arc<Mutex<PathBuf>>,
    config_path: PathBuf,
}

impl VpnService {
    pub fn new(binary_path: Arc<Mutex<PathBuf>>, config_path: PathBuf) -> Self {
        Self {
            state: Arc::new(Mutex::new(StatePayload {
                state: ConnectionState::Disconnected,
                profile_id: None,
                profile_name: None,
                message: None,
                started_at: None,
            })),
            log_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES))),
            child: Arc::new(AsyncMutex::new(None)),
            binary_path,
            config_path,
        }
    }

    pub fn state(&self) -> StatePayload {
        self.state.lock().unwrap().clone()
    }

    pub fn logs(&self) -> Vec<LogPayload> {
        self.log_buffer.lock().unwrap().iter().cloned().collect()
    }

    pub fn binary_path(&self) -> PathBuf {
        self.binary_path.lock().unwrap().clone()
    }

    pub fn set_binary_path(&self, new_path: PathBuf) {
        *self.binary_path.lock().unwrap() = new_path;
    }

    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    fn set_state(&self, app: &AppHandle, new: StatePayload) {
        {
            let mut s = self.state.lock().unwrap();
            *s = new.clone();
        }
        let _ = app.emit("vpn://state", &new);
    }

    fn push_log(&self, app: &AppHandle, line: String, level: &str) {
        let payload = LogPayload { line, level: level.into() };
        {
            let mut buf = self.log_buffer.lock().unwrap();
            if buf.len() >= MAX_LOG_LINES {
                buf.pop_front();
            }
            buf.push_back(payload.clone());
        }
        let _ = app.emit("vpn://log", &payload);
    }

    pub async fn connect(&self, app: AppHandle, profile: Profile) -> Result<()> {
        // VPN child needs admin to create the WinTUN adapter and WFP firewall objects.
        // We require the GUI itself to be elevated so the child inherits admin.
        #[cfg(windows)]
        if !crate::elevate::is_elevated() {
            anyhow::bail!(
                "Для подключения нужны права администратора. Нажми «Перезапустить от админа» в боковой панели."
            );
        }

        // shut down any existing process first
        self.disconnect_internal().await.ok();

        // write config file
        let toml_text = profile.to_client_toml();
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&self.config_path, toml_text)
            .with_context(|| format!("write config to {}", self.config_path.display()))?;

        self.set_state(&app, StatePayload {
            state: ConnectionState::Connecting,
            profile_id: Some(profile.id.clone()),
            profile_name: Some(profile.name.clone()),
            message: Some(format!("Подключение к {}...", profile.hostname)),
            started_at: None,
        });
        self.push_log(&app, format!("=== Запуск клиента: профиль \"{}\" ({}) ===", profile.name, profile.hostname), "info");

        let binary_path = self.binary_path();
        let mut cmd = Command::new(&binary_path);
        cmd.arg("--config").arg(&self.config_path);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            anyhow!("Не удалось запустить trusttunnel_client.exe: {} (путь: {})", e, binary_path.display())
        })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        {
            let mut slot = self.child.lock().await;
            *slot = Some(child);
        }

        let state = self.state.clone();
        let log_buffer = self.log_buffer.clone();
        let app_clone = app.clone();
        let profile_id = profile.id.clone();
        let profile_name = profile.name.clone();

        if let Some(stdout) = stdout {
            let app2 = app.clone();
            let state2 = state.clone();
            let log_buffer2 = log_buffer.clone();
            let profile_id2 = profile_id.clone();
            let profile_name2 = profile_name.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    handle_log_line(&app2, &state2, &log_buffer2, &profile_id2, &profile_name2, &line, "info");
                }
            });
        }
        if let Some(stderr) = stderr {
            let app2 = app.clone();
            let state2 = state.clone();
            let log_buffer2 = log_buffer.clone();
            let profile_id2 = profile_id.clone();
            let profile_name2 = profile_name.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    handle_log_line(&app2, &state2, &log_buffer2, &profile_id2, &profile_name2, &line, "warn");
                }
            });
        }

        // Watcher: if process exits, transition to Disconnected/Error
        let child_slot = self.child.clone();
        let state_for_watcher = self.state.clone();
        let app_for_watcher = app_clone.clone();
        let profile_id_w = profile_id.clone();
        let profile_name_w = profile_name.clone();
        tokio::spawn(async move {
            // Loop until process is taken away (disconnect) or it exits naturally
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                let mut guard = child_slot.lock().await;
                if let Some(child) = guard.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            drop(guard);
                            let was_connected = {
                                let s = state_for_watcher.lock().unwrap();
                                s.state == ConnectionState::Connected
                            };
                            let new = StatePayload {
                                state: if status.success() || was_connected {
                                    ConnectionState::Disconnected
                                } else {
                                    ConnectionState::Error
                                },
                                profile_id: Some(profile_id_w.clone()),
                                profile_name: Some(profile_name_w.clone()),
                                message: Some(format!("Клиент завершился (код {})", status.code().unwrap_or(-1))),
                                started_at: None,
                            };
                            {
                                let mut s = state_for_watcher.lock().unwrap();
                                *s = new.clone();
                            }
                            let _ = app_for_watcher.emit("vpn://state", &new);
                            break;
                        }
                        Ok(None) => continue,
                        Err(_) => break,
                    }
                } else {
                    // disconnect() emptied the slot
                    break;
                }
            }
        });

        Ok(())
    }

    pub async fn disconnect(&self, app: AppHandle) -> Result<()> {
        self.push_log(&app, "=== Отключение ===".into(), "info");
        let prev_profile = {
            let s = self.state.lock().unwrap();
            (s.profile_id.clone(), s.profile_name.clone())
        };
        self.disconnect_internal().await?;
        let new = StatePayload {
            state: ConnectionState::Disconnected,
            profile_id: prev_profile.0,
            profile_name: prev_profile.1,
            message: Some("Отключено".into()),
            started_at: None,
        };
        self.set_state(&app, new);
        Ok(())
    }

    async fn disconnect_internal(&self) -> Result<()> {
        let mut slot = self.child.lock().await;
        if let Some(mut child) = slot.take() {
            #[cfg(windows)]
            {
                use windows::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
                if let Some(pid) = child.id() {
                    unsafe {
                        let _ = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid);
                    }
                    // small grace period
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(1500),
                        child.wait(),
                    ).await;
                }
            }
            // Force kill if still alive
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }
}

fn handle_log_line(
    app: &AppHandle,
    state: &Arc<Mutex<StatePayload>>,
    log_buffer: &Arc<Mutex<VecDeque<LogPayload>>>,
    profile_id: &str,
    profile_name: &str,
    line: &str,
    default_level: &str,
) {
    if line.trim().is_empty() {
        return;
    }
    let lower = line.to_ascii_lowercase();
    let level = if lower.contains("error") || lower.contains(" err ") || lower.contains("failed") || lower.contains("fatal") {
        "error"
    } else if lower.contains("warn") {
        "warn"
    } else {
        default_level
    };

    {
        let payload = LogPayload { line: line.to_string(), level: level.to_string() };
        let mut buf = log_buffer.lock().unwrap();
        if buf.len() >= MAX_LOG_LINES {
            buf.pop_front();
        }
        buf.push_back(payload.clone());
        let _ = app.emit("vpn://log", &payload);
    }

    let mut transition: Option<StatePayload> = None;
    {
        let cur = state.lock().unwrap().clone();
        // Connection-up heuristics: "connected", "established", "tunnel ready", "session opened"
        let up_markers = ["connected to", "tunnel established", "tunnel ready", "session established", "successfully connected", "vpn active"];
        let err_markers = ["authentication failed", "connection failed", "unable to connect", "fatal", "could not start"];

        if cur.state == ConnectionState::Connecting && up_markers.iter().any(|m| lower.contains(m)) {
            transition = Some(StatePayload {
                state: ConnectionState::Connected,
                profile_id: Some(profile_id.into()),
                profile_name: Some(profile_name.into()),
                message: Some("Соединение установлено".into()),
                started_at: Some(chrono::Utc::now().timestamp()),
            });
        } else if err_markers.iter().any(|m| lower.contains(m)) {
            transition = Some(StatePayload {
                state: ConnectionState::Error,
                profile_id: Some(profile_id.into()),
                profile_name: Some(profile_name.into()),
                message: Some(line.to_string()),
                started_at: None,
            });
        }
    }
    if let Some(new) = transition {
        {
            let mut s = state.lock().unwrap();
            *s = new.clone();
        }
        let _ = app.emit("vpn://state", &new);
    }
}
